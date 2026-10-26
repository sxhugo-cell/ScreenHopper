#!/bin/zsh
# Build the Windows installer (NSIS setup.exe) on a Mac, without GitHub Actions.
# One-time setup:
#   brew install nsis llvm lld
#   cargo install --locked cargo-xwin
#   rustup target add x86_64-pc-windows-msvc
# cargo-xwin downloads Microsoft's Windows SDK/CRT libraries on first run (you accept their license).
set -e
cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/opt/llvm/bin:/opt/homebrew/opt/lld/bin:$PATH"
export XWIN_ACCEPT_LICENSE=1
# makensis crashes (std::bad_alloc) when no locale is set, so always set one.
export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8
source ~/.cargo/env
npx tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis
ls -la src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/*.exe
