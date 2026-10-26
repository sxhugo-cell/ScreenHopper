<div align="center">

<br>

<img src="src-tauri/icons/128x128@2x.png" width="112" alt="Screen Hopper icon">

# Screen Hopper

### One mouse. Four computers.

**Push your cursor off the edge of a screen and it hops to the next computer.**<br>
Mouse, keyboard, clipboard and files. All on your own network.

A free, open-source **software KVM**: share one mouse and keyboard across multiple computers.<br>
A Synergy, Barrier and Input Leap alternative, built with Rust and Tauri.

<br>

[![Version](https://img.shields.io/badge/version-1.0.0-14a37f?style=for-the-badge)](#what-it-does)
[![macOS](https://img.shields.io/badge/macOS-menu--bar-18222d?style=for-the-badge&logo=apple&logoColor=white)](#platforms)
[![Windows](https://img.shields.io/badge/Windows-installer-18222d?style=for-the-badge&logo=windows&logoColor=white)](#platforms)
[![Linux](https://img.shields.io/badge/Linux-Wayland-18222d?style=for-the-badge&logo=linux&logoColor=white)](#platforms)
[![Android](https://img.shields.io/badge/Android-receive-18222d?style=for-the-badge&logo=android&logoColor=white)](#platforms)

<br>

<img src="docs/media/preview.gif" width="800" alt="Screen Hopper: one mouse, four computers">

**[Features](#what-it-does)** &nbsp;·&nbsp; **[How it works](#how-it-works)** &nbsp;·&nbsp; **[Platforms](#platforms)**

<br>

</div>

---

## The idea

You have more than one computer on your desk. You should not need more than one mouse.

Arrange your screens the way they sit on your desk, push the cursor off the edge of one, and it appears on the other. The keyboard follows. So do your copied text and your files.

<div align="center">

<img src="docs/media/hop.jpg" width="860" alt="The cursor hopping from Mac to Windows, Android and Linux">

</div>

---

## What it does

<table>
<tr>
<td width="50%" valign="top">

### 🖱️ Edge hopping
Drag the screens in **Layout** to match your desk, like Windows display settings. Push the cursor off any shared edge and it hops. It even hops between two *other* computers.

</td>
<td width="50%" valign="top">

### ⌨️ Keyboard follows
Type on whichever computer the cursor is on. Cmd and Ctrl can be swapped between Mac and Windows, so Cmd+C and Ctrl+C both copy.

</td>
</tr>
<tr>
<td valign="top">

### 📋 Shared clipboard
Text, rich text and images. Copy on one computer, paste on another. Copied files travel too.

</td>
<td valign="top">

### 📁 File transfer
Drop files or folders onto a computer, or copy in Finder or Explorer and paste on the other side. Received files land in a folder you choose.

</td>
</tr>
<tr>
<td valign="top">

### 🌗 Dim and pause
Dim the screen that is not in control. Pause hopping from the app, or hit the panic shortcut <kbd>Ctrl</kbd> <kbd>Alt</kbd> <kbd>Shift</kbd> <kbd>Esc</kbd> to take the cursor back.

</td>
<td valign="top">

### 🔒 Private by design
Everything is encrypted end to end and never leaves your local network. No account, no cloud, no server.

</td>
</tr>
</table>

<div align="center">

<img src="docs/media/computers.jpg" width="860" alt="The Computers tab with the pairing key and three connected computers">

<sub>The Computers tab. Same pairing key on every computer, and they find each other.</sub>

</div>

---

## How it works

1. **Install Screen Hopper on each computer** and open it.
2. **Use the same pairing key** on all of them. Computers on the same network find each other automatically (or connect by IP).
3. **Arrange your screens** in the Layout tab.
4. **Push the cursor off an edge.** That's it.

---

## Platforms

| | Platform | What it does | Notes |
|---|---|---|---|
| 🍎 | **macOS** | Menu-bar app. Full control both ways. | Needs Accessibility and Input Monitoring. |
| 🪟 | **Windows** | Installer (NSIS). Full control both ways. | Unsigned, so SmartScreen may warn. The pause shortcut is Mac-only. |
| 🐧 | **Linux** | `.deb` for Wayland (GNOME 46+, KDE Plasma 6.1+). | See [LINUX.md](LINUX.md). |
| 🤖 | **Android** | Your computers' mouse and keyboard hop onto the phone or tablet. | Receive side only. See [ANDROID.md](ANDROID.md). |

> **Honest status:** macOS is the most tested. The Windows, Linux and Android builds are newer and have seen less real-world use, so expect rough edges and please report them.

More detail: [Linux](LINUX.md) · [Android](ANDROID.md)

---

## License

MIT. Free to use, change and share. See [LICENSE](LICENSE).

<div align="center"><br><sub>Screen Hopper 1.0.0</sub></div>
