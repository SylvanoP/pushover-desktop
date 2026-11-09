mod password_field;
mod session;
mod settings;
mod tray;

extern crate gpui_ce_backend as gpui;

use clap::Parser;
use gpui::{App, QuitMode, SystemNotification};
use gpui_platform::application;
use tracing_subscriber::EnvFilter;

const DEMO_NOTIFICATION_TAG: &str = "openover-demo";

#[derive(Debug, Parser)]
#[command(
    name = "openover-desktop",
    version,
    about = "Unofficial Pushover Open Client (tray)"
)]
struct Cli {
    /// Show a demo system notification at startup (no Pushover account required).
    #[arg(long)]
    demo_toast: bool,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("openover_desktop=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    application()
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            cx.set_app_identity("net.openover.desktop", "Openover");

            if let Err(err) = tray::setup(cx) {
                eprintln!("tray setup failed: {err}");
                cx.quit();
                return;
            }

            settings::install(cx);

            if cli.demo_toast {
                show_demo_notification(cx);
            }
        });
}

fn show_demo_notification(cx: &mut App) {
    cx.show_system_notification(SystemNotification {
        tag: DEMO_NOTIFICATION_TAG.into(),
        title: "Openover (Demo)".into(),
        body: "Inoffizieller Pushover Open Client — Scaffold v0.1.0".into(),
        actions: vec![],
    });
}
