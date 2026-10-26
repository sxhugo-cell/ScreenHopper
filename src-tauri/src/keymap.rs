//! Cross-platform key table. The wire format is the USB HID usage ID (keyboard page).
//! Columns: HID usage, macOS virtual keycode, Windows set-1 scancode, Windows extended (E0) flag.

pub struct Key {
    pub hid: u16,
    pub mac: u16,
    pub win: u16,
    pub ext: bool,
}

const fn k(hid: u16, mac: u16, win: u16, ext: bool) -> Key {
    Key { hid, mac, win, ext }
}

pub const KEYS: &[Key] = &[
    // Letters
    k(0x04, 0x00, 0x1E, false), // A
    k(0x05, 0x0B, 0x30, false), // B
    k(0x06, 0x08, 0x2E, false), // C
    k(0x07, 0x02, 0x20, false), // D
    k(0x08, 0x0E, 0x12, false), // E
    k(0x09, 0x03, 0x21, false), // F
    k(0x0A, 0x05, 0x22, false), // G
    k(0x0B, 0x04, 0x23, false), // H
    k(0x0C, 0x22, 0x17, false), // I
    k(0x0D, 0x26, 0x24, false), // J
    k(0x0E, 0x28, 0x25, false), // K
    k(0x0F, 0x25, 0x26, false), // L
    k(0x10, 0x2E, 0x32, false), // M
    k(0x11, 0x2D, 0x31, false), // N
    k(0x12, 0x1F, 0x18, false), // O
    k(0x13, 0x23, 0x19, false), // P
    k(0x14, 0x0C, 0x10, false), // Q
    k(0x15, 0x0F, 0x13, false), // R
    k(0x16, 0x01, 0x1F, false), // S
    k(0x17, 0x11, 0x14, false), // T
    k(0x18, 0x20, 0x16, false), // U
    k(0x19, 0x09, 0x2F, false), // V
    k(0x1A, 0x0D, 0x11, false), // W
    k(0x1B, 0x07, 0x2D, false), // X
    k(0x1C, 0x10, 0x15, false), // Y
    k(0x1D, 0x06, 0x2C, false), // Z
    // Digits
    k(0x1E, 0x12, 0x02, false), // 1
    k(0x1F, 0x13, 0x03, false), // 2
    k(0x20, 0x14, 0x04, false), // 3
    k(0x21, 0x15, 0x05, false), // 4
    k(0x22, 0x17, 0x06, false), // 5
    k(0x23, 0x16, 0x07, false), // 6
    k(0x24, 0x1A, 0x08, false), // 7
    k(0x25, 0x1C, 0x09, false), // 8
    k(0x26, 0x19, 0x0A, false), // 9
    k(0x27, 0x1D, 0x0B, false), // 0
    // Main block
    k(0x28, 0x24, 0x1C, false), // Return
    k(0x29, 0x35, 0x01, false), // Escape
    k(0x2A, 0x33, 0x0E, false), // Backspace
    k(0x2B, 0x30, 0x0F, false), // Tab
    k(0x2C, 0x31, 0x39, false), // Space
    k(0x2D, 0x1B, 0x0C, false), // -
    k(0x2E, 0x18, 0x0D, false), // =
    k(0x2F, 0x21, 0x1A, false), // [
    k(0x30, 0x1E, 0x1B, false), // ]
    k(0x31, 0x2A, 0x2B, false), // backslash
    k(0x32, 0x0A, 0x56, false), // ISO non-US backslash
    k(0x33, 0x29, 0x27, false), // ;
    k(0x34, 0x27, 0x28, false), // '
    k(0x35, 0x32, 0x29, false), // `
    k(0x36, 0x2B, 0x33, false), // ,
    k(0x37, 0x2F, 0x34, false), // .
    k(0x38, 0x2C, 0x35, false), // /
    k(0x39, 0x39, 0x3A, false), // Caps Lock
    // Function keys
    k(0x3A, 0x7A, 0x3B, false), // F1
    k(0x3B, 0x78, 0x3C, false), // F2
    k(0x3C, 0x63, 0x3D, false), // F3
    k(0x3D, 0x76, 0x3E, false), // F4
    k(0x3E, 0x60, 0x3F, false), // F5
    k(0x3F, 0x61, 0x40, false), // F6
    k(0x40, 0x62, 0x41, false), // F7
    k(0x41, 0x64, 0x42, false), // F8
    k(0x42, 0x65, 0x43, false), // F9
    k(0x43, 0x6D, 0x44, false), // F10
    k(0x44, 0x67, 0x57, false), // F11
    k(0x45, 0x6F, 0x58, false), // F12
    // Navigation
    k(0x49, 0x72, 0x52, true),  // Insert / Help
    k(0x4A, 0x73, 0x47, true),  // Home
    k(0x4B, 0x74, 0x49, true),  // Page Up
    k(0x4C, 0x75, 0x53, true),  // Forward Delete
    k(0x4D, 0x77, 0x4F, true),  // End
    k(0x4E, 0x79, 0x51, true),  // Page Down
    k(0x4F, 0x7C, 0x4D, true),  // Right
    k(0x50, 0x7B, 0x4B, true),  // Left
    k(0x51, 0x7D, 0x50, true),  // Down
    k(0x52, 0x7E, 0x48, true),  // Up
    // Keypad
    k(0x53, 0x47, 0x45, false), // Num Lock / Clear
    k(0x54, 0x4B, 0x35, true),  // KP /
    k(0x55, 0x43, 0x37, false), // KP *
    k(0x56, 0x4E, 0x4A, false), // KP -
    k(0x57, 0x45, 0x4E, false), // KP +
    k(0x58, 0x4C, 0x1C, true),  // KP Enter
    k(0x59, 0x53, 0x4F, false), // KP 1
    k(0x5A, 0x54, 0x50, false), // KP 2
    k(0x5B, 0x55, 0x51, false), // KP 3
    k(0x5C, 0x56, 0x4B, false), // KP 4
    k(0x5D, 0x57, 0x4C, false), // KP 5
    k(0x5E, 0x58, 0x4D, false), // KP 6
    k(0x5F, 0x59, 0x47, false), // KP 7
    k(0x60, 0x5B, 0x48, false), // KP 8
    k(0x61, 0x5C, 0x49, false), // KP 9
    k(0x62, 0x52, 0x52, false), // KP 0
    k(0x63, 0x41, 0x53, false), // KP .
    k(0x67, 0x51, 0x59, false), // KP =
    k(0x65, 0x6E, 0x5D, true),  // Menu / Application
    // Modifiers
    k(0xE0, 0x3B, 0x1D, false), // Left Ctrl
    k(0xE1, 0x38, 0x2A, false), // Left Shift
    k(0xE2, 0x3A, 0x38, false), // Left Alt / Option
    k(0xE3, 0x37, 0x5B, true),  // Left Win / Cmd
    k(0xE4, 0x3E, 0x1D, true),  // Right Ctrl
    k(0xE5, 0x3C, 0x36, false), // Right Shift
    k(0xE6, 0x3D, 0x38, true),  // Right Alt / Option
    k(0xE7, 0x36, 0x5C, true),  // Right Win / Cmd
];

