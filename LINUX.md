# Screen Hopper on Linux

Ubuntu 22.04+ / Fedora 36+ / Debian 12+, **Wayland** desktops (GNOME, KDE). x86_64 `.deb`.

## Install
1. Download `ScreenHopper-Linux.deb` from the latest release.
2. `sudo apt install ./ScreenHopper-Linux.deb`.
3. **Log out and back in once** (the install adds a rule that lets you use the virtual mouse/keyboard).
4. Start "Screen Hopper" from the app menu. Use the same pairing code as your other computers.
5. The first time you push the cursor past a screen edge, GNOME/KDE asks to allow **input capture**: allow it.

## What works
| Feature | How on Linux |
|---|---|
| Control other computers from Linux | Input Capture portal (barriers on screen edges) + libei. Needs **GNOME 46+** (Ubuntu 24.04, Fedora 40) or **KDE Plasma 6.1+** |
| Be controlled from other computers | Virtual mouse (absolute, like a VM tablet) + keyboard via `/dev/uinput` |
| Clipboard, files | Same code as Mac/Windows |

## Known limits / untested
- Not yet run on a real Linux desktop: only built, unit-tested (15 tests) and smoke-started in Ubuntu 24.04 Docker.
- Touching a screen edge with no computer beyond it briefly captures the cursor and gives it straight back (Wayland does not let apps change edges after start).
- Older desktops (GNOME 45 and below, X11 sessions) cannot control other computers; they can still be controlled.
- Code: `src-tauri/src/platform/linux.rs`; build: `scripts/build-linux-on-mac.sh` (Docker/Colima, see the script header).
