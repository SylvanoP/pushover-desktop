mod form_theme;
mod instance;
mod password_field;
mod session;
mod settings;
mod sound;
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
    /// Download and play one Pushover sound, for example `bike`.
    #[arg(long, value_name = "CODE")]
    play_sound: Option<String>,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("openover_desktop=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();
    if !instance::claim_primary() {
        instance::signal_running_instance();
        eprintln!("Openover läuft bereits.");
        return;
    }

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
            if let Some(code) = cli.play_sound.clone() {
                play_sound_code(code);
            }
        });
}

fn play_sound_code(code: String) {
    std::thread::Builder::new()
        .name("openover-sound-fetch".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            let Ok(runtime) = runtime else {
                return;
            };
            runtime.block_on(async move {
                let Ok(client) = openover_core::PushoverClient::new() else {
                    return;
                };
                match client.cache_sound(&code).await {
                    Ok(path) => {
                        tracing::info!("playing sound {code}");
                        sound::play(path);
                    }
                    Err(err) => tracing::warn!("could not play sound {code}: {err}"),
                }
            });
        })
        .ok();
}

fn show_demo_notification(cx: &mut App) {
    cx.show_system_notification(SystemNotification {
        tag: DEMO_NOTIFICATION_TAG.into(),
        title: "Openover (Demo)".into(),
        body: "Inoffizieller Pushover Open Client — Scaffold v0.1.0".into(),
        actions: vec![],
    });
}
