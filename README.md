# Openover Desktop

**Deutsch (primär)** — Inoffizieller **Pushover Open Client**. Die veröffentlichte Datei gibt es nur für Windows. Tray-first: die App startet im System-Tray; das Fenster ist die Anmeldung und die Einstellungen.

> **Disclaimer:** Dieses Projekt ist **nicht** von Pushover LLC veröffentlicht oder unterstützt. Pushover® ist eine Marke von Pushover LLC. Bitte beachtet die [Pushover Distribution Guidelines](https://pushover.net/api/client/#distribution-guidelines) (kein „Pushover for …“ im Namen, kein Pushover-Logo als App-Icon).

**Lizenz für den Dienst:** Für Open-Client-Geräte ist eine [Pushover-Desktop-Lizenz](https://pushover.net/clients/desktop) erforderlich (30-Tage-Test, danach einmalig ~5 USD). Kauf und Account nur über [pushover.net](https://pushover.net/).

**Download (nur Windows x64):** [openover-desktop-windows-x64.exe](https://github.com/SylvanoP/pushover-desktop/releases/download/v0.1.2/openover-desktop-windows-x64.exe) aus dem Release [v0.1.2](https://github.com/SylvanoP/pushover-desktop/releases/tag/v0.1.2).

**English (short):** Unofficial Pushover **Open Client** (`os=O`). Not affiliated with Pushover LLC. The app does not use the Pushover logo and is not named “Pushover for …”. The only published file is the Windows x64 build [openover-desktop-windows-x64.exe](https://github.com/SylvanoP/pushover-desktop/releases/download/v0.1.2/openover-desktop-windows-x64.exe) on the [v0.1.2 release](https://github.com/SylvanoP/pushover-desktop/releases/tag/v0.1.2). A Pushover desktop license is required. The source is MIT-licensed.

## Status

- GPUI (`gpui-ce`) + `gpui-tray`
- Anmeldung im Einstellungsfenster (E-Mail, Passwort, optional 2FA, Gerätename)
- `openover-core`: Login, Gerät `os=O`, Nachrichten, WebSocket mit Reconnect
- Nach `!` werden neue Nachrichten als Systembenachrichtigung gezeigt (Titel, Text, App-Name)
- Pushover-Sounds (`sounds/{code}.mp3`) werden lokal gecacht und unter Windows abgespielt. Priorität -1 bleibt stumm. Die Windows-Toasts selbst haben keinen Ton
- Beim ersten Start wird eine große alte Warteschlange verworfen und einmal angezeigt. Die letzte Nachrichten-ID wird erst nach erfolgreichem Löschen gespeichert
- Notfälle bleiben lokal und werden etwa alle 30 Sekunden erneut gezeigt, bis sie bestätigt sind
- Tray: Einstellungen, Abmelden, Beenden. Der Tooltip zeigt den Verbindungszustand. Ein zweiter Start öffnet die laufende Instanz
- Zugangsdaten im OS-Keyring (`user key`, `secret`, `device id`). Das Passwort wird nicht gespeichert. Abmelden löscht nur das Sitzungsgeheimnis
- Keine Historie, keine Bildanhänge

Ohne gespeicherte Anmeldung öffnet sich das Fenster von selbst. Mit Anmeldung bleibt die App im Tray; Einstellungen liegen im Tray-Menü.

## Build (from source)

Voraussetzungen: **Rust stable** (siehe `rust-toolchain.toml`), plattformspezifische GPUI-Abhängigkeiten (Linux: u. a. X11/Wayland, `libxkbcommon`, Audio/Font-Pakete je nach Distro).

```bash
cargo build --release
cargo run -- --demo-toast
```

`--demo-toast` zeigt eine **Demo-Benachrichtigung** über die GPUI-Systemnotification-API (ohne Pushover-Account).

Mit gespeicherten Zugangsdaten verbindet sich die App mit `wss://client.pushover.net/push`. Bei `!` holt sie Nachrichten per HTTPS und zeigt sie als Systembenachrichtigung.

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
