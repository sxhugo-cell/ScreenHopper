use super::{CaptureCb, InputEvent, Permissions, Rect};
use crate::keymap;
use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_foundation::string::CFString;
use core_graphics::display::{CGDisplay, CGPoint};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CGMouseButton, EventField, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Tag put on events we inject, so our own tap lets them through.
const MAGIC: i64 = 0x5348_4F50; // "SHOP"
static TAP_PORT: AtomicUsize = AtomicUsize::new(0);

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: core_foundation::dictionary::CFDictionaryRef) -> bool;
}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapEnable(tap: *mut std::ffi::c_void, enable: bool);
    fn CGEventTapIsEnabled(tap: *mut std::ffi::c_void) -> bool;
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
    fn CGSetLocalEventsSuppressionInterval(seconds: f64) -> i32;
}

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}

pub fn secure_input() -> bool {
    unsafe { IsSecureEventInputEnabled() }
}

pub fn permissions() -> Permissions {
    unsafe {
        Permissions {
            accessibility: AXIsProcessTrusted(),
            input_monitoring: CGPreflightListenEventAccess(),
            required: true,
        }
    }
}

pub fn request_permissions() {
    unsafe {
        let key = CFString::new("AXTrustedCheckOptionPrompt");
        let dict = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFBoolean::true_value().as_CFType())]);
        AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef());
        CGRequestListenEventAccess();
        if !CGPreflightPostEventAccess() {
            CGRequestPostEventAccess();
        }
    }
}

pub fn monitors() -> Vec<Rect> {
    let mut v = Vec::new();
    if let Ok(ids) = CGDisplay::active_displays() {
        for id in ids {
            let b = CGDisplay::new(id).bounds();
            v.push(Rect { x: b.origin.x, y: b.origin.y, w: b.size.width, h: b.size.height });
        }
    }
    if v.is_empty() {
        v.push(Rect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 });
    }
    v
}

#[allow(dead_code)]
pub fn cursor_pos() -> (f64, f64) {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .ok()
        .and_then(|s| CGEvent::new(s).ok())
        .map(|e| {
            let p = e.location();
            (p.x, p.y)
        })
        .unwrap_or((0.0, 0.0))
}

pub fn warp_cursor(x: f64, y: f64) {
    log::info!("cursor moved by Screen Hopper to ({x:.0}, {y:.0})");
    unsafe {
        CGSetLocalEventsSuppressionInterval(0.0);
    }
    let _ = CGDisplay::warp_mouse_cursor_position(CGPoint::new(x, y));
    let _ = CGDisplay::associate_mouse_and_mouse_cursor_position(true);
}

fn is(a: CGEventType, b: CGEventType) -> bool {
    a as u32 == b as u32
}

fn flag_for_key(code: i64) -> Option<CGEventFlags> {
    match code {
        0x37 | 0x36 => Some(CGEventFlags::CGEventFlagCommand),
        0x38 | 0x3C => Some(CGEventFlags::CGEventFlagShift),
        0x3A | 0x3D => Some(CGEventFlags::CGEventFlagAlternate),
        0x3B | 0x3E => Some(CGEventFlags::CGEventFlagControl),
        0x39 => Some(CGEventFlags::CGEventFlagAlphaShift),
        _ => None,
    }
}

