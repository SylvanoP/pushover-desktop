//! Show a toast without holding GPUI's `App` borrow.
//!
//! `ToastNotificationManager::Show` pumps the Windows message queue. Doing that
//! from inside `cx.update` makes GPUI's resize and inset callbacks fail their
//! re-entrant borrow and log `RefCell already borrowed` as an error.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use gpui::{App, SystemNotification};

use crate::session::SessionCommands;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationAction {
    Acknowledge(String),
    Open(u64),
}

/// Body click uses `launch` (the message id). An acknowledge button stays on
/// the message and does not open it.
pub fn notification_action(
    action_id: Option<&str>,
    launch: Option<&str>,
) -> Option<NotificationAction> {
    if let Some(action) = action_id {
        if let Some(receipt) = action.strip_prefix("ack:")
            && !receipt.is_empty()
        {
            return Some(NotificationAction::Acknowledge(receipt.to_string()));
        }
        return None;
    }
    let id = launch?.parse().ok()?;
    Some(NotificationAction::Open(id))
}

fn launches() -> &'static Mutex<HashMap<String, String>> {
    static LAUNCHES: LazyLock<Mutex<HashMap<String, String>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    &LAUNCHES
}

pub fn remember_launch(tag: &str, launch: &str) {
    if tag.is_empty() || launch.is_empty() {
        return;
    }
    launches()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .insert(tag.to_string(), launch.to_string());
}

pub fn launch_for(tag: &str) -> Option<String> {
    launches()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .get(tag)
        .cloned()
}

pub fn present(cx: &mut App, notification: SystemNotification, launch: Option<String>) {
    if let Some(launch) = launch.as_deref().filter(|launch| !launch.is_empty()) {
        remember_launch(notification.tag.as_ref(), launch);
    }
    let commands = cx
        .try_global::<SessionCommands>()
        .map(|commands| commands.0.clone());
    #[cfg(windows)]
    {
        cx.foreground_executor()
            .spawn(async move {
                windows_toast::show(notification, launch, commands);
            })
            .detach();
    }
    #[cfg(not(windows))]
    {
        let _ = (commands, launch);
        let async_cx = cx.to_async();
        cx.foreground_executor()
            .spawn(async move {
                async_cx.update(|cx| cx.show_system_notification(notification));
            })
            .detach();
    }
}

#[cfg(windows)]
mod windows_toast {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use gpui::{SharedString, SystemNotification};
    use tokio::sync::mpsc;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{
        ToastActivatedEventArgs, ToastNotification, ToastNotificationManager,
    };
    use windows::core::{IInspectable, Interface as _, h};

    use crate::session::SessionCommand;

    const APP_ID: &str = "net.openover.desktop";
    const APP_NAME: &str = "Openover";

    struct Shown {
        notifier: windows::UI::Notifications::ToastNotifier,
        active: HashMap<SharedString, ToastNotification>,
    }

    thread_local! {
        static SHOWN: RefCell<Option<Shown>> = const { RefCell::new(None) };
    }

    pub fn show(
        notification: SystemNotification,
        launch: Option<String>,
        commands: Option<mpsc::UnboundedSender<SessionCommand>>,
    ) {
        let Some(notifier) = notifier() else {
            return;
        };
        let Ok(document) = toast_document(&notification, launch.as_deref()) else {
            tracing::warn!("could not build toast");
            return;
        };
        let Ok(toast) = ToastNotification::CreateToastNotification(&document) else {
            tracing::warn!("could not create toast");
            return;
        };
        let tag = notification.tag.clone();
        let _ = toast.SetTag(&tag.as_ref().into());
        if let Some(commands) = commands {
            let activated = TypedEventHandler::<ToastNotification, IInspectable>::new(
                move |_sender, arguments| {
                    let action_id = arguments
                        .as_ref()
                        .and_then(|arguments| arguments.cast::<ToastActivatedEventArgs>().ok())
                        .and_then(|arguments| arguments.Arguments().ok())
                        .filter(|arguments| !arguments.is_empty())
                        .map(|arguments| arguments.to_string());
                    match action_id.as_deref().map(str::trim) {
                        Some(action) if let Some(receipt) = action.strip_prefix("ack:") => {
                            if !receipt.is_empty() {
                                let _ = commands.send(SessionCommand::Acknowledge {
                                    receipt: receipt.to_string(),
                                });
                            }
                        }
                        Some(action) if let Ok(id) = action.parse::<u64>() => {
                            let _ = commands.send(SessionCommand::OpenMessage { id });
                        }
                        _ => {}
                    }
                    Ok(())
                },
            );
            if toast.Activated(&activated).is_err() {
                tracing::warn!("could not attach toast action");
            }
        }
        let previous = SHOWN.with(|slot| {
            slot.borrow_mut()
                .as_mut()
                .and_then(|shown| shown.active.remove(&tag))
        });
        if let Some(previous) = previous.as_ref() {
            let _ = notifier.Hide(previous);
        }
        if notifier.Show(&toast).is_err() {
            tracing::warn!("could not show toast");
            return;
        }
        SHOWN.with(|slot| {
            if let Some(shown) = slot.borrow_mut().as_mut() {
                shown.active.insert(tag, toast);
            }
        });
    }

