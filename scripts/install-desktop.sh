#!/usr/bin/env bash
# Build and install StepX Companion for the current user on Linux or macOS.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
cd "$ROOT/desktop"
if [ "${STEPX_SKIP_BUILD:-0}" != "1" ]; then
    if [ "${STEPX_PROFILE:-debug}" = "release" ]; then
        cargo build --release --locked
    else
        cargo build --locked
    fi
fi
if [ "${STEPX_PROFILE:-}" = "release" ]; then
    STEPX_BIN_SRC="target/release/stepx-companion"
elif [ "${STEPX_PROFILE:-}" = "debug" ]; then
    STEPX_BIN_SRC="target/debug/stepx-companion"
elif [ "${STEPX_SKIP_BUILD:-0}" = "1" ] && [ -f target/release/stepx-companion ] && { [ ! -f target/debug/stepx-companion ] || [ target/release/stepx-companion -nt target/debug/stepx-companion ]; }; then
    STEPX_BIN_SRC="target/release/stepx-companion"
else
    STEPX_BIN_SRC="target/debug/stepx-companion"
fi

OS="$(uname -s)"
if [ "$OS" = "Linux" ] && [ "$(uname -m)" = "x86_64" ] && [ ! -x "$ROOT/.tools/bin/adb" ]; then
    "$ROOT/scripts/setup-desktop-tools.sh"
fi
if [ "$OS" = "Darwin" ]; then
    APP_DIR="${STEPX_MAC_APP_DIR:-$HOME/Applications/StepX Companion.app}"
    mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources/tools"
    install -m755 "$STEPX_BIN_SRC" "$APP_DIR/Contents/MacOS/stepx-companion.new"
    mv "$APP_DIR/Contents/MacOS/stepx-companion.new" "$APP_DIR/Contents/MacOS/stepx-companion"
    if [ -d "$ROOT/.tools/bin" ]; then
        cp -a "$ROOT/.tools/bin" "$APP_DIR/Contents/Resources/tools/"
        [ -d "$ROOT/.tools/root" ] && cp -a "$ROOT/.tools/root" "$APP_DIR/Contents/Resources/tools/" || true
    fi
    mkdir -p "$APP_DIR/Contents/Resources/desktop"
    cp -a "$ROOT/scripts" "$ROOT/handoff" "$APP_DIR/Contents/Resources/"
    cp -a "$ROOT/desktop/assets" "$APP_DIR/Contents/Resources/desktop/"
    python3 "$ROOT/scripts/package-runtime.py" "$APP_DIR/Contents/Resources"
    STEPX_ICON_WORK="$(mktemp -d)"
    mkdir -p "$STEPX_ICON_WORK/StepXCompanion.iconset"
    for size in 16 32 128 256 512; do
        sips -z "$size" "$size" assets/stepx_companion_icon.png --out "$STEPX_ICON_WORK/StepXCompanion.iconset/icon_${size}x${size}.png" >/dev/null
        double=$((size * 2))
        sips -z "$double" "$double" assets/stepx_companion_icon.png --out "$STEPX_ICON_WORK/StepXCompanion.iconset/icon_${size}x${size}@2x.png" >/dev/null
    done
    iconutil -c icns "$STEPX_ICON_WORK/StepXCompanion.iconset" -o "$APP_DIR/Contents/Resources/StepXCompanion.icns"
    rm -r "$STEPX_ICON_WORK"
    STEPX_APP_VERSION="$(cargo pkgid --offline -p stepx-companion)"
    STEPX_APP_VERSION="${STEPX_APP_VERSION##*@}"
    cat > "$APP_DIR/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDisplayName</key>
    <string>StepX Companion</string>
    <key>CFBundleName</key>
    <string>StepX Companion</string>
    <key>CFBundleIconFile</key>
    <string>StepXCompanion.icns</string>
    <key>CFBundleIdentifier</key>
    <string>com.stepx.companion</string>
    <key>CFBundleVersion</key>
    <string>$STEPX_APP_VERSION</string>
    <key>CFBundleShortVersionString</key>
    <string>$STEPX_APP_VERSION</string>
    <key>CFBundleExecutable</key>
    <string>stepx-companion</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST
    echo "已安装 macOS 应用：$APP_DIR"
    exit 0
fi

STEPX_INSTALL_DIR="${STEPX_INSTALL_DIR:-$HOME/.local/opt/stepx-companion}"
STEPX_DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
mkdir -p "$STEPX_INSTALL_DIR/tools" "$STEPX_INSTALL_DIR/desktop" "$STEPX_DATA_DIR/applications" "$STEPX_DATA_DIR/icons"
install -m755 "$STEPX_BIN_SRC" "$STEPX_INSTALL_DIR/stepx-companion.new"
mv "$STEPX_INSTALL_DIR/stepx-companion.new" "$STEPX_INSTALL_DIR/stepx-companion"
cp -af "$ROOT/scripts" "$ROOT/handoff" "$STEPX_INSTALL_DIR/"
cp -af "$ROOT/desktop/assets" "$STEPX_INSTALL_DIR/desktop/"
if [ -d "$ROOT/.tools/bin" ] && [ ! -x "$STEPX_INSTALL_DIR/tools/bin/adb" ]; then
    cp -a "$ROOT/.tools/bin" "$ROOT/.tools/root" "$STEPX_INSTALL_DIR/tools/"
fi
python3 - "$STEPX_INSTALL_DIR/stepx-companion" "$STEPX_DATA_DIR" <<'PY'
import hashlib, pathlib, shutil, sys
binary = sys.argv[1].replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%')
data_dir = pathlib.Path(sys.argv[2]).resolve()
entry = data_dir/'applications/stepx-companion.desktop'
source = pathlib.Path('assets/stepx_companion_icon.png')
digest = hashlib.sha256(source.read_bytes()).hexdigest()[:12]
icon_path = data_dir/'icons'/f'stepx-companion-{digest}.png'
shutil.copy2(source, icon_path)
icon = str(icon_path).replace('\\', '\\\\')
entry.write_text(f'''[Desktop Entry]
Type=Application
Name=StepX Companion
Name[en]=StepX Companion
Comment=连接 StepX，协同手机与电脑
Exec="{binary}"
Icon={icon}
Terminal=false
Categories=Utility;
StartupWMClass=stepx-companion
''')
PY
if command -v update-desktop-database >/dev/null; then update-desktop-database "$STEPX_DATA_DIR/applications"; fi
if [ "${STEPX_RESTART_RUNNING:-0}" = "1" ]; then
    killall -q stepx-companion 2>/dev/null || true
    sleep 0.2
    nohup "$STEPX_INSTALL_DIR/stepx-companion" </dev/null >/dev/null 2>&1 &
    disown 2>/dev/null || true
fi
echo "已安装：$STEPX_INSTALL_DIR/stepx-companion"