pub const HID_LCTRL: u16 = 0xE0;
pub const HID_LSHIFT: u16 = 0xE1;
pub const HID_LALT: u16 = 0xE2;
pub const HID_LGUI: u16 = 0xE3;
pub const HID_RCTRL: u16 = 0xE4;
pub const HID_RSHIFT: u16 = 0xE5;
pub const HID_RALT: u16 = 0xE6;
pub const HID_RGUI: u16 = 0xE7;
pub const HID_ESC: u16 = 0x29;

#[allow(dead_code)]
pub fn from_mac(code: u16) -> Option<u16> {
    KEYS.iter().find(|x| x.mac == code).map(|x| x.hid)
}
#[allow(dead_code)]
pub fn to_mac(hid: u16) -> Option<u16> {
    KEYS.iter().find(|x| x.hid == hid).map(|x| x.mac)
}
#[allow(dead_code)]
pub fn from_win(scan: u16, ext: bool) -> Option<u16> {
    KEYS.iter().find(|x| x.win == scan && x.ext == ext).map(|x| x.hid)
}
#[allow(dead_code)]
pub fn to_win(hid: u16) -> Option<(u16, bool)> {
    KEYS.iter().find(|x| x.hid == hid).map(|x| (x.win, x.ext))
}

/// Linux evdev key code for a Windows set-1 scancode. The plain range is identical; E0 keys differ.
fn win_to_evdev(scan: u16, ext: bool) -> Option<u16> {
    if !ext {
        return Some(match scan {
            0x59 => 117, // KP =
            s => s,
        });
    }
    Some(match scan {
        0x1C => 96,  // KP Enter
        0x1D => 97,  // Right Ctrl
        0x35 => 98,  // KP /
        0x37 => 99,  // Print Screen
        0x38 => 100, // Right Alt
        0x47 => 102, // Home
        0x48 => 103, // Up
        0x49 => 104, // Page Up
        0x4B => 105, // Left
        0x4D => 106, // Right
        0x4F => 107, // End
        0x50 => 108, // Down
        0x51 => 109, // Page Down
        0x52 => 110, // Insert
        0x53 => 111, // Delete
        0x5B => 125, // Left Meta
        0x5C => 126, // Right Meta
        0x5D => 127, // Menu (KEY_COMPOSE)
        0x20 => 113, // Mute
        0x2E => 114, // Volume Down
        0x30 => 115, // Volume Up
        _ => return None,
    })
}

