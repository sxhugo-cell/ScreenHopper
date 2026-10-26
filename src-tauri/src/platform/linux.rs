//! Linux (Wayland, GNOME/KDE). Two halves:
//! - Controlling other computers: the desktop's Input Capture portal. We put "barriers" on the outer screen edges;
//!   when the cursor pushes into one, the desktop hands us the mouse and keyboard (through libei) until we release.
//!   GNOME/KDE ask the user once for permission.
//! - Being controlled: a virtual mouse and keyboard through /dev/uinput (the .deb installs a udev rule that gives
//!   the logged-in user access). The mouse is an absolute pointer, like a VM tablet, so positions are exact.

use super::{CaptureCb, InputEvent, Permissions, Rect};
use crate::keymap;
use parking_lot::{Mutex, RwLock};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static MONS: RwLock<Vec<Rect>> = RwLock::new(Vec::new());
/// True once the Input Capture portal gave us screen zones; those then win over the toolkit's monitor list.
static ZONES_FROM_PORTAL: AtomicBool = AtomicBool::new(false);
static LAST_POS: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));
static CAPTURE_OK: AtomicBool = AtomicBool::new(false);
static UINPUT_OK: AtomicBool = AtomicBool::new(false);
static CMD: OnceLock<tokio::sync::mpsc::UnboundedSender<Cmd>> = OnceLock::new();

enum Cmd {
    Release(f64, f64),
}

pub fn secure_input() -> bool {
    false
}

pub fn permissions() -> Permissions {
    Permissions { accessibility: true, input_monitoring: true, required: false }
}

pub fn request_permissions() {}

/// Screens in desktop (logical) coordinates.
pub fn monitors() -> Vec<Rect> {
    let m = MONS.read().clone();
    if m.is_empty() {
        vec![Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 }]
    } else {
        m
    }
}

/// Monitor list from the toolkit (used until the Input Capture portal reports its zones).
pub fn set_monitors(m: Vec<Rect>) {
    if !m.is_empty() && !ZONES_FROM_PORTAL.load(Ordering::Relaxed) {
        *MONS.write() = m;
    }
}

/// Wayland does not let apps read the cursor; this is the last position we know of.
#[allow(dead_code)]
pub fn cursor_pos() -> (f64, f64) {
    *LAST_POS.lock()
}

/// Called when control comes back to this computer: end the capture and put the cursor at (x, y).
pub fn warp_cursor(x: f64, y: f64) {
    *LAST_POS.lock() = (x, y);
    if let Some(tx) = CMD.get() {
        let _ = tx.send(Cmd::Release(x, y));
    }
}

pub fn foreground_blocks_keys() -> bool {
    false
}
pub fn reset_privacy_on_update(_dir: &std::path::Path, _version: &str) {}
pub fn play_toggle_sound(_paused: bool) {}

// ------------------------------------------------------------------------------------ capture (portal + libei)

pub fn start_capture(cb: CaptureCb) {
    let cb: &'static CaptureCb = Box::leak(Box::new(cb));
    std::thread::Builder::new()
        .name("sh-capture".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    log::warn!("capture runtime: {e}");
                    return;
                }
            };
            rt.block_on(async move {
                if let Err(e) = run_capture(cb).await {
                    CAPTURE_OK.store(false, Ordering::Relaxed);
                    log::warn!(
                        "Input Capture is not available ({e}). This computer can be controlled from others, \
                         but cannot control them. It needs GNOME 46+ or KDE Plasma 6.1+ on Wayland."
                    );
                }
            });
        })
        .expect("spawn capture thread");
}

/// Which way an edge barrier points out of the screen.
#[derive(Clone, Copy)]
struct Out {
    dx: f64,
    dy: f64,
}

