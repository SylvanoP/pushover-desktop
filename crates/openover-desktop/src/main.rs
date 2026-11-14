#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod form_theme;
mod icon;
mod instance;
mod notify;
mod password_field;
#[cfg(windows)]
mod power;
mod session;
mod settings;
mod sound;
mod startup;
mod tray;
mod update;

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
    /// Start in the tray. A saved session stays hidden until the tray opens it.
    #[arg(long)]
    background: bool,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("openover_desktop=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();
    if !instance::claim_primary() {
        if let Some(code) = cli.play_sound.clone() {
            play_sound_code(&code, true);
        } else {
            eprintln!("Openover läuft bereits.");
        }
        instance::signal_running_instance();
        return;
    }
    #[cfg(windows)]
    discard_replaced_exe();

    application()
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            cx.set_app_identity("net.openover.desktop", "Openover");

            if let Err(err) = tray::setup(cx) {
                eprintln!("tray setup failed: {err}");
                cx.quit();
                return;
            }

            settings::install(cx, cli.background);

            if cli.demo_toast {
                show_demo_notification(cx);
            }
            if let Some(code) = cli.play_sound.clone() {
                play_sound_code(&code, false);
            }
        });
}

#[cfg(windows)]
fn discard_replaced_exe() {
    let Ok(current) = std::env::current_exe() else {
        return;
    };
    let _ = std::fs::remove_file(current.with_extension("exe.old"));
}

fn play_sound_code(code: &str, wait: bool) {
    let code = code.to_string();
    let volume = openover_core::clamp_volume(openover_core::LocalState::load().sound_volume);
    let run = move || {
        if volume == 0 {
            tracing::info!("sound {code} stays silent because volume is 0");
            return;
        }
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
                    tracing::info!("playing sound {code} at volume {volume}");
                    if wait {
                        sound::play_and_wait(path, volume);
                    } else {
                        sound::play(path, volume);
                    }
                }
                Err(err) => tracing::warn!("could not play sound {code}: {err}"),
            }
        });
    };
    if wait {
        run();
        return;
    }
    std::thread::Builder::new()
        .name("openover-sound-fetch".into())
        .spawn(run)
        .ok();
}

fn show_demo_notification(cx: &mut App) {
    notify::present(
        cx,
        SystemNotification {
            tag: DEMO_NOTIFICATION_TAG.into(),
            title: "Openover (Demo)".into(),
            body: "Inoffizieller Pushover Open Client".into(),
            actions: vec![],
        },
        None,
    );
}
