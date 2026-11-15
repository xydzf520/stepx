#!/usr/bin/env bash
# Set up local ADB and scrcpy tools on Linux (Debian/Ubuntu) or macOS without requiring root.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TOOLS="$ROOT/.tools"
mkdir -p "$TOOLS/packages" "$TOOLS/root" "$TOOLS/bin"

file_sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

download_github_verified() {
    local url="$1"
    local dest="$2"
    local expected_sha="$3"
    if [ -f "$dest" ] && [ "$(file_sha256 "$dest")" = "$expected_sha" ]; then
        return 0
    fi
    local pending="${dest}.download"
    local candidate
    for candidate in "$url" "https://ghfast.top/$url" "https://gh-proxy.com/$url"; do
        rm -f "$pending"
        if curl -fL --connect-timeout 10 --max-time 120 "$candidate" -o "$pending"; then
            if [ "$(file_sha256 "$pending")" = "$expected_sha" ]; then
                mv -f "$pending" "$dest"
                return 0
            fi
            echo "SHA-256 mismatch from $candidate; trying next mirror..." >&2
        else
            echo "Download failed from $candidate; trying next mirror..." >&2
        fi
    done
    rm -f "$pending"
    echo "Failed to download and verify $url" >&2
    return 1
}

OS="$(uname -s)"
if [ "$OS" = "Linux" ] && [ "$(uname -m)" = "x86_64" ]; then
    # Pin the matching official client/server. Distribution packages may be too
    # old for Android 16 and --keep-active; the portable build needs no sudo.
    VERSION=5.0.1
    ARCHIVE="scrcpy-linux-x86_64-v$VERSION.tar.gz"
    CHECKSUM=9f969d30cc574816077edecda65719c1c70b0aee1df1ba5fa00779ac07b8fd6e
    download_github_verified \
        "https://github.com/Genymobile/scrcpy/releases/download/v$VERSION/$ARCHIVE" \
        "$TOOLS/packages/$ARCHIVE" \
        "$CHECKSUM"
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
            CHECKSUM="33611e51977a8289e2e124b220f727895181ef2eb9060e0987a4dc2db9cf0d6b"
        else
            MAC_DIR="scrcpy-macos-x86_64-v5.0.1"
            ARCHIVE="scrcpy-macos-x86_64-v5.0.1.tar.gz"
            CHECKSUM="31a5467a9e907f162b093267cbbab1276b15c6e709c2ea8159566a8d165beb50"
        fi
        echo "Downloading official $ARCHIVE for macOS into .tools..."
        download_github_verified \
            "https://github.com/Genymobile/scrcpy/releases/download/v5.0.1/$ARCHIVE" \
            "$TOOLS/packages/$ARCHIVE" \
            "$CHECKSUM"
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
