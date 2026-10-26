#!/bin/zsh
# Build the Linux .deb (x86_64) on a Mac inside Docker (Colima with Rosetta).
# One-time setup:  brew install colima docker docker-buildx
#                  colima start --vm-type vz --vz-rosetta --cpu 6 --memory 8 --disk 60
#                  docker build --platform linux/amd64 -t screenhopper-linux scripts/linux
set -e
cd "$(dirname "$0")/.."
colima status >/dev/null 2>&1 || colima start
docker run --rm --platform linux/amd64 \
  -v "$PWD":/src -v sh-cargo:/root/.cargo/registry -v sh-target:/src/src-tauri/target-linux -v sh-node:/src/node_modules \
  -e CARGO_TARGET_DIR=/src/src-tauri/target-linux -w /src screenhopper-linux \
  sh -c 'npm ci --no-audit --no-fund >/dev/null && npx tauri build --bundles deb && cp src-tauri/target-linux/release/bundle/deb/*.deb /src/dist/ScreenHopper-Linux.deb.tmp'
mv dist/ScreenHopper-Linux.deb.tmp dist/ScreenHopper-Linux.deb
ls -la dist/ScreenHopper-Linux.deb
