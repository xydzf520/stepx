#!/usr/bin/env bash
# Set up local ADB and scrcpy tools on Linux (Debian/Ubuntu) or macOS without requiring root.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TOOLS="$ROOT/.tools"
mkdir -p "$TOOLS/packages" "$TOOLS/root" "$TOOLS/bin"

OS="$(uname -s)"
if [ "$OS" = "Linux" ] && [ "$(uname -m)" = "x86_64" ]; then
    # Pin the matching official client/server. Distribution packages may be too
    # old for Android 16 and --keep-active; the portable build needs no sudo.
    VERSION=5.0.1
    ARCHIVE="scrcpy-linux-x86_64-v$VERSION.tar.gz"
    CHECKSUM=9f969d30cc574816077edecda65719c1c70b0aee1df1ba5fa00779ac07b8fd6e
    if ! (cd "$TOOLS/packages" && echo "$CHECKSUM  $ARCHIVE" | sha256sum --check --status 2>/dev/null); then
        curl -fL --connect-timeout 10 --max-time 120 \
            "https://github.com/Genymobile/scrcpy/releases/download/v$VERSION/$ARCHIVE" -o "$TOOLS/packages/$ARCHIVE.download"
        (cd "$TOOLS/packages" && echo "$CHECKSUM  $ARCHIVE.download" | sha256sum --check)
        mv "$TOOLS/packages/$ARCHIVE.download" "$TOOLS/packages/$ARCHIVE"
    fi
    tar -xzf "$TOOLS/packages/$ARCHIVE" -C "$TOOLS/root"
    cat > "$TOOLS/bin/adb" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail
TOOLS="$(cd "$(dirname "$0")/.." && pwd)"
export ADB_MDNS_OPENSCREEN=1
exec "$TOOLS/root/scrcpy-linux-x86_64-v5.0.1/adb" "$@"
WRAPPER
    cat > "$TOOLS/bin/scrcpy" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail
TOOLS="$(cd "$(dirname "$0")/.." && pwd)"
export ADB="${ADB:-$TOOLS/bin/adb}"
export SCRCPY_SERVER_PATH="$TOOLS/root/scrcpy-linux-x86_64-v5.0.1/scrcpy-server"
exec "$TOOLS/root/scrcpy-linux-x86_64-v5.0.1/scrcpy" "$@"
WRAPPER
    chmod +x "$TOOLS/bin/adb" "$TOOLS/bin/scrcpy"
    "$TOOLS/bin/adb" version
    "$TOOLS/bin/scrcpy" --version
    echo 'StepX Companion local tools are ready.'
    exit 0
fi
if [ "$OS" = "Darwin" ]; then
    if command -v brew >/dev/null 2>&1; then
        echo "Detected macOS with Homebrew. Installing/linking android-platform-tools and scrcpy..."
        brew list --versions android-platform-tools >/dev/null 2>&1 || brew install --cask android-platform-tools || true
        brew list --versions scrcpy >/dev/null 2>&1 || brew install scrcpy || true
        command -v adb >/dev/null 2>&1 && ln -sf "$(command -v adb)" "$TOOLS/bin/adb" || true
        command -v scrcpy >/dev/null 2>&1 && ln -sf "$(command -v scrcpy)" "$TOOLS/bin/scrcpy" || true
    fi
    if [ ! -x "$TOOLS/bin/scrcpy" ] || ! "$TOOLS/bin/scrcpy" --help 2>&1 | grep -q -- '--keep-active'; then
        if [ "$(uname -m)" = "arm64" ] || [ "$(uname -m)" = "aarch64" ]; then
            MAC_DIR="scrcpy-macos-aarch64-v5.0.1"
            ARCHIVE="scrcpy-macos-aarch64-v5.0.1.tar.gz"
        else
            MAC_DIR="scrcpy-macos-x86_64-v5.0.1"
            ARCHIVE="scrcpy-macos-x86_64-v5.0.1.tar.gz"
        fi
        echo "Downloading official $ARCHIVE for macOS into .tools..."
        curl -fL --connect-timeout 10 --max-time 120 \
            "https://github.com/Genymobile/scrcpy/releases/download/v5.0.1/$ARCHIVE" -o "$TOOLS/packages/$ARCHIVE"
        tar -xzf "$TOOLS/packages/$ARCHIVE" -C "$TOOLS/root"
        rm -f "$TOOLS/bin/adb" "$TOOLS/bin/scrcpy"
        cat > "$TOOLS/bin/adb" <<WRAPPER
#!/usr/bin/env bash
set -euo pipefail
TOOLS="\$(cd "\$(dirname "\$0")/.." && pwd)"
export ADB_MDNS_OPENSCREEN=1
exec "\$TOOLS/root/$MAC_DIR/adb" "\$@"
WRAPPER
        cat > "$TOOLS/bin/scrcpy" <<WRAPPER
#!/usr/bin/env bash
set -euo pipefail
TOOLS="\$(cd "\$(dirname "\$0")/.." && pwd)"
export ADB="\${ADB:-\$TOOLS/bin/adb}"
export SCRCPY_SERVER_PATH="\$TOOLS/root/$MAC_DIR/scrcpy-server"
exec "\$TOOLS/root/$MAC_DIR/scrcpy" "\$@"
WRAPPER
        chmod +x "$TOOLS/bin/adb" "$TOOLS/bin/scrcpy"
    fi
    "$TOOLS/bin/adb" version
    "$TOOLS/bin/scrcpy" --version
    exit 0
fi

command -v apt >/dev/null || { echo 'Install adb and scrcpy with your OS package manager.'; exit 1; }
python3 "$ROOT/scripts/setup-desktop-tools.py"
cat > "$TOOLS/bin/adb" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail
TOOLS="$(cd "$(dirname "$0")/.." && pwd)"
ARCH="$(dpkg-architecture -qDEB_HOST_MULTIARCH)"
export LD_LIBRARY_PATH="$TOOLS/root/usr/lib/$ARCH/android:$TOOLS/root/usr/lib/$ARCH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export ADB_MDNS_OPENSCREEN=1
exec "$TOOLS/root/usr/bin/adb" "$@"
WRAPPER
chmod +x "$TOOLS/bin/adb"
cat > "$TOOLS/bin/scrcpy" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail
TOOLS="$(cd "$(dirname "$0")/.." && pwd)"
ARCH="$(dpkg-architecture -qDEB_HOST_MULTIARCH)"
export LD_LIBRARY_PATH="$TOOLS/root/usr/lib/$ARCH:$TOOLS/root/usr/lib/$ARCH/android${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export ADB="${ADB:-$TOOLS/bin/adb}"
export SCRCPY_SERVER_PATH="$TOOLS/root/usr/share/scrcpy/scrcpy-server"
exec "$TOOLS/root/usr/bin/scrcpy" "$@"
WRAPPER
chmod +x "$TOOLS/bin/scrcpy"
"$TOOLS/bin/adb" version
"$TOOLS/bin/scrcpy" --version
echo 'StepX Companion local tools are ready.'
