# Source this before building the Android app:  source scripts/android-env.sh
export JAVA_HOME=/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home
export ANDROID_HOME=$HOME/Library/Android/sdk
export ANDROID_SDK_ROOT=$ANDROID_HOME
export NDK_HOME=$ANDROID_HOME/ndk/26.1.10909125
export PATH="$JAVA_HOME/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$HOME/.cargo/bin:$PATH"
export LANG=en_US.UTF-8
export PATH="$ANDROID_HOME/cmdline-tools/latest/bin:$PATH"