/// Start the global event tap on its own thread. Retries until permissions are granted.
pub fn start_capture(cb: CaptureCb) {
    // Watchdog: macOS can silently switch an event tap off; turn it back on.
    std::thread::Builder::new()
        .name("sh-tap-watchdog".into())
        .spawn(|| loop {
            std::thread::sleep(Duration::from_secs(2));
            let p = TAP_PORT.load(Ordering::SeqCst);
            if p != 0 {
                unsafe {
                    if !CGEventTapIsEnabled(p as *mut _) {
                        log::warn!("event tap was disabled; re-enabling");
                        CGEventTapEnable(p as *mut _, true);
                    }
                }
            }
        })
        .expect("spawn watchdog");
    std::thread::Builder::new()
        .name("sh-capture".into())
        .spawn(move || {
            let cb = std::sync::Arc::new(cb);
            loop {
                let cb2 = cb.clone();
                let tap = CGEventTap::new(
                    CGEventTapLocation::HID,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::Default,
                    vec![
                        CGEventType::LeftMouseDown,
                        CGEventType::LeftMouseUp,
                        CGEventType::RightMouseDown,
                        CGEventType::RightMouseUp,
                        CGEventType::MouseMoved,
                        CGEventType::LeftMouseDragged,
                        CGEventType::RightMouseDragged,
                        CGEventType::OtherMouseDown,
                        CGEventType::OtherMouseUp,
                        CGEventType::OtherMouseDragged,
                        CGEventType::ScrollWheel,
                        CGEventType::KeyDown,
                        CGEventType::KeyUp,
                        CGEventType::FlagsChanged,
                    ],
                    move |_proxy, etype, ev| {
                        if matches!(etype, CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput) {
                            let p = TAP_PORT.load(Ordering::SeqCst);
                            if p != 0 {
                                unsafe { CGEventTapEnable(p as *mut _, true) };
                            }
                            return None;
                        }
                        if ev.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == MAGIC {
                            return None;
                        }
                        let mut swallow = false;
                        match etype {
                            CGEventType::MouseMoved
                            | CGEventType::LeftMouseDragged
                            | CGEventType::RightMouseDragged
                            | CGEventType::OtherMouseDragged => {
                                let p = ev.location();
                                let dx = ev.get_integer_value_field(EventField::MOUSE_EVENT_DELTA_X) as f64;
                                let dy = ev.get_integer_value_field(EventField::MOUSE_EVENT_DELTA_Y) as f64;
                                swallow = cb2(InputEvent::Move { x: p.x, y: p.y, dx, dy });
                            }
                            CGEventType::LeftMouseDown | CGEventType::LeftMouseUp => {
                                swallow = cb2(InputEvent::Button { button: 0, down: is(etype, CGEventType::LeftMouseDown) });
                            }
                            CGEventType::RightMouseDown | CGEventType::RightMouseUp => {
                                swallow = cb2(InputEvent::Button { button: 1, down: is(etype, CGEventType::RightMouseDown) });
                            }
                            CGEventType::OtherMouseDown | CGEventType::OtherMouseUp => {
                                let b = ev.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER).clamp(2, 4) as u8;
                                swallow = cb2(InputEvent::Button { button: b, down: is(etype, CGEventType::OtherMouseDown) });
                            }
                            CGEventType::ScrollWheel => {
                                let dy = ev.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1);
                                let dx = ev.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2);
                                if dx != 0 || dy != 0 {
                                    swallow = cb2(InputEvent::Wheel { dx: (dx * 3) as i32, dy: (dy * 3) as i32 });
                                }
                            }
                            CGEventType::KeyDown | CGEventType::KeyUp => {
                                let code = ev.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
                                if let Some(hid) = keymap::from_mac(code) {
                                    swallow = cb2(InputEvent::Key { hid, down: is(etype, CGEventType::KeyDown) });
                                }
                            }
                            CGEventType::FlagsChanged => {
                                let code = ev.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
                                if let (Some(flag), Some(hid)) = (flag_for_key(code), keymap::from_mac(code as u16)) {
                                    if flag == CGEventFlags::CGEventFlagAlphaShift {
                                        swallow = cb2(InputEvent::Key { hid, down: true });
                                        let _ = cb2(InputEvent::Key { hid, down: false });
                                    } else {
                                        let down = ev.get_flags().contains(flag);
                                        swallow = cb2(InputEvent::Key { hid, down });
                                    }
                                }
                            }
                            _ => {}
                        }
                        if swallow {
                            ev.set_type(CGEventType::Null);
                        }
                        None
                    },
                );
                match tap {
                    Ok(tap) => {
                        TAP_PORT.store(tap.mach_port.as_concrete_TypeRef() as usize, Ordering::SeqCst);
                        if let Ok(src) = tap.mach_port.create_runloop_source(0) {
                            unsafe {
                                CFRunLoop::get_current().add_source(&src, kCFRunLoopCommonModes);
                            }
                            tap.enable();
                            log::info!("event tap running");
                            CFRunLoop::run_current();
                        }
                    }
                    Err(_) => {
                        log::warn!("event tap unavailable (grant Accessibility + Input Monitoring); retrying");
                    }
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        })
        .expect("spawn capture thread");
}

// ---------------------------------------------------------------------------------------------
// Injection
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct InjState {
    flags: u64,
    buttons: HashSet<u8>,
    keys: HashSet<u16>,
    last_click: Option<(Instant, u8, i64)>,
}

pub struct Injector {
    st: Mutex<InjState>,
}

fn source() -> Option<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState).ok()
}

fn tag(ev: &CGEvent) {
    ev.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, MAGIC);
}

impl Injector {
    pub fn new() -> Self {
        Injector { st: Mutex::new(InjState::default()) }
    }

    pub fn move_to(&self, x: f64, y: f64) {
        let st = self.st.lock();
        let (ty, btn) = if st.buttons.contains(&0) {
            (CGEventType::LeftMouseDragged, CGMouseButton::Left)
        } else if st.buttons.contains(&1) {
            (CGEventType::RightMouseDragged, CGMouseButton::Right)
        } else {
            (CGEventType::MouseMoved, CGMouseButton::Left)
        };
        if let Some(s) = source() {
            if let Ok(ev) = CGEvent::new_mouse_event(s, ty, CGPoint::new(x, y), btn) {
                tag(&ev);
                ev.set_flags(CGEventFlags::from_bits_retain(st.flags));
                ev.post(CGEventTapLocation::HID);
            }
        }
    }

