use super::{CaptureCb, InputEvent, Permissions, Rect};
use crate::keymap;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::OnceLock;
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

static CB: OnceLock<CaptureCb> = OnceLock::new();

struct CapState {
    last: Option<(i32, i32)>,
    /// True while the previous move was swallowed; deltas are then measured from screen centre.
    swallowing: bool,
}
static CAP: Mutex<CapState> = Mutex::new(CapState { last: None, swallowing: false });

pub fn secure_input() -> bool {
    false
}

pub fn permissions() -> Permissions {
    Permissions { accessibility: true, input_monitoring: true, required: false }
}
pub fn request_permissions() {}

unsafe extern "system" fn enum_cb(h: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
    let v = &mut *(lp.0 as *mut Vec<Rect>);
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if GetMonitorInfoW(h, &mut mi).as_bool() {
        let r = mi.rcMonitor;
        v.push(Rect { x: r.left as f64, y: r.top as f64, w: (r.right - r.left) as f64, h: (r.bottom - r.top) as f64 });
    }
    BOOL(1)
}

pub fn monitors() -> Vec<Rect> {
    let mut v: Vec<Rect> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(enum_cb), LPARAM(&mut v as *mut Vec<Rect> as isize));
    }
    if v.is_empty() {
        v.push(Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 });
    }
    v
}

#[allow(dead_code)]
pub fn cursor_pos() -> (f64, f64) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x as f64, p.y as f64)
}

pub fn warp_cursor(x: f64, y: f64) {
    unsafe {
        let _ = SetCursorPos(x as i32, y as i32);
    }
    let mut c = CAP.lock();
    c.last = Some((x as i32, y as i32));
    c.swallowing = false;
}

fn centre() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXSCREEN) / 2, GetSystemMetrics(SM_CYSCREEN) / 2) }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
    if info.flags & 0x1 != 0 {
        // injected (ours or another tool's): never touch
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let Some(cb) = CB.get() else {
        return CallNextHookEx(None, code, wparam, lparam);
    };
    let msg = wparam.0 as u32;
    let hi = (info.mouseData >> 16) as i16;
    let swallow = match msg {
        WM_MOUSEMOVE => {
            let (cx, cy) = centre();
            let (dx, dy, was_swallowing) = {
                let c = CAP.lock();
                if c.swallowing {
                    (info.pt.x - cx, info.pt.y - cy, true)
                } else {
                    let (lx, ly) = c.last.unwrap_or((info.pt.x, info.pt.y));
                    (info.pt.x - lx, info.pt.y - ly, false)
                }
            };
            if was_swallowing && dx == 0 && dy == 0 {
                true
            } else {
                let sw = cb(InputEvent::Move { x: info.pt.x as f64, y: info.pt.y as f64, dx: dx as f64, dy: dy as f64 });
                {
                    let mut c = CAP.lock();
                    c.swallowing = sw;
                    if !sw {
                        c.last = Some((info.pt.x, info.pt.y));
                    }
                }
                if sw {
                    let _ = SetCursorPos(cx, cy);
                }
                sw
            }
        }
        WM_LBUTTONDOWN => cb(InputEvent::Button { button: 0, down: true }),
        WM_LBUTTONUP => cb(InputEvent::Button { button: 0, down: false }),
        WM_RBUTTONDOWN => cb(InputEvent::Button { button: 1, down: true }),
        WM_RBUTTONUP => cb(InputEvent::Button { button: 1, down: false }),
        WM_MBUTTONDOWN => cb(InputEvent::Button { button: 2, down: true }),
        WM_MBUTTONUP => cb(InputEvent::Button { button: 2, down: false }),
        WM_XBUTTONDOWN => cb(InputEvent::Button { button: if hi == 1 { 3 } else { 4 }, down: true }),
        WM_XBUTTONUP => cb(InputEvent::Button { button: if hi == 1 { 3 } else { 4 }, down: false }),
        WM_MOUSEWHEEL => cb(InputEvent::Wheel { dx: 0, dy: hi as i32 }),
        WM_MOUSEHWHEEL => cb(InputEvent::Wheel { dx: hi as i32, dy: 0 }),
        _ => false,
    };
    if swallow {
        LRESULT(1)
    } else {
        CallNextHookEx(None, code, wparam, lparam)
    }
}

unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    KEY_HOOK_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if info.flags.0 & 0x10 != 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let Some(cb) = CB.get() else {
        return CallNextHookEx(None, code, wparam, lparam);
    };
    let msg = wparam.0 as u32;
    let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let ext = info.flags.0 & 0x1 != 0;
    let mut swallow = false;
    if let Some(hid) = keymap::from_win(info.scanCode as u16, ext) {
        swallow = cb(InputEvent::Key { hid, down });
    }
    if swallow {
        LRESULT(1)
    } else {
        CallNextHookEx(None, code, wparam, lparam)
    }
}

pub fn start_capture(cb: CaptureCb) {
    let _ = CB.set(cb);
    std::thread::Builder::new()
        .name("sh-capture".into())
        .spawn(|| unsafe {
            let hmod = GetModuleHandleW(None).map(|m| HINSTANCE(m.0)).unwrap_or_default();
            let install = || {
                (
                    SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod, 0).ok(),
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), hmod, 0).ok(),
                )
            };
            let (mut m, mut k) = install();
            log::info!("low-level hooks installed");
            CAPTURE_TID.store(windows::Win32::System::Threading::GetCurrentThreadId(), std::sync::atomic::Ordering::Relaxed);
            let sink = make_focus_sink(hmod);
            let mut prev_fg = HWND::default();
            // Windows silently drops a low-level hook if it ever answers too slowly, so
            // re-install both every few seconds to self-heal.
            SetTimer(None, 1, 5000, None);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_APP_FOCUS_TAKE {
                    prev_fg = take_focus(sink);
                    continue;
                }
                if msg.message == WM_APP_FOCUS_GIVE {
                    give_focus(sink, prev_fg);
                    prev_fg = HWND::default();
                    continue;
                }
                if msg.message == WM_TIMER {
                    if let Some(h) = m.take() {
                        let _ = UnhookWindowsHookEx(h);
                    }
                    if let Some(h) = k.take() {
                        let _ = UnhookWindowsHookEx(h);
                    }
                    let (nm, nk) = install();
                    m = nm;
                    k = nk;
                    continue;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .expect("spawn capture thread");
}

// ---------------------------------------------------------------------------------------------
// Injection
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct InjState {
    buttons: HashSet<u8>,
    keys: HashSet<u16>,
}

pub struct Injector {
    st: Mutex<InjState>,
}

fn send(input: INPUT) {
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn mouse_input(flags: MOUSE_EVENT_FLAGS, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

impl Injector {
    pub fn new() -> Self {
        Injector { st: Mutex::new(InjState::default()) }
    }

    /// Absolute move via SendInput, so Windows treats it as real mouse input
    /// (wakes the display, updates hover and keyboard-focus tracking).
    pub fn move_to(&self, x: f64, y: f64) {
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN) as f64;
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN) as f64;
            let vw = (GetSystemMetrics(SM_CXVIRTUALSCREEN) as f64 - 1.0).max(1.0);
            let vh = (GetSystemMetrics(SM_CYVIRTUALSCREEN) as f64 - 1.0).max(1.0);
            let nx = (((x - vx) * 65535.0) / vw).round().clamp(0.0, 65535.0) as i32;
            let ny = (((y - vy) * 65535.0) / vh).round().clamp(0.0, 65535.0) as i32;
            let mut inp = mouse_input(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK, 0);
            inp.Anonymous.mi.dx = nx;
            inp.Anonymous.mi.dy = ny;
            send(inp);
        }
    }

    pub fn button(&self, _x: f64, _y: f64, button: u8, down: bool) {
        let mut st = self.st.lock();
        if down {
            st.buttons.insert(button);
        } else {
            st.buttons.remove(&button);
        }
        let (flags, data) = match (button, down) {
            (0, true) => (MOUSEEVENTF_LEFTDOWN, 0),
            (0, false) => (MOUSEEVENTF_LEFTUP, 0),
            (1, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
            (1, false) => (MOUSEEVENTF_RIGHTUP, 0),
            (2, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
            (2, false) => (MOUSEEVENTF_MIDDLEUP, 0),
            (3, true) => (MOUSEEVENTF_XDOWN, 1),
            (3, false) => (MOUSEEVENTF_XUP, 1),
            (_, true) => (MOUSEEVENTF_XDOWN, 2),
            (_, false) => (MOUSEEVENTF_XUP, 2),
        };
        send(mouse_input(flags, data));
    }

    pub fn wheel(&self, dx: i32, dy: i32) {
        if dy != 0 {
            send(mouse_input(MOUSEEVENTF_WHEEL, dy));
        }
        if dx != 0 {
            send(mouse_input(MOUSEEVENTF_HWHEEL, dx));
        }
    }

    pub fn key(&self, hid: u16, down: bool) {
        let Some((scan, ext)) = keymap::to_win(hid) else { return };
        {
            let mut st = self.st.lock();
            if down {
                st.keys.insert(hid);
            } else {
                st.keys.remove(&hid);
            }
        }
        let mut flags = KEYEVENTF_SCANCODE;
        if ext {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if !down {
            flags |= KEYEVENTF_KEYUP;
        }
        send(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        });
    }

    pub fn reset_modifiers(&self) {}

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

pub fn reset_privacy_on_update(_dir: &std::path::Path, _version: &str) {}

// ---------------------------------------------------------------------------------------------
// Keyboard access check (UIPI): a normal app's keyboard hook can't see keys sent to an
// administrator (elevated) window, which is the classic "type on Windows first" symptom.
// ---------------------------------------------------------------------------------------------

fn process_elevated(pid: u32) -> Option<bool> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut tok = HANDLE::default();
        let opened = OpenProcessToken(h, TOKEN_QUERY, &mut tok).is_ok();
        let mut result = None;
        if opened {
            let mut e = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            if GetTokenInformation(tok, TokenElevation, Some(&mut e as *mut _ as *mut std::ffi::c_void), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len).is_ok() {
                result = Some(e.TokenIsElevated != 0);
            }
            let _ = CloseHandle(tok);
        } else {
            // Not even allowed to read its token: it runs at a higher level than us.
            result = Some(true);
        }
        let _ = CloseHandle(h);
        result
    }
}