#[allow(dead_code)]
pub fn from_linux(code: u16) -> Option<u16> {
    KEYS.iter().find(|x| win_to_evdev(x.win, x.ext) == Some(code)).map(|x| x.hid)
}
#[allow(dead_code)]
pub fn to_linux(hid: u16) -> Option<u16> {
    KEYS.iter().find(|x| x.hid == hid).and_then(|x| win_to_evdev(x.win, x.ext))
}

/// Swap Cmd<->Ctrl (and Win<->Alt is *not* swapped) for macOS<->Windows shortcut parity.
pub fn swap_cmd_ctrl(hid: u16) -> u16 {
    match hid {
        HID_LCTRL => HID_LGUI,
        HID_LGUI => HID_LCTRL,
        HID_RCTRL => HID_RGUI,
        HID_RGUI => HID_RCTRL,
        x => x,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn linux_codes_round_trip() {
        let mut seen = HashSet::new();
        for k in KEYS {
            let code = to_linux(k.hid).unwrap_or_else(|| panic!("no Linux code for hid {:#x}", k.hid));
            assert!(seen.insert(code), "dup Linux code {code} (hid {:#x})", k.hid);
            assert_eq!(from_linux(code), Some(k.hid));
        }
        // spot checks against linux/input-event-codes.h
        assert_eq!(to_linux(0x04), Some(30)); // KEY_A
        assert_eq!(to_linux(0x52), Some(103)); // KEY_UP
        assert_eq!(to_linux(HID_LGUI), Some(125)); // KEY_LEFTMETA
        assert_eq!(to_linux(HID_RALT), Some(100)); // KEY_RIGHTALT
    }

    #[test]
    fn table_is_unambiguous() {
        let mut hid = HashSet::new();
        let mut mac = HashSet::new();
        let mut win = HashSet::new();
        for k in KEYS {
            assert!(hid.insert(k.hid), "dup hid {:#x}", k.hid);
            assert!(mac.insert(k.mac), "dup mac {:#x}", k.mac);
            assert!(win.insert((k.win, k.ext)), "dup win {:#x}/{}", k.win, k.ext);
        }
    }

    #[test]
    fn roundtrips() {
        for k in KEYS {
            assert_eq!(from_mac(to_mac(k.hid).unwrap()), Some(k.hid));
            let (s, e) = to_win(k.hid).unwrap();
            assert_eq!(from_win(s, e), Some(k.hid));
        }
        assert_eq!(swap_cmd_ctrl(swap_cmd_ctrl(HID_LCTRL)), HID_LCTRL);
    }
}