async fn run_capture(cb: &'static CaptureCb) -> Result<(), String> {
    use ashpd::desktop::input_capture::{
        ActivatedBarrier, Barrier, BarrierID, BarrierPosition, Capabilities, CreateSessionOptions, InputCapture, ReleaseOptions,
    };
    use futures_util::StreamExt;
    use reis::ei;
    use reis::event::{DeviceCapability, EiEvent};

    let e = |x: ashpd::Error| x.to_string();
    let ic = InputCapture::new().await.map_err(e)?;
    let (session, _caps) = ic
        .create_session(None, CreateSessionOptions::default().set_capabilities(Capabilities::Keyboard | Capabilities::Pointer))
        .await
        .map_err(e)?;
    let fd = ic.connect_to_eis(&session, Default::default()).await.map_err(e)?;
    let stream = std::os::unix::net::UnixStream::from(fd);
    stream.set_nonblocking(true).map_err(|x| x.to_string())?;
    let context = ei::Context::new(stream).map_err(|x| x.to_string())?;
    let _ = context.flush();
    let (_conn, mut events) = context
        .handshake_tokio("Screen Hopper", ei::handshake::ContextType::Receiver)
        .await
        .map_err(|x| format!("libei handshake: {x}"))?;

    // Screens, then a barrier on every edge. Edges between two screens are refused by the desktop; that's fine.
    let zones = ic.zones(&session, Default::default()).await.map_err(e)?.response().map_err(e)?;
    let rects: Vec<Rect> = zones
        .regions()
        .iter()
        .map(|r| Rect { x: r.x_offset() as f64, y: r.y_offset() as f64, w: r.width() as f64, h: r.height() as f64 })
        .collect();
    if !rects.is_empty() {
        *MONS.write() = rects.clone();
        ZONES_FROM_PORTAL.store(true, Ordering::Relaxed);
    }
    let mut barriers = Vec::new();
    let mut dirs: Vec<Out> = Vec::new();
    for r in zones.regions() {
        let (x, y, w, h) = (r.x_offset(), r.y_offset(), r.width() as i32, r.height() as i32);
        for (pos, out) in [
            (BarrierPosition::new(x, y, x, y + h - 1), Out { dx: -1.0, dy: 0.0 }),
            (BarrierPosition::new(x + w, y, x + w, y + h - 1), Out { dx: 1.0, dy: 0.0 }),
            (BarrierPosition::new(x, y, x + w - 1, y), Out { dx: 0.0, dy: -1.0 }),
            (BarrierPosition::new(x, y + h, x + w - 1, y + h), Out { dx: 0.0, dy: 1.0 }),
        ] {
            dirs.push(out);
            let id = BarrierID::new(dirs.len() as u32).expect("non-zero");
            barriers.push(Barrier::new(id, pos));
        }
    }
    let resp = ic
        .set_pointer_barriers(&session, &barriers, zones.zone_set(), Default::default())
        .await
        .map_err(e)?
        .response()
        .map_err(e)?;
    log::info!("input capture: {} screens, {} of {} edge barriers accepted", rects.len(), barriers.len() - resp.failed_barriers().len(), barriers.len());
    ic.enable(&session, Default::default()).await.map_err(e)?;
    let mut activated = ic.receive_activated().await.map_err(e)?;
    let mut deactivated = ic.receive_deactivated().await.map_err(e)?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Cmd>();
    let _ = CMD.set(tx);
    CAPTURE_OK.store(true, Ordering::Relaxed);
    log::info!("input capture ready: push the cursor past a screen edge to hop");

    let mut capturing = false;
    let mut activation: Option<u32> = None;
    let mut entry = (0.0f64, 0.0f64, Out { dx: 0.0, dy: 0.0 });
    let mut last_discrete = std::time::Instant::now() - std::time::Duration::from_secs(1);

    loop {
        tokio::select! {
            a = activated.next() => {
                let Some(a) = a else { return Err("portal closed".into()) };
                let (x, y) = a.cursor_position().map(|(x, y)| (x as f64, y as f64)).unwrap_or(*LAST_POS.lock());
                let out = match a.barrier_id() {
                    Some(ActivatedBarrier::Barrier(id)) => dirs.get(id.get() as usize - 1).copied().unwrap_or(Out { dx: 0.0, dy: 0.0 }),
                    _ => Out { dx: 0.0, dy: 0.0 },
                };
                activation = a.activation_id();
                *LAST_POS.lock() = (x, y);
                entry = (x, y, out);
                // The cursor touches the edge from inside; ask the core whether a computer is out there.
                let (ex, ey) = (x - out.dx.max(0.0), y - out.dy.max(0.0));
                let hop = out.dx != 0.0 || out.dy != 0.0;
                let taken = hop && cb(InputEvent::Move { x: ex, y: ey, dx: out.dx, dy: out.dy });
                if taken {
                    capturing = true;
                } else {
                    // Nobody on that side: give the cursor straight back.
                    let back = (ex - out.dx, ey - out.dy);
                    let _ = ic.release(&session, ReleaseOptions::default().set_activation_id(activation).set_cursor_position(back)).await;
                }
            }
            d = deactivated.next() => {
                if d.is_none() { return Err("portal closed".into()); }
                capturing = false;
            }
            c = rx.recv() => {
                let Some(Cmd::Release(x, y)) = c else { continue };
                if capturing {
                    capturing = false;
                    // Put the cursor just inside the screen it re-enters, at the point the core chose.
                    let _ = ic.release(&session, ReleaseOptions::default().set_activation_id(activation).set_cursor_position((x, y))).await;
                }
            }
            ev = events.next() => {
                let Some(ev) = ev else { return Err("libei connection closed".into()) };
                let ev = match ev {
                    Ok(ev) => ev,
                    Err(err) => { log::warn!("libei: {err}"); continue; }
                };
                match ev {
                    EiEvent::SeatAdded(s) => {
                        s.seat.bind_capabilities(DeviceCapability::Pointer | DeviceCapability::Keyboard | DeviceCapability::Scroll | DeviceCapability::Button);
                        let _ = context.flush();
                    }
                    EiEvent::Disconnected(_) => return Err("libei disconnected".into()),
                    _ if !capturing => {}
                    EiEvent::PointerMotion(m) => {
                        cb(InputEvent::Move { x: entry.0, y: entry.1, dx: m.dx as f64, dy: m.dy as f64 });
                    }
                    EiEvent::Button(b) => {
                        let button = match b.button { 0x110 => 0, 0x111 => 1, 0x112 => 2, 0x113 => 3, 0x114 => 4, _ => continue };
                        cb(InputEvent::Button { button, down: b.state == ei::button::ButtonState::Press });
                    }
                    EiEvent::ScrollDiscrete(s) => {
                        last_discrete = std::time::Instant::now();
                        cb(InputEvent::Wheel { dx: -s.discrete_dx, dy: -s.discrete_dy });
                    }
                    EiEvent::ScrollDelta(s) => {
                        // Touchpads send smooth scrolling only; wheels send both, so skip the smooth copy then.
                        if last_discrete.elapsed().as_millis() > 200 {
                            cb(InputEvent::Wheel { dx: (-s.dx * 8.0) as i32, dy: (-s.dy * 8.0) as i32 });
                        }
                    }
                    EiEvent::KeyboardKey(k) => {
                        if let Some(hid) = keymap::from_linux(k.key as u16) {
                            cb(InputEvent::Key { hid, down: k.state == ei::keyboard::KeyState::Press });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------ injection (uinput)

const ABS_MAX: i32 = 32767;

struct Devs {
    pointer: evdev::uinput::VirtualDevice,
    keyboard: evdev::uinput::VirtualDevice,
}

#[derive(Default)]
struct InjState {
    buttons: HashSet<u8>,
    keys: HashSet<u16>,
    wheel_acc: (i32, i32),
}

pub struct Injector {
    devs: Mutex<Option<Devs>>,
    tried: Mutex<Option<std::time::Instant>>,
    st: Mutex<InjState>,
}

fn make_devices() -> std::io::Result<Devs> {
    use evdev::{uinput::VirtualDevice, AbsInfo, AbsoluteAxisCode, AttributeSet, KeyCode, RelativeAxisCode, UinputAbsSetup};
    let mut btns = AttributeSet::<KeyCode>::new();
    for b in [KeyCode::BTN_LEFT, KeyCode::BTN_RIGHT, KeyCode::BTN_MIDDLE, KeyCode::BTN_SIDE, KeyCode::BTN_EXTRA] {
        btns.insert(b);
    }
    let abs = |c| UinputAbsSetup::new(c, AbsInfo::new(0, 0, ABS_MAX, 0, 0, 0));
    let pointer = VirtualDevice::builder()?
        .name("Screen Hopper pointer")
        .with_keys(&btns)?
        .with_absolute_axis(&abs(AbsoluteAxisCode::ABS_X))?
        .with_absolute_axis(&abs(AbsoluteAxisCode::ABS_Y))?
        .with_relative_axes(&AttributeSet::from_iter([
            RelativeAxisCode::REL_WHEEL,
            RelativeAxisCode::REL_HWHEEL,
            RelativeAxisCode::REL_WHEEL_HI_RES,
            RelativeAxisCode::REL_HWHEEL_HI_RES,
        ]))?
        .build()?;
    let mut keys = AttributeSet::<KeyCode>::new();
    for k in keymap::KEYS {
        if let Some(c) = keymap::to_linux(k.hid) {
            keys.insert(KeyCode::new(c));
        }
    }
    let keyboard = VirtualDevice::builder()?.name("Screen Hopper keyboard").with_keys(&keys)?.build()?;
    Ok(Devs { pointer, keyboard })
}

impl Injector {
    pub fn new() -> Self {
        let inj = Injector { devs: Mutex::new(None), tried: Mutex::new(None), st: Mutex::new(InjState::default()) };
        inj.ensure();
        inj
    }

    /// Create the virtual devices (again, at most every 5 s if /dev/uinput was not accessible).
    fn ensure(&self) -> bool {
        if self.devs.lock().is_some() {
            return true;
        }
        let mut t = self.tried.lock();
        if t.is_some_and(|t| t.elapsed().as_secs() < 5) {
            return false;
        }
        *t = Some(std::time::Instant::now());
        match make_devices() {
            Ok(d) => {
                *self.devs.lock() = Some(d);
                UINPUT_OK.store(true, Ordering::Relaxed);
                log::info!("virtual mouse and keyboard ready (/dev/uinput)");
                true
            }
            Err(e) => {
                log::warn!(
                    "cannot create the virtual mouse/keyboard: {e}. Other computers cannot control this one until \
                     /dev/uinput is accessible: reinstall the .deb, or run 'sudo modprobe uinput' and log out and in."
                );
                false
            }
        }
    }

    fn emit(&self, keyboard: bool, evs: &[evdev::InputEvent]) {
        if !self.ensure() {
            return;
        }
        let mut g = self.devs.lock();
        let Some(d) = g.as_mut() else { return };
        let dev = if keyboard { &mut d.keyboard } else { &mut d.pointer };
        if let Err(e) = dev.emit(evs) {
            log::warn!("uinput: {e}");
        }
    }

    pub fn move_to(&self, x: f64, y: f64) {
        use evdev::{AbsoluteAxisCode, EventType, InputEvent};
        *LAST_POS.lock() = (x, y);
        let u = super::union_rect(&monitors());
        let nx = (((x - u.x) / u.w.max(1.0)) * ABS_MAX as f64).round().clamp(0.0, ABS_MAX as f64) as i32;
        let ny = (((y - u.y) / u.h.max(1.0)) * ABS_MAX as f64).round().clamp(0.0, ABS_MAX as f64) as i32;
        self.emit(false, &[
            InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_X.0, nx),
            InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_Y.0, ny),
        ]);
    }

    pub fn button(&self, _x: f64, _y: f64, button: u8, down: bool) {
        use evdev::{EventType, InputEvent, KeyCode};
        {
            let mut st = self.st.lock();
            if down {
                st.buttons.insert(button);
            } else {
                st.buttons.remove(&button);
            }
        }
        let code = match button {
            0 => KeyCode::BTN_LEFT,
            1 => KeyCode::BTN_RIGHT,
            2 => KeyCode::BTN_MIDDLE,
            3 => KeyCode::BTN_SIDE,
            _ => KeyCode::BTN_EXTRA,
        };
        self.emit(false, &[InputEvent::new(EventType::KEY.0, code.0, down as i32)]);
    }

    /// dx/dy in 1/120-notch units; positive dy scrolls up (away from the user).
    pub fn wheel(&self, dx: i32, dy: i32) {
        use evdev::{EventType, InputEvent, RelativeAxisCode as R};
        let mut evs = Vec::new();
        {
            let mut st = self.st.lock();
            if dy != 0 {
                evs.push(InputEvent::new(EventType::RELATIVE.0, R::REL_WHEEL_HI_RES.0, dy));
                st.wheel_acc.1 += dy;
                let notches = st.wheel_acc.1 / 120;
                if notches != 0 {
                    st.wheel_acc.1 -= notches * 120;
                    evs.push(InputEvent::new(EventType::RELATIVE.0, R::REL_WHEEL.0, notches));
                }
            }
            if dx != 0 {
                evs.push(InputEvent::new(EventType::RELATIVE.0, R::REL_HWHEEL_HI_RES.0, dx));
                st.wheel_acc.0 += dx;
                let notches = st.wheel_acc.0 / 120;
                if notches != 0 {
                    st.wheel_acc.0 -= notches * 120;
                    evs.push(InputEvent::new(EventType::RELATIVE.0, R::REL_HWHEEL.0, notches));
                }
            }
        }
        if !evs.is_empty() {
            self.emit(false, &evs);
        }
    }

    pub fn key(&self, hid: u16, down: bool) {
        use evdev::{EventType, InputEvent};
        let Some(code) = keymap::to_linux(hid) else { return };
        {
            let mut st = self.st.lock();
            if down {
                st.keys.insert(hid);
            } else {
                st.keys.remove(&hid);
            }
        }
        self.emit(true, &[InputEvent::new(EventType::KEY.0, code, down as i32)]);
    }

    pub fn reset_modifiers(&self) {}

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

// ------------------------------------------------------------------------------------ not on Linux

pub fn on_away() {}
pub fn on_back() {}

/// Short status for the log/UI: can this computer control others, and can it be controlled?
#[allow(dead_code)]
pub fn status() -> (bool, bool) {
    (CAPTURE_OK.load(Ordering::Relaxed), UINPUT_OK.load(Ordering::Relaxed))
}
