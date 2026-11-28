use crate::session::{SessionCommand, SessionCommands};
use crate::settings;
use gpui::{App, Global, Menu, MenuItem, actions};
use gpui_tray::{Icon, Tray};
use openover_core::QuietReason;

actions!(
    openover_tray,
    [
        OpenSettings,
        OpenPreferences,
        Logout,
        Quit,
        Pause30,
        Pause60,
        Pause480,
        Resume,
        ToggleQuietWindow,
        OpenLogs
    ]
);

struct AppTray(Tray);

impl Global for AppTray {}

pub fn setup(cx: &mut App) -> gpui_tray::Result<()> {
    cx.on_action(open_settings)
        .on_action(open_preferences)
        .on_action(logout)
        .on_action(quit)
        .on_action(|_: &Pause30, cx| settings::pause_for(30, cx))
        .on_action(|_: &Pause60, cx| settings::pause_for(60, cx))
        .on_action(|_: &Pause480, cx| settings::pause_for(8 * 60, cx))
        .on_action(|_: &Resume, cx| settings::resume_now(cx))
        .on_action(|_: &ToggleQuietWindow, cx| settings::toggle_quiet_window(cx))
        .on_action(|_: &OpenLogs, cx| crate::logging::reveal(cx));

    let tray = Tray::builder()
        .icon(tray_icon()?)
        .title("Openover")
        .tooltip("Openover — unofficial Pushover Open Client")
        .on_activate_with(|token, cx| {
            settings::open_settings_window_with_token(token.as_deref(), cx);
        })
        .menu(build_menu)
        .build(cx)?;
    cx.set_global(AppTray(tray));
    Ok(())
}

fn build_menu(cx: &mut App) -> Vec<MenuItem> {
    vec![
        MenuItem::action("Öffnen", OpenSettings),
        MenuItem::action("Einstellungen…", OpenPreferences),
        MenuItem::submenu(quiet_menu(cx)),
        MenuItem::separator(),
        MenuItem::action("Protokoll öffnen", OpenLogs),
        MenuItem::action("Abmelden", Logout),
        MenuItem::separator(),
        MenuItem::action("Beenden", Quit),
    ]
}

/// "Nicht stören", titled with the time it ends while it holds, so the state
/// reads from the menu without opening the window.
fn quiet_menu(cx: &mut App) -> Menu {
    let (reason, window_enabled) = settings::quiet_state(cx);
    let title = match reason {
        Some(QuietReason::Paused(until)) => {
            format!("Stumm bis {}", crate::clock::format_local_clock(until))
        }
        Some(QuietReason::Window { until_minute }) => {
            format!(
                "Ruhezeit bis {}",
                openover_core::format_minute(until_minute)
            )
        }
        None => "Nicht stören".to_string(),
    };
    let paused = matches!(reason, Some(QuietReason::Paused(_)));
    let mut items = vec![
        MenuItem::action("30 Minuten", Pause30),
        MenuItem::action("1 Stunde", Pause60),
        MenuItem::action("8 Stunden", Pause480),
    ];
    if paused {
        items.push(MenuItem::separator());
        items.push(MenuItem::action("Jetzt fortsetzen", Resume));
    }
    items.push(MenuItem::separator());
    items.push(MenuItem::action("Tägliche Ruhezeit", ToggleQuietWindow).checked(window_enabled));
    Menu::new(title).items(items)
}

/// Rebuilds the tray menu, so its titles follow the current state.
pub fn refresh_menu(cx: &mut App) {
    let async_cx = cx.to_async();
    cx.foreground_executor()
        .spawn(async move {
            async_cx.update(|cx| {
                let Some(tray) = cx.try_global::<AppTray>().map(|tray| tray.0.clone()) else {
                    return;
                };
                if let Err(err) = tray.refresh_menu(cx) {
                    tracing::warn!("could not refresh the tray menu: {err}");
                }
            });
        })
        .detach();
}

fn open_settings(_: &OpenSettings, cx: &mut App) {
    settings::open_settings_window(cx);
}

fn open_preferences(_: &OpenPreferences, cx: &mut App) {
    settings::open_preferences(cx);
}

fn logout(_: &Logout, cx: &mut App) {
    if let Some(commands) = cx.try_global::<SessionCommands>() {
        let _ = commands.0.send(SessionCommand::Logout);
    }
}

pub fn set_unread_dot(cx: &mut App, unread: bool) {
    #[cfg(any(windows, target_os = "macos"))]
    {
        let async_cx = cx.to_async();
        cx.foreground_executor()
            .spawn(async move {
                async_cx.update(|cx| apply_unread_icon(cx, unread));
            })
            .detach();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (cx, unread);
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn apply_unread_icon(cx: &mut App, unread: bool) {
    let Some(tray) = cx.try_global::<AppTray>().map(|tray| tray.0.clone()) else {
        return;
    };
    let image = crate::icon::tray_image_with_dot(unread);
    let (width, height) = image.dimensions();
    let Ok(icon) = Icon::from_rgba(image.into_raw(), width, height).build() else {
        return;
    };
    let _ = tray.set_icon(icon, cx);
}

pub fn set_tooltip_text(cx: &mut App, text: &str) {
    let text = text.to_string();
    let async_cx = cx.to_async();
    cx.foreground_executor()
        .spawn(async move {
            async_cx.update(|cx| {
                let Some(tray) = cx.try_global::<AppTray>().map(|tray| tray.0.clone()) else {
                    return;
                };
                let _ = tray.set_tooltip(Some(text), cx);
            });
        })
        .detach();
}

fn quit(_: &Quit, cx: &mut App) {
    exit(cx);
}

pub fn exit(cx: &mut App) {
    if let Some(tray) = cx.try_global::<AppTray>().map(|tray| tray.0.clone()) {
        let _ = tray.close(cx);
    }
    cx.quit();
}

fn tray_icon() -> gpui_tray::Result<Icon> {
    let image = crate::icon::tray_image();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).build()
}
