#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
JDK="${STEPX_JDK:-${JAVA_HOME:?Set JAVA_HOME to JDK 17}}"
SDK_JAR="${STEPX_ANDROID_JAR:-${ANDROID_SDK_ROOT:-${ANDROID_HOME:?Set ANDROID_HOME}}/platforms/android-36/android.jar}"
R8_JAR="${STEPX_R8_JAR:-${ANDROID_SDK_ROOT:-${ANDROID_HOME:?Set ANDROID_HOME}}/cmdline-tools/latest/lib/r8.jar}"
OUT="$ROOT/desktop/.runtime/bridge-build"
mkdir -p "$OUT/classes" "$OUT/dex" "$ROOT/desktop/assets"
"$JDK/bin/javac" -encoding UTF-8 -source 11 -target 11 -classpath "$SDK_JAR" -d "$OUT/classes" "$ROOT"/desktop/android-bridge/src/com/stepx/companion/*.java
mkdir -p "$OUT/tests"
"$JDK/bin/javac" -encoding UTF-8 -classpath "$OUT/classes" -d "$OUT/tests" "$ROOT"/desktop/android-bridge/tests/ContextReadPolicyTest.java
"$JDK/bin/java" -cp "$OUT/classes:$OUT/tests" com.stepx.companion.ContextReadPolicyTest
"$JDK/bin/jar" cf "$OUT/classes.jar" -C "$OUT/classes" .
"$JDK/bin/java" -cp "$R8_JAR" com.android.tools.r8.D8 --release --min-api 33 --lib "$SDK_JAR" --output "$OUT/dex" "$OUT/classes.jar"
"$JDK/bin/jar" cf "$ROOT/desktop/assets/stepx-bridge.jar" -C "$OUT/dex" classes.dex
echo 'Android bridge built.'