    fn notifier() -> Option<windows::UI::Notifications::ToastNotifier> {
        SHOWN.with(|slot| {
            let existing = slot.borrow().as_ref().map(|shown| shown.notifier.clone());
            if existing.is_some() {
                return existing;
            }
            register_identity();
            let created =
                ToastNotificationManager::CreateToastNotifierWithId(&APP_ID.into()).ok()?;
            *slot.borrow_mut() = Some(Shown {
                notifier: created.clone(),
                active: HashMap::new(),
            });
            Some(created)
        })
    }

    fn register_identity() {
        let result = windows_registry::CURRENT_USER
            .create(format!(r"Software\Classes\AppUserModelId\{APP_ID}"))
            .and_then(|key| key.set_string("DisplayName", APP_NAME));
        if let Err(error) = result {
            tracing::warn!("could not register toast identity: {error}");
        }
    }

    fn toast_document(
        notification: &SystemNotification,
        launch: Option<&str>,
    ) -> windows::core::Result<XmlDocument> {
        let document = XmlDocument::new()?;
        let toast = document.CreateElement(h!("toast"))?;
        if let Some(launch) = launch.filter(|launch| !launch.is_empty()) {
            toast.SetAttribute(h!("launch"), &launch.into())?;
        }
        document.AppendChild(&toast)?;
        let visual = document.CreateElement(h!("visual"))?;
        toast.AppendChild(&visual)?;
        let binding = document.CreateElement(h!("binding"))?;
        binding.SetAttribute(h!("template"), h!("ToastGeneric"))?;
        visual.AppendChild(&binding)?;
        for text in [&notification.title, &notification.body] {
            let element = document.CreateElement(h!("text"))?;
            element.SetInnerText(&text.as_ref().into())?;
            binding.AppendChild(&element)?;
        }
        if !notification.actions.is_empty() {
            let actions = document.CreateElement(h!("actions"))?;
            toast.AppendChild(&actions)?;
            for action in &notification.actions {
                let element = document.CreateElement(h!("action"))?;
                element.SetAttribute(h!("content"), &action.label.as_ref().into())?;
                element.SetAttribute(h!("arguments"), &action.id.as_ref().into())?;
                actions.AppendChild(&element)?;
            }
        }
        let audio = document.CreateElement(h!("audio"))?;
        audio.SetAttribute(h!("silent"), h!("true"))?;
        toast.AppendChild(&audio)?;
        Ok(document)
    }
}

#[cfg(test)]
mod tests {
    use super::{NotificationAction, notification_action};

    #[test]
    fn body_click_opens_the_launched_message() {
        assert_eq!(
            notification_action(None, Some("42")),
            Some(NotificationAction::Open(42))
        );
    }

    #[test]
    fn acknowledge_stays_on_the_message() {
        assert_eq!(
            notification_action(Some("ack:receipt-1"), Some("42")),
            Some(NotificationAction::Acknowledge("receipt-1".into()))
        );
        assert_eq!(notification_action(Some("ack:"), Some("42")), None);
    }

    #[test]
    fn a_click_without_a_launch_does_nothing() {
        assert_eq!(notification_action(None, None), None);
        assert_eq!(notification_action(None, Some("")), None);
        assert_eq!(notification_action(None, Some("demo")), None);
    }
}