    pub fn button(&self, x: f64, y: f64, button: u8, down: bool) {
        let mut st = self.st.lock();
        let (ty, btn) = match (button, down) {
            (0, true) => (CGEventType::LeftMouseDown, CGMouseButton::Left),
            (0, false) => (CGEventType::LeftMouseUp, CGMouseButton::Left),
            (1, true) => (CGEventType::RightMouseDown, CGMouseButton::Right),
            (1, false) => (CGEventType::RightMouseUp, CGMouseButton::Right),
            (_, true) => (CGEventType::OtherMouseDown, CGMouseButton::Center),
            (_, false) => (CGEventType::OtherMouseUp, CGMouseButton::Center),
        };
        let mut click_state = 1;
        if down {
            if let Some((t, b, c)) = st.last_click {
                if b == button && t.elapsed() < Duration::from_millis(450) {
                    click_state = c + 1;
                }
            }
            st.last_click = Some((Instant::now(), button, click_state));
            st.buttons.insert(button);
        } else {
            if let Some((_, _, c)) = st.last_click {
                click_state = c;
            }
            st.buttons.remove(&button);
        }
        if let Some(s) = source() {
            if let Ok(ev) = CGEvent::new_mouse_event(s, ty, CGPoint::new(x, y), btn) {
                tag(&ev);
                ev.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
                if button >= 2 {
                    ev.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, button as i64);
                }
                ev.set_flags(CGEventFlags::from_bits_retain(st.flags));
                ev.post(CGEventTapLocation::HID);
            }
        }
    }

    pub fn wheel(&self, dx: i32, dy: i32) {
        if let Some(s) = source() {
            if let Ok(ev) = CGEvent::new_scroll_event(s, ScrollEventUnit::PIXEL, 2, dy / 3, dx / 3, 0) {
                tag(&ev);
                ev.post(CGEventTapLocation::HID);
            }
        }
    }

    pub fn key(&self, hid: u16, down: bool) {
        let Some(code) = keymap::to_mac(hid) else { return };
        let mut st = self.st.lock();
        let bit = match hid {
            keymap::HID_LCTRL | keymap::HID_RCTRL => Some(CGEventFlags::CGEventFlagControl),
            keymap::HID_LSHIFT | keymap::HID_RSHIFT => Some(CGEventFlags::CGEventFlagShift),
            keymap::HID_LALT | keymap::HID_RALT => Some(CGEventFlags::CGEventFlagAlternate),
            keymap::HID_LGUI | keymap::HID_RGUI => Some(CGEventFlags::CGEventFlagCommand),
            _ => None,
        };
        if let Some(b) = bit {
            if down {
                st.flags |= b.bits();
            } else {
                st.flags &= !b.bits();
            }
        }
        if down {
            st.keys.insert(hid);
        } else {
            st.keys.remove(&hid);
        }
        if let Some(s) = source() {
            if let Ok(ev) = CGEvent::new_keyboard_event(s, code, down) {
                tag(&ev);
                ev.set_flags(CGEventFlags::from_bits_retain(st.flags));
                ev.post(CGEventTapLocation::HID);
            }
        }
    }

    /// Forget any modifier state left over from a previous visit.
    pub fn reset_modifiers(&self) {
        self.st.lock().flags = 0;
    }

    /// Lift every key and button we are holding down on behalf of a remote peer.
    /// True while a mouse button is held down on behalf of the controlling computer.
    pub fn buttons_held(&self) -> bool {
        !self.st.lock().buttons.is_empty()
    }

    pub fn release_all(&self, x: f64, y: f64) {
        let (keys, buttons): (Vec<u16>, Vec<u8>) = {
            let st = self.st.lock();
            (st.keys.iter().copied().collect(), st.buttons.iter().copied().collect())
        };
        for k in keys {
            self.key(k, false);
        }
        for b in buttons {
            self.button(x, y, b, false);
        }
    }
}

/// After every new install (version change) wipe this app's stale macOS privacy entries,
/// so Accessibility / Input Monitoring / Local Network are asked for again cleanly.
pub fn reset_privacy_on_update(dir: &std::path::Path, version: &str) {
    // Developer/test runs use their own config folder; never touch the installed app's permissions.
    if std::env::var_os("SCREEN_HOPPER_CONFIG_DIR").is_some() {
        return;
    }
    let f = dir.join("last-run-version");
    if std::fs::read_to_string(&f).map(|v| v.trim() == version).unwrap_or(false) {
        return;
    }
    let ok = std::process::Command::new("/usr/bin/tccutil")
        .args(["reset", "All", "com.screenhopper.app"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    log::info!("privacy permissions reset for new version {version}: {ok}");
    let _ = std::fs::write(f, version);
}

/// macOS never hides the keyboard from an approved event tap this way (Secure Input is reported separately).
pub fn foreground_blocks_keys() -> bool {
    false
}

/// Short system sound when the pause shortcut toggles.
pub fn play_toggle_sound(paused: bool) {
    let f = if paused { "/System/Library/Sounds/Tink.aiff" } else { "/System/Library/Sounds/Pop.aiff" };
    let _ = std::process::Command::new("/usr/bin/afplay").arg(f).spawn();
}

/// Windows only: keyboard focus while the cursor is on another computer.
pub fn on_away() {}
pub fn on_back() {}
