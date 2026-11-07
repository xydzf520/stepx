#!/usr/bin/env bash
set -euo pipefail
TASK_NATIVE_GUI_PROBE=false
case "${1:-}" in
  "") ;;
  --native-gui-probe) TASK_NATIVE_GUI_PROBE=true; shift ;;
  *) printf 'Usage: %s [--native-gui-probe]\n' "$0" >&2; exit 2 ;;
esac
if [[ $# -ne 0 ]]; then
  printf 'Usage: %s [--native-gui-probe]\n' "$0" >&2
  exit 2
fi
TASK_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TASK_JDK="${STEPX_JDK:-${JAVA_HOME:?Set JAVA_HOME to JDK 17}}"
TASK_SDK="${ANDROID_SDK_ROOT:-${ANDROID_HOME:?Set ANDROID_HOME to Android SDK}}"
TASK_BT="$TASK_SDK/build-tools/36.0.0"
TASK_API="$TASK_SDK/platforms/android-36/android.jar"
TASK_SRC="$TASK_ROOT/desktop/companion-app"
TASK_OUT="$TASK_ROOT/desktop/.runtime/companion-build"
TASK_APK="$TASK_ROOT/desktop/assets/stepx-companion.apk"
if "$TASK_NATIVE_GUI_PROBE"; then
  TASK_OUT="$TASK_ROOT/desktop/.runtime/native-gui-research/build"
  TASK_APK="$TASK_ROOT/desktop/.runtime/native-gui-research/stepx-companion-probe.apk"
fi
TASK_ZXING="$TASK_SRC/libs/zxing-core-3.5.4.jar"
printf '%s  %s\n' '71de5d89341b5fcf5dd89da7f44e84d825d0e084cdf3ec77c9abe26b0f0ceb13' "$TASK_ZXING" | sha256sum --check --status
TASK_KEY_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/stepx-companion/signing"
# Never allow classes from an earlier build mode to leak into a new APK.
rm -rf "$TASK_OUT/classes" "$TASK_OUT/api-check" "$TASK_OUT/tests" "$TASK_OUT/dex" "$TASK_OUT/generated"
mkdir -p "$TASK_OUT/classes" "$TASK_OUT/api-check" "$TASK_OUT/tests" "$TASK_OUT/dex" "$TASK_OUT/generated" "$(dirname "$TASK_APK")" "$TASK_KEY_DIR"
chmod 700 "$TASK_KEY_DIR"
if [[ ! -f "$TASK_KEY_DIR/companion.pass" ]]; then
  (umask 077; python3 -c 'import secrets; print(secrets.token_urlsafe(40))' > "$TASK_KEY_DIR/companion.pass")
fi
if [[ ! -f "$TASK_KEY_DIR/companion.p12" ]]; then
  umask 077
  "$TASK_JDK/bin/keytool" -genkeypair -keystore "$TASK_KEY_DIR/companion.p12" -storetype PKCS12 -storepass:file "$TASK_KEY_DIR/companion.pass" -keypass:file "$TASK_KEY_DIR/companion.pass" -alias companion -keyalg RSA -keysize 3072 -validity 10000 -dname 'CN=StepX Companion Local Build' >/dev/null 2>&1
fi
TASK_MANIFEST="$TASK_SRC/AndroidManifest.xml"
TASK_JAVA_SOURCES=("$TASK_SRC"/src/com/stepx/companion/app/*.java)
if "$TASK_NATIVE_GUI_PROBE"; then
  TASK_MANIFEST="$TASK_OUT/generated/AndroidManifest.xml"
  python3 - "$TASK_SRC/AndroidManifest.xml" "$TASK_SRC/research/native-gui/AndroidManifest.xml" "$TASK_MANIFEST" <<'PY'
import copy, sys, xml.etree.ElementTree as ET
ET.register_namespace('android', 'http://schemas.android.com/apk/res/android')
base = ET.parse(sys.argv[1])
application = base.getroot().find('application')
overlay = ET.parse(sys.argv[2]).getroot().find('application')
if application is None or overlay is None:
    raise SystemExit('Missing companion application manifest')
for component in overlay:
    application.append(copy.deepcopy(component))
base.write(sys.argv[3], encoding='utf-8', xml_declaration=True)
PY
  TASK_JAVA_SOURCES+=("$TASK_SRC"/research/native-gui/src/com/stepx/companion/app/*.java)
fi
"$TASK_BT/aapt2" compile --dir "$TASK_SRC/res" -o "$TASK_OUT/resources.zip"
"$TASK_BT/aapt2" link -o "$TASK_OUT/unsigned.apk" -I "$TASK_API" --manifest "$TASK_MANIFEST" --java "$TASK_OUT/generated" "$TASK_OUT/resources.zip"
# Resolve standard-library APIs against Android too. The host JDK otherwise
# accepts methods such as Files.readString that crash on the phone. The core
# jar supplies the lambda compiler stubs omitted from the public Android jar.
"$TASK_JDK/bin/javac" -encoding UTF-8 -source 8 -target 8 -Xlint:-options -bootclasspath "$(dirname "$TASK_API")/core-for-system-modules.jar:$TASK_API" -classpath "$TASK_ZXING" -d "$TASK_OUT/api-check" "${TASK_JAVA_SOURCES[@]}"
"$TASK_JDK/bin/javac" -encoding UTF-8 -source 11 -target 11 -classpath "$TASK_API:$TASK_ZXING" -d "$TASK_OUT/classes" "${TASK_JAVA_SOURCES[@]}"
"$TASK_JDK/bin/javac" -encoding UTF-8 -source 11 -target 11 -classpath "$TASK_OUT/classes:$TASK_API:$TASK_ZXING" -d "$TASK_OUT/tests" "$TASK_SRC"/tests/*.java
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests" com.stepx.companion.app.GameCoreTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests" com.stepx.companion.app.ExpansionRulesTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests" com.stepx.companion.app.GuiTaskStateTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests" com.stepx.companion.app.GelabActionTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests" com.stepx.companion.app.GuiRetryPolicyTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests" com.stepx.companion.app.NeoPolicyTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests" com.stepx.companion.app.ChatReplyPolicyTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests" com.stepx.companion.app.McpRoutePolicyTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests" com.stepx.companion.app.OfficialGuiSkillPolicyTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests" com.stepx.companion.app.AmooLeaseTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests:$TASK_API:$TASK_ZXING" com.stepx.companion.app.PairingCodeTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests:$TASK_API" com.stepx.companion.app.StudioWebPolicyTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests:$TASK_API" com.stepx.companion.app.StudioLocalPolicyTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests:$TASK_API" com.stepx.companion.app.ImPushCatalogTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests" com.stepx.companion.app.ContextProvenanceTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests" com.stepx.companion.app.DesktopWorkPolicyTest
"$TASK_JDK/bin/java" -cp "$TASK_OUT/classes:$TASK_OUT/tests:$TASK_API" com.stepx.companion.app.TransferPreviewTest
"$TASK_JDK/bin/jar" cf "$TASK_OUT/classes.jar" -C "$TASK_OUT/classes" com/stepx/companion/app
"$TASK_JDK/bin/java" -cp "$TASK_SDK/cmdline-tools/latest/lib/r8.jar" com.android.tools.r8.D8 --release --min-api 33 --lib "$TASK_API" --output "$TASK_OUT/dex" "$TASK_OUT/classes.jar" "$TASK_ZXING"
python3 - "$TASK_OUT/unsigned.apk" "$TASK_OUT/dex/classes.dex" <<'PY'
import sys,zipfile
with zipfile.ZipFile(sys.argv[1],'a',compression=zipfile.ZIP_DEFLATED) as z:z.write(sys.argv[2],'classes.dex')
PY
"$TASK_BT/zipalign" -f 4 "$TASK_OUT/unsigned.apk" "$TASK_OUT/aligned.apk"
# V4 sidecars are unnecessary for regular adb installs. Disable V4 explicitly
# so a previous .idsig can never silently describe a different APK.
rm -f "$TASK_OUT/signed.apk.idsig"
JAVA_HOME="$TASK_JDK" PATH="$TASK_JDK/bin:$PATH" "$TASK_BT/apksigner" sign --v4-signing-enabled false --ks "$TASK_KEY_DIR/companion.p12" --ks-key-alias companion --ks-pass "file:$TASK_KEY_DIR/companion.pass" --out "$TASK_OUT/signed.apk" "$TASK_OUT/aligned.apk"
JAVA_HOME="$TASK_JDK" PATH="$TASK_JDK/bin:$PATH" "$TASK_BT/apksigner" verify "$TASK_OUT/signed.apk"
cp "$TASK_OUT/signed.apk" "$TASK_APK.tmp"
mv -f "$TASK_APK.tmp" "$TASK_APK"
rm -f "$TASK_APK.idsig"
printf 'Built StepX伴侣%s: %s\n' "$(if "$TASK_NATIVE_GUI_PROBE"; then printf ' (native GUI research probe)'; fi)" "${TASK_APK#"$TASK_ROOT"/}"
