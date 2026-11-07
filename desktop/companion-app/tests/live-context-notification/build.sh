#!/usr/bin/env bash
# Build only this independent notification fixture. Never installs or contacts a phone.
set -euo pipefail

if [[ $# -ne 0 ]]; then
  printf 'Usage: %s\nConfigure STEPX_JDK, ANDROID_SDK_ROOT and optional SDK version overrides via environment.\n' "$0" >&2
  exit 2
fi
TASK_SRC="$(cd "$(dirname "$0")" && pwd)"
TASK_OUT="$TASK_SRC/.build"
command -v python3 >/dev/null || { printf 'python3 is required.\n' >&2; exit 1; }

TASK_JDK="${STEPX_JDK:-${JAVA_HOME:-}}"
if [[ -z "$TASK_JDK" ]]; then
  TASK_JAVAC="$(command -v javac || true)"
  [[ -n "$TASK_JAVAC" ]] || { printf 'Set STEPX_JDK to a JDK containing java, javac, jar and keytool.\n' >&2; exit 1; }
  TASK_JDK="$(python3 - "$TASK_JAVAC" <<'PY'
from pathlib import Path
import sys
print(Path(sys.argv[1]).resolve().parent.parent)
PY
)"
fi
for TASK_TOOL in java javac jar keytool; do
  [[ -x "$TASK_JDK/bin/$TASK_TOOL" ]] || { printf 'Missing JDK tool: %s\n' "$TASK_TOOL" >&2; exit 1; }
done

TASK_SDK="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
if [[ -z "$TASK_SDK" ]]; then
  for TASK_CANDIDATE in "$HOME/Android/Sdk" "$HOME/Library/Android/sdk"; do
    if [[ -d "$TASK_CANDIDATE/build-tools" ]]; then TASK_SDK="$TASK_CANDIDATE"; break; fi
  done
fi
[[ -n "$TASK_SDK" ]] || { printf 'Set ANDROID_SDK_ROOT or ANDROID_HOME to an existing Android SDK.\n' >&2; exit 1; }
TASK_API_VERSION="${STEPX_ANDROID_API:-36}"
TASK_BT_VERSION="${STEPX_ANDROID_BUILD_TOOLS:-36.0.0}"
TASK_API="${STEPX_ANDROID_JAR:-$TASK_SDK/platforms/android-$TASK_API_VERSION/android.jar}"
TASK_BT="$TASK_SDK/build-tools/$TASK_BT_VERSION"
TASK_R8="${STEPX_R8_JAR:-$TASK_SDK/cmdline-tools/latest/lib/r8.jar}"
for TASK_TOOL in aapt2 zipalign apksigner; do
  [[ -x "$TASK_BT/$TASK_TOOL" ]] || { printf 'Missing Android build tool: %s (build-tools %s)\n' "$TASK_TOOL" "$TASK_BT_VERSION" >&2; exit 1; }
done
[[ -f "$TASK_API" && -f "$TASK_R8" ]] || { printf 'Android platform jar or R8 jar missing; install the requested SDK packages first.\n' >&2; exit 1; }

# Keep the test identity between builds; never borrow the companion production key.
umask 077
mkdir -p "$TASK_OUT/signing"
TASK_KEY="$TASK_OUT/signing/test-notification.p12"
TASK_PASS="$TASK_OUT/signing/test-notification.pass"
if [[ -f "$TASK_KEY" && ! -f "$TASK_PASS" ]]; then
  printf 'Existing test signing key has no password file; restore its original signing files.\n' >&2
  exit 1
fi
if [[ ! -f "$TASK_PASS" ]]; then
  python3 -c 'import secrets; print(secrets.token_urlsafe(40))' > "$TASK_PASS"
fi
if [[ ! -f "$TASK_KEY" ]]; then
  "$TASK_JDK/bin/keytool" -genkeypair -keystore "$TASK_KEY" -storetype PKCS12 \
    -storepass:file "$TASK_PASS" -keypass:file "$TASK_PASS" -alias context-verification \
    -keyalg RSA -keysize 3072 -validity 3650 -dname 'CN=StepX Context Verification Only' >/dev/null 2>&1
fi

# Only transient build directories are replaced; the signing identity remains.
rm -rf "$TASK_OUT/classes" "$TASK_OUT/api-check" "$TASK_OUT/dex"
mkdir -p "$TASK_OUT/classes" "$TASK_OUT/api-check" "$TASK_OUT/dex"
"$TASK_BT/aapt2" link -o "$TASK_OUT/unsigned.apk" -I "$TASK_API" --manifest "$TASK_SRC/AndroidManifest.xml"
TASK_BOOT="$TASK_API"
if [[ -f "$(dirname "$TASK_API")/core-for-system-modules.jar" ]]; then
  TASK_BOOT="$(dirname "$TASK_API")/core-for-system-modules.jar:$TASK_API"
fi
"$TASK_JDK/bin/javac" -encoding UTF-8 -source 8 -target 8 -Xlint:-options \
  -bootclasspath "$TASK_BOOT" -d "$TASK_OUT/api-check" "$TASK_SRC"/*.java
"$TASK_JDK/bin/javac" -encoding UTF-8 -source 11 -target 11 -Xlint:-options \
  -classpath "$TASK_API" -d "$TASK_OUT/classes" "$TASK_SRC"/*.java
"$TASK_JDK/bin/jar" cf "$TASK_OUT/classes.jar" -C "$TASK_OUT/classes" .
"$TASK_JDK/bin/java" -cp "$TASK_R8" com.android.tools.r8.D8 --release --min-api 33 \
  --lib "$TASK_API" --output "$TASK_OUT/dex" "$TASK_OUT/classes.jar"
python3 - "$TASK_OUT/unsigned.apk" "$TASK_OUT/dex/classes.dex" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], 'a', compression=zipfile.ZIP_DEFLATED) as apk:
    apk.write(sys.argv[2], 'classes.dex')
PY
"$TASK_BT/zipalign" -f 4 "$TASK_OUT/unsigned.apk" "$TASK_OUT/aligned.apk"
rm -f "$TASK_OUT/stepx-context-verification.apk.idsig"
JAVA_HOME="$TASK_JDK" PATH="$TASK_JDK/bin:$PATH" "$TASK_BT/apksigner" sign \
  --v4-signing-enabled false --ks "$TASK_KEY" --ks-key-alias context-verification \
  --ks-pass "file:$TASK_PASS" --out "$TASK_OUT/stepx-context-verification.apk" "$TASK_OUT/aligned.apk"
JAVA_HOME="$TASK_JDK" PATH="$TASK_JDK/bin:$PATH" "$TASK_BT/apksigner" verify "$TASK_OUT/stepx-context-verification.apk"
"$TASK_BT/zipalign" -c 4 "$TASK_OUT/stepx-context-verification.apk"
"$TASK_BT/aapt2" dump badging "$TASK_OUT/stepx-context-verification.apk" > "$TASK_OUT/apk-metadata.txt"
python3 - "$TASK_OUT/stepx-context-verification.apk" <<'PY'
import hashlib, pathlib, sys
path = pathlib.Path(sys.argv[1])
print('Built independent notification test APK:', path)
print('SHA-256:', hashlib.sha256(path.read_bytes()).hexdigest())
print('No phone was contacted or modified.')
PY
