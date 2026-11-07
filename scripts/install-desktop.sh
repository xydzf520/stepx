#!/usr/bin/env bash
# Build and install StepX Companion for the current user on Linux or macOS.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
if [ -z "${BINDGEN_EXTRA_CLANG_ARGS:-}" ] && command -v gcc >/dev/null 2>&1; then
    GCC_INC="$(gcc -print-file-name=include 2>/dev/null || true)"
    if [ -n "$GCC_INC" ] && [ -d "$GCC_INC" ]; then
        export BINDGEN_EXTRA_CLANG_ARGS="-isystem $GCC_INC"
    fi
fi
cd "$ROOT/desktop"
if [ -x "$ROOT/scripts/prune-rust-target.sh" ]; then
    "$ROOT/scripts/prune-rust-target.sh" || true
fi
if [ "${STEPX_SKIP_BUILD:-0}" != "1" ]; then
    python3 "$ROOT/scripts/apply-ui-window-compat.py"
    if [ "${STEPX_PROFILE:-release}" = "release" ]; then
        cargo build --release --locked
    else
        cargo build --locked
    fi
fi
if [ "${STEPX_PROFILE:-release}" = "release" ] && [ -f target/release/stepx-companion ]; then
    STEPX_BIN_SRC="target/release/stepx-companion"
elif [ "${STEPX_PROFILE:-}" = "debug" ] && [ -f target/debug/stepx-companion ]; then
    STEPX_BIN_SRC="target/debug/stepx-companion"
elif [ -f target/release/stepx-companion ] && { [ ! -f target/debug/stepx-companion ] || [ target/release/stepx-companion -nt target/debug/stepx-companion ]; }; then
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
    python3 "$ROOT/scripts/stage-release-resources.py" "$ROOT" "$APP_DIR/Contents/Resources"
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
mkdir -p "$STEPX_INSTALL_DIR/tools" "$STEPX_INSTALL_DIR/desktop" "$STEPX_DATA_DIR/applications" "$STEPX_DATA_DIR/icons" "$STEPX_DATA_DIR/icons/hicolor/256x256/apps" "$HOME/.local/bin"
install -m755 "$STEPX_BIN_SRC" "$STEPX_INSTALL_DIR/stepx-companion.new"
mv "$STEPX_INSTALL_DIR/stepx-companion.new" "$STEPX_INSTALL_DIR/stepx-companion"
ln -sfn "$STEPX_INSTALL_DIR/stepx-companion" "$HOME/.local/bin/stepx-companion"
python3 "$ROOT/scripts/stage-release-resources.py" "$ROOT" "$STEPX_INSTALL_DIR"
if [ -d "$ROOT/.tools/bin" ]; then
    cp -a "$ROOT/.tools/bin" "$ROOT/.tools/root" "$STEPX_INSTALL_DIR/tools/"
fi
if [ "${STEPX_SKIP_RUNTIME:-0}" != "1" ] && { [ ! -f "$STEPX_INSTALL_DIR/runtime/android-build-tools/lib/apk-identity.jar" ] || [ "${STEPX_FORCE_RUNTIME:-0}" = "1" ]; }; then
    if [ -z "${JAVA_HOME:-}" ] && [ ! -d "$HOME/.local/opt/android-studio/jbr" ] && [ -d "/opt/android-studio/jbr" ]; then
        export JAVA_HOME="/opt/android-studio/jbr"
    fi
    ANDROID_SDK_ROOT="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$HOME/Android/Sdk}}" JAVA_HOME="${JAVA_HOME:-$HOME/.local/opt/android-studio/jbr}" python3 "$ROOT/scripts/package-runtime.py" "$STEPX_INSTALL_DIR"
fi
python3 - "$STEPX_INSTALL_DIR/stepx-companion" "$STEPX_DATA_DIR" <<'PY'
import ast, hashlib, pathlib, shutil, subprocess, sys
raw_binary = sys.argv[1]
binary = raw_binary.replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%')
data_dir = pathlib.Path(sys.argv[2]).resolve()
entry = data_dir/'applications/stepx-companion.desktop'
source = pathlib.Path('assets/stepx_companion_icon.png')
digest = hashlib.sha256(source.read_bytes()).hexdigest()[:12]
for old_icon in (data_dir/'icons').glob('stepx-companion-*.png'):
    if old_icon.name != f'stepx-companion-{digest}.png':
        old_icon.unlink(missing_ok=True)
icon_path = data_dir/'icons'/f'stepx-companion-{digest}.png'
shutil.copy2(source, icon_path)
shutil.copy2(source, data_dir/'icons'/'stepx-companion.png')
shutil.copy2(source, data_dir/'icons/hicolor/256x256/apps/stepx-companion.png')
icon = str(icon_path).replace('\\', '\\\\')
entry_tmp = data_dir/'applications/.stepx-companion.desktop.tmp'
entry_tmp.write_text(f'''[Desktop Entry]
Type=Application
Name=StepX Companion
Name[zh_CN]=StepX Companion
Name[en]=StepX Companion
Comment=连接 StepX，协同手机与电脑
Exec="{binary}"
TryExec={raw_binary}
Icon={icon}
Terminal=false
StartupNotify=true
Categories=Utility;
StartupWMClass=stepx-companion
''')
entry_tmp.replace(entry)

# Ensure GNOME / Ubuntu Dock favorite-apps contains stepx-companion.desktop and refreshes its entry.
try:
    out = subprocess.check_output(
        ['gsettings', 'get', 'org.gnome.shell', 'favorite-apps'],
        stderr=subprocess.DEVNULL,
        text=True,
    ).strip()
    if out.startswith('@as '):
        out = out[4:].strip()
    favs = list(ast.literal_eval(out)) if out else []
    if isinstance(favs, list):
        target = 'stepx-companion.desktop'
        if target in favs:
            idx = favs.index(target)
            without = [x for x in favs if x != target]
            subprocess.run(
                ['gsettings', 'set', 'org.gnome.shell', 'favorite-apps', repr(without)],
                stderr=subprocess.DEVNULL,
                check=False,
            )
            without.insert(idx, target)
            subprocess.run(
                ['gsettings', 'set', 'org.gnome.shell', 'favorite-apps', repr(without)],
                stderr=subprocess.DEVNULL,
                check=False,
            )
        else:
            favs.append(target)
            subprocess.run(
                ['gsettings', 'set', 'org.gnome.shell', 'favorite-apps', repr(favs)],
                stderr=subprocess.DEVNULL,
                check=False,
            )
except Exception:
    pass
PY
if command -v update-desktop-database >/dev/null; then update-desktop-database "$STEPX_DATA_DIR/applications"; fi
if command -v gtk-update-icon-cache >/dev/null; then gtk-update-icon-cache -f -t "$STEPX_DATA_DIR/icons/hicolor" 2>/dev/null || true; fi
if [ "${STEPX_RESTART_RUNNING:-0}" = "1" ]; then
    killall -q stepx-companion 2>/dev/null || true
    sleep 0.2
    if command -v systemd-run >/dev/null 2>&1; then
        systemd-run --user --unit="stepx-companion-dock-$(date +%s)" --collect "$STEPX_INSTALL_DIR/stepx-companion" >/dev/null 2>&1 &
    else
        nohup "$STEPX_INSTALL_DIR/stepx-companion" </dev/null >/dev/null 2>&1 &
        disown 2>/dev/null || true
    fi
fi
echo "已安装：$STEPX_INSTALL_DIR/stepx-companion"
