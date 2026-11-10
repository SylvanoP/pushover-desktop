use crate::session::{SessionCommand, SessionCommands};
use crate::settings;
use gpui::{App, Global, MenuItem, actions};
use gpui_tray::{Icon, Tray};

actions!(openover_tray, [OpenSettings, OpenPreferences, Logout, Quit]);

struct AppTray(Tray);

impl Global for AppTray {}

pub fn setup(cx: &mut App) -> gpui_tray::Result<()> {
    cx.on_action(open_settings)
        .on_action(open_preferences)
        .on_action(logout)
        .on_action(quit);

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

fn build_menu(_cx: &mut App) -> Vec<MenuItem> {
    vec![
        MenuItem::action("Öffnen", OpenSettings),
        MenuItem::action("Einstellungen…", OpenPreferences),
        MenuItem::action("Abmelden", Logout),
        MenuItem::separator(),
        MenuItem::action("Beenden", Quit),
    ]
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
