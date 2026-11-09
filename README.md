# Openover Desktop

**Deutsch (primär)** — Inoffizieller **Pushover Open Client** für Windows, macOS und Linux. Tray-first: die App startet im System-Tray; ein minimales Fenster dient später den Einstellungen.

> **Disclaimer:** Dieses Projekt ist **nicht** von Pushover LLC veröffentlicht oder unterstützt. Pushover® ist eine Marke von Pushover LLC. Bitte beachtet die [Pushover Distribution Guidelines](https://pushover.net/api/client/#distribution-guidelines) (kein „Pushover for …“ im Namen, kein Pushover-Logo als App-Icon).

**Lizenz für den Dienst:** Für Open-Client-Geräte ist eine [Pushover for Desktop](https://pushover.net/clients/desktop)-Lizenz erforderlich (30-Tage-Test, danach einmalig ~5 USD). Kauf und Account nur über [pushover.net](https://pushover.net/).

**English (short):** Unofficial Pushover **Open Client** desktop app (`os=O`). Not affiliated with Pushover LLC. Desktop license required per Pushover policy. MIT-licensed source; build from source today; GitHub Releases planned later.

## Status (v0.1.0 scaffold)

- GPUI (`gpui-ce`) + `gpui-tray` shell
- Rust crate `openover-core`: Login, Geräteregistrierung, Nachrichten-Typen, WebSocket-Skelett mit Reconnect
- Credential-Speicher via OS-Keyring (`keyring`); Dev/CI: `OPENOVER_SECRET`, `OPENOVER_DEVICE_ID`, optional `OPENOVER_USER_KEY`
- Kein vollständiges Onboarding, keine Historie, keine Installer-Releases in diesem Stand

## Build (from source)

Voraussetzungen: **Rust stable** (siehe `rust-toolchain.toml`), plattformspezifische GPUI-Abhängigkeiten (Linux: u. a. X11/Wayland, `libxkbcommon`, Audio/Font-Pakete je nach Distro).

```bash
cargo build --release
cargo run -- --demo-toast
```

`--demo-toast` zeigt eine **Demo-Benachrichtigung** über die GPUI-Systemnotification-API (ohne Pushover-Account).

Mit gespeicherten Credentials startet ein Hintergrund-WebSocket zu `wss://client.pushover.net/push` (Logging; vollständige Sync-UI folgt).

## Entwicklung / CI

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --all
```

## Sicherheit

Passwörter gehören **nicht** ins Repository. Session-`secret` und `device_id` nur im OS-Keyring oder in lokalen Umgebungsvariablen für Tests.

## Contributing

Issues und PRs willkommen. Bitte inoffiziellen Charakter und Pushover-Richtlinien respektieren.
