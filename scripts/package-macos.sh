#!/bin/sh
# Package the release binary as Openover.app. Does not notarize.
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
bin="$root/target/release/openover-desktop"
icon_src="$root/crates/openover-desktop/assets/openover-64.png"
dest="${1:-$HOME/Applications/Openover.app}"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n 1)

if [ ! -x "$bin" ]; then
    echo "missing release binary: $bin" >&2
    exit 1
fi

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
iconset="$stage/AppIcon.iconset"
mkdir -p "$iconset"

sips -z 16 16 "$icon_src" --out "$iconset/icon_16x16.png" >/dev/null
sips -z 32 32 "$icon_src" --out "$iconset/icon_16x16@2x.png" >/dev/null
sips -z 32 32 "$icon_src" --out "$iconset/icon_32x32.png" >/dev/null
sips -z 64 64 "$icon_src" --out "$iconset/icon_32x32@2x.png" >/dev/null
sips -z 128 128 "$icon_src" --out "$iconset/icon_128x128.png" >/dev/null
sips -z 256 256 "$icon_src" --out "$iconset/icon_128x128@2x.png" >/dev/null
sips -z 256 256 "$icon_src" --out "$iconset/icon_256x256.png" >/dev/null
sips -z 512 512 "$icon_src" --out "$iconset/icon_256x256@2x.png" >/dev/null
sips -z 512 512 "$icon_src" --out "$iconset/icon_512x512.png" >/dev/null
sips -z 1024 1024 "$icon_src" --out "$iconset/icon_512x512@2x.png" >/dev/null
iconutil -c icns "$iconset" -o "$stage/AppIcon.icns"

mkdir -p "$stage/Openover.app/Contents/MacOS" "$stage/Openover.app/Contents/Resources"
cp "$bin" "$stage/Openover.app/Contents/MacOS/openover-desktop"
chmod 755 "$stage/Openover.app/Contents/MacOS/openover-desktop"
cp "$stage/AppIcon.icns" "$stage/Openover.app/Contents/Resources/AppIcon.icns"

cat >"$stage/Openover.app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Openover</string>
    <key>CFBundleDisplayName</key>
    <string>Openover</string>
    <key>CFBundleIdentifier</key>
    <string>net.openover.desktop</string>
    <key>CFBundleExecutable</key>
    <string>openover-desktop</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${version}</string>
    <key>CFBundleVersion</key>
    <string>${version}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

# Ad-hoc signature so Finder can open the bundle. This is not notarization.
codesign --force --sign - "$stage/Openover.app"

mkdir -p "$(dirname "$dest")"
rm -rf "$dest"
mv "$stage/Openover.app" "$dest"
echo "$dest"
