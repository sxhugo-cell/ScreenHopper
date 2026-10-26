//! OS abstraction: global input capture, input injection, screen geometry, permissions.

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Copy, Debug)]
pub enum InputEvent {
    /// Absolute cursor position plus movement delta since the previous event.
    Move { x: f64, y: f64, dx: f64, dy: f64 },
    /// 0 = left, 1 = right, 2 = middle, 3 = back, 4 = forward.
    Button { button: u8, down: bool },
    /// 1/120-notch units.
    Wheel { dx: i32, dy: i32 },
    Key { hid: u16, down: bool },
}

/// Return `true` to swallow the event (it never reaches the local OS).
pub type CaptureCb = Box<dyn Fn(InputEvent) -> bool + Send + Sync>;

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Permissions {
    pub accessibility: bool,
    pub input_monitoring: bool,
    /// Windows needs none; macOS needs both.
    pub required: bool,
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "windows")]
mod windows_impl;
#[cfg(target_os = "windows")]
pub use windows_impl::*;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub use android::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "android", target_os = "linux")))]
compile_error!("Screen Hopper supports macOS, Windows, Linux and Android only");

pub fn os_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "android") {
        "Android"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else {
        "Windows"
    }
}

/// Union of all monitors, in raw OS coordinates.
pub fn union_rect(ms: &[Rect]) -> Rect {
    let mut it = ms.iter();
    let Some(f) = it.next() else { return Rect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 } };
    let (mut x0, mut y0, mut x1, mut y1) = (f.x, f.y, f.x + f.w, f.y + f.h);
    for m in it {
        x0 = x0.min(m.x);
        y0 = y0.min(m.y);
        x1 = x1.max(m.x + m.w);
        y1 = y1.max(m.y + m.h);
    }
    Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

#[allow(dead_code)]
pub fn desktop_rect() -> Rect {
    union_rect(&monitors())
}