/// True when the window in front belongs to an elevated process and we are not elevated.
pub fn foreground_blocks_keys() -> bool {
    use windows::Win32::System::Threading::GetCurrentProcessId;
    unsafe {
        let w = GetForegroundWindow();
        if w.0.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(w, Some(&mut pid));
        if pid == 0 || pid == GetCurrentProcessId() {
            return false;
        }
        let me = process_elevated(GetCurrentProcessId()).unwrap_or(false);
        !me && process_elevated(pid).unwrap_or(false)
    }
}

/// Short system sound when the pause shortcut toggles.
pub fn play_toggle_sound(paused: bool) {
    unsafe {
        let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(if paused { MB_ICONASTERISK } else { MB_OK });
    }
}

// ---------------------------------------------------------------------------------------------
// Keyboard focus while another computer is in control.
// Windows only shows a keyboard hook the keys that the window in front allows it to see, so
// right after a hop the keys could vanish until something on Windows changed the foreground.
// While the cursor is away we put our own invisible window in front (as Synergy/Barrier do);
// when it comes back, the window you were using gets the focus again.
// ---------------------------------------------------------------------------------------------

static KEY_HOOK_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CAPTURE_TID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static AWAY_START_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SINK_KEYS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static AWAY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const WM_APP_FOCUS_TAKE: u32 = WM_APP + 41;
const WM_APP_FOCUS_GIVE: u32 = WM_APP + 42;

/// While the cursor is away the focus window gets every key the hook did not catch; pass those on too.
unsafe extern "system" fn sink_proc(h: windows::Win32::Foundation::HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    use std::sync::atomic::Ordering;
    let is_key = matches!(msg, WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP);
    if is_key && AWAY.load(Ordering::Relaxed) {
        let scan = ((lp.0 >> 16) & 0xff) as u16;
        let ext = (lp.0 >> 24) & 1 != 0;
        let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        if let (Some(hid), Some(cb)) = (keymap::from_win(scan, ext), CB.get()) {
            SINK_KEYS.fetch_add(1, Ordering::Relaxed);
            cb(InputEvent::Key { hid, down });
        }
        return LRESULT(0);
    }
    if AWAY.load(Ordering::Relaxed) && matches!(msg, WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR) {
        return LRESULT(0);
    }
    DefWindowProcW(h, msg, wp, lp)
}

