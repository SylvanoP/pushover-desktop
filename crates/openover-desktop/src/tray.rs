use crate::session::{SessionCommand, SessionCommands};
use crate::settings;
use gpui::{App, Global, MenuItem, actions};
use gpui_tray::{Icon, Tray};

actions!(openover_tray, [OpenSettings, Logout, Quit]);

struct AppTray(Tray);

impl Global for AppTray {}

pub fn setup(cx: &mut App) -> gpui_tray::Result<()> {
    cx.on_action(open_settings)
        .on_action(logout)
        .on_action(quit);

    let tray = Tray::builder()
        .icon(tray_icon()?)
        .title("Openover")
        .tooltip("Openover — unofficial Pushover Open Client")
        .menu(build_menu)
        .build(cx)?;
    cx.set_global(AppTray(tray));
    Ok(())
}

fn build_menu(_cx: &mut App) -> Vec<MenuItem> {
    vec![
        MenuItem::action("Einstellungen…", OpenSettings),
        MenuItem::action("Abmelden", Logout),
        MenuItem::separator(),
        MenuItem::action("Beenden", Quit),
    ]
}

fn open_settings(_: &OpenSettings, cx: &mut App) {
    settings::open_settings_window(cx);
}

fn logout(_: &Logout, cx: &mut App) {
    if let Some(commands) = cx.try_global::<SessionCommands>() {
        let _ = commands.0.send(SessionCommand::Logout);
    }
}

pub fn set_tooltip_text(cx: &mut App, text: &str) {
    let Some(tray) = cx.try_global::<AppTray>().map(|tray| tray.0.clone()) else {
        return;
    };
    let _ = tray.set_tooltip(Some(text.to_string()), cx);
}

fn quit(_: &Quit, cx: &mut App) {
    let tray = cx.global::<AppTray>().0.clone();
    let _ = tray.close(cx);
    cx.quit();
}

/// Simple neutral tray icon (no Pushover logo).
fn tray_icon() -> gpui_tray::Result<Icon> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let dx = x as i32 - 16;
            let dy = y as i32 - 16;
            let dist = ((dx * dx + dy * dy) as f32).sqrt();
            if dist < 12.0 {
                rgba[i] = 90;
                rgba[i + 1] = 140;
                rgba[i + 2] = 200;
                rgba[i + 3] = 255;
            } else if dist < 14.0 {
                rgba[i] = 40;
                rgba[i + 1] = 60;
                rgba[i + 2] = 90;
                rgba[i + 3] = 180;
            }
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).build()
}
