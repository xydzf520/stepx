#!/usr/bin/env bash
set -euo pipefail
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
JDK="${STEPX_JDK:-${JAVA_HOME:?Set JAVA_HOME to JDK 17}}"
SDK="${ANDROID_SDK_ROOT:-${ANDROID_HOME:?Set ANDROID_HOME to Android SDK}}"
BUILD="$PROJECT_ROOT/desktop/.runtime/companion-permissions-build"
mkdir -p "$BUILD/classes" "$BUILD/dex" "$PROJECT_ROOT/desktop/assets"
find "$BUILD/classes" -name '*.class' -delete
"$JDK/bin/javac" -encoding UTF-8 -source 11 -target 11 -cp "$SDK/platforms/android-36/android.jar" -d "$BUILD/classes" "$PROJECT_ROOT/desktop/companion-permissions/CompanionGrant.java" "$PROJECT_ROOT/desktop/companion-permissions/CompanionRuntime.java" "$PROJECT_ROOT/desktop/companion-permissions/CompanionUpdateRuntime.java" "$PROJECT_ROOT/desktop/companion-permissions/GoogleLoginRuntime.java" "$PROJECT_ROOT/handoff/google-play-stepx/java/GmsProbe.java" "$PROJECT_ROOT/desktop/amoo-background/PairingDaemon.java" "$PROJECT_ROOT/desktop/amoo-background/ScopedPairing.java" "$PROJECT_ROOT/desktop/amoo-background/PairingProviders.java" "$PROJECT_ROOT/desktop/amoo-background/MdiTransport.java" "$PROJECT_ROOT/desktop/amoo-background/MdiBroker.java"
"$JDK/bin/jar" cf "$BUILD/classes.jar" -C "$BUILD/classes" .
"$JDK/bin/java" -cp "$SDK/cmdline-tools/latest/lib/r8.jar" com.android.tools.r8.D8 --release --min-api 36 --lib "$SDK/platforms/android-36/android.jar" --output "$BUILD/dex" "$BUILD/classes.jar"
cp "$BUILD/dex/classes.dex" "$PROJECT_ROOT/desktop/assets/stepx-companion-permissions.dex"
echo 'StepX伴侣 scoped permission helper built.'