unsafe fn make_focus_sink(hmod: HINSTANCE) -> windows::Win32::Foundation::HWND {
    use windows::core::w;
    let wc = WNDCLASSW { lpfnWndProc: Some(sink_proc), hInstance: hmod, lpszClassName: w!("ScreenHopperKeyboardFocus"), ..Default::default() };
    RegisterClassW(&wc);
    let h = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TOPMOST,
        w!("ScreenHopperKeyboardFocus"),
        w!("Screen Hopper keyboard"),
        WS_POPUP,
        0, 0, 1, 1,
        None, None, hmod, None,
    )
    .unwrap_or_default();
    if !h.0.is_null() {
        let _ = SetLayeredWindowAttributes(h, windows::Win32::Foundation::COLORREF(0), 1, LWA_ALPHA);
    }
    h
}

/// Who is in front right now: title, program, and whether Windows lets a normal app see its keys.
fn describe_foreground() -> String {
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let w = GetForegroundWindow();
        if w.0.is_null() {
            return "nothing in front".into();
        }
        let mut t = [0u16; 128];
        let n = GetWindowTextW(w, &mut t);
        let title = String::from_utf16_lossy(&t[..n.max(0) as usize]);
        let mut pid = 0u32;
        GetWindowThreadProcessId(w, Some(&mut pid));
        let mut exe = String::from("?");
        if let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut b = [0u16; 260];
            let mut len = b.len() as u32;
            if QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, windows::core::PWSTR(b.as_mut_ptr()), &mut len).is_ok() {
                let full = String::from_utf16_lossy(&b[..len as usize]);
                exe = full.rsplit('\\').next().unwrap_or(&full).to_string();
            }
            let _ = windows::Win32::Foundation::CloseHandle(p);
        }
        let elevated = process_elevated(pid).map(|e| if e { "admin" } else { "normal" }).unwrap_or("unknown");
        format!("\"{title}\" ({exe}, {elevated})")
    }
}

unsafe fn take_focus(sink: windows::Win32::Foundation::HWND) -> windows::Win32::Foundation::HWND {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    let before = GetForegroundWindow();
    let who = describe_foreground();
    if sink.0.is_null() {
        log::info!("hop: in front on Windows: {who}; no focus window available");
        return before;
    }
    let _ = ShowWindow(sink, SW_SHOWNA);
    let me = GetCurrentThreadId();
    let fg_thread = if before.0.is_null() { 0 } else { GetWindowThreadProcessId(before, None) };
    let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
    let _ = BringWindowToTop(sink);
    let mut ok = SetForegroundWindow(sink).as_bool();
    if attached {
        let _ = AttachThreadInput(me, fg_thread, false);
    }
    if !ok {
        // Second try: a tap of Alt lets this process take the foreground. Our hook ignores injected keys.
        keybd_event(VK_MENU.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
        ok = SetForegroundWindow(sink).as_bool();
        keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
    AWAY_START_CALLS.store(KEY_HOOK_CALLS.load(std::sync::atomic::Ordering::Relaxed), std::sync::atomic::Ordering::Relaxed);
    SINK_KEYS.store(0, std::sync::atomic::Ordering::Relaxed);
    AWAY.store(true, std::sync::atomic::Ordering::Relaxed);
    log::info!("hop: in front on Windows was {who}; keyboard focus taken: {ok}");
    before
}

unsafe fn give_focus(sink: windows::Win32::Foundation::HWND, prev: windows::Win32::Foundation::HWND) {
    use std::sync::atomic::Ordering;
    AWAY.store(false, Ordering::Relaxed);
    let total = KEY_HOOK_CALLS.load(Ordering::Relaxed);
    let seen = total - AWAY_START_CALLS.load(Ordering::Relaxed);
    let sunk = SINK_KEYS.load(Ordering::Relaxed);
    log::info!("back: while away the keyboard hook saw {seen} key events (total since start {total}); the focus window passed on {sunk}");
    if sink.0.is_null() {
        return;
    }
    // Only hand focus back if nobody else took it meanwhile.
    if GetForegroundWindow() == sink && !prev.0.is_null() && IsWindow(prev).as_bool() {
        let _ = SetForegroundWindow(prev);
    }
    let _ = ShowWindow(sink, SW_HIDE);
}

/// The cursor just left this computer for another one.
pub fn on_away() {
    let tid = CAPTURE_TID.load(std::sync::atomic::Ordering::Relaxed);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_APP_FOCUS_TAKE, WPARAM(0), LPARAM(0));
        }
    }
}

/// The cursor is back on this computer.
pub fn on_back() {
    let tid = CAPTURE_TID.load(std::sync::atomic::Ordering::Relaxed);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_APP_FOCUS_GIVE, WPARAM(0), LPARAM(0));
        }
    }
}
