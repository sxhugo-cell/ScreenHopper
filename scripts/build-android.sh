#!/bin/zsh
# Build the signed Android APK on a Mac. One-time setup is described in ANDROID.md.
set -e
cd "$(dirname "$0")/.."
source scripts/android-env.sh
npx tauri android build --apk --target aarch64
APK=src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
"$ANDROID_HOME/build-tools/34.0.0/apksigner" verify --print-certs "$APK" | head -3
mkdir -p dist && cp "$APK" dist/ScreenHopper-Android.apk
ls -la dist/ScreenHopper-Android.apk
