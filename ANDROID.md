# Screen Hopper for Android

Your computers' mouse and keyboard can **hop onto an Android phone**, and the phone shares clipboard text and files with them.

## What it can and cannot do
Android does not let an app capture touches or keys system-wide, so the phone is always the *receiving* side:
- **Mouse onto the phone:** a pointer is drawn; clicks become taps, drags become swipes, the wheel scrolls, right-click = Back, middle-click = Home.
- **Typing onto the phone:** needs the *Screen Hopper Keyboard* (see below) to reach web pages and every app.
- **Clipboard:** text from a computer lands on the phone's clipboard at once. The phone's clipboard is sent when you open the app (Android hides the clipboard from background apps), or with the **Send clipboard** button.
- **Files:** send from the phone with *Choose files*; received files go to **Downloads/Screen Hopper**.
- Not possible: moving the phone's own touchscreen/keys to control the computers.

## First-time setup on the phone
1. Install `ScreenHopper-Android.apk` (allow "install unknown apps" for your file manager or browser).
2. Open the app and tap **Open Accessibility settings**. Under *Downloaded apps* turn on **Screen Hopper**.
   If Android says the setting is restricted: Settings > Apps > Screen Hopper > ⋮ menu > **Allow restricted settings**, then try again.
3. Enter the same **pairing key** as your computers (Settings > Pairing code), on the same Wi-Fi.
4. For typing: Settings tab > **Typing from your computer** > *Turn on in Settings*, enable **Screen Hopper Keyboard**, then *Choose keyboard* while a text box is open. Tap **Switch keyboard** on its bar to get your normal keyboard back.
   Optional one-time automation (phone plugged into a computer with USB debugging on):
   `adb shell pm grant com.screenhopper.app android.permission.WRITE_SECURE_SETTINGS`
   After that the phone switches to the Screen Hopper keyboard while a computer drives it and back afterwards.

## Building
One-time: `brew install openjdk@17`, `brew install --cask android-commandlinetools`, install SDK 34/36 + NDK 26 with `sdkmanager`, `rustup target add aarch64-linux-android`.
Then `./scripts/build-android.sh` (signs with `android-signing/screenhopper-release.jks`, which is git-ignored: **back it up**, you need the same key to update the app later).

Tests against an emulator: `adb forward tcp:25000 tcp:24987`, then
`ANDROID_E2E_CODE=<phone pairing key> ANDROID_E2E_DIR=/tmp/shots cargo test --lib android_ -- --ignored --nocapture`
