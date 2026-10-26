//! Shared application core: settings, sessions, and the input-routing state machine.
use crate::config::{self, PeerLayout, Settings};
use crate::keymap;
use crate::geom;
use crate::platform::{self, InputEvent, Injector, Permissions, Rect};
use crate::protocol::Msg;
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

pub struct Session {
    pub id: String,
    pub name: String,
    pub os: String,
    pub addr: String,
    pub ctrl: mpsc::UnboundedSender<Msg>,
    pub bulk: mpsc::Sender<Msg>,
}

#[derive(Clone, Debug)]
pub enum Mode {
    Local,
    /// `exit` is the raw OS point we left from, used to restore the cursor on disconnect.
    Forwarding { peer: String, exit: (f64, f64) },
    /// (x, y) is the virtual cursor in *desktop-normalised* coordinates (origin = top-left of all monitors).
    Controlled { peer: String, x: f64, y: f64 },
}

#[derive(Clone, Debug)]
pub struct Discovered {
    pub id: String,
    pub name: String,
    pub os: String,
    pub addrs: Vec<IpAddr>,
    pub port: u16,
    pub error: Option<String>,
    pub next_try: Instant,
}

#[derive(Serialize, Clone)]
pub struct PeerView {
    pub id: String,
    pub name: String,
    pub os: String,
    pub connected: bool,
    pub discovered: bool,
    pub placed: bool,
    pub paused: bool,
    pub version: Option<String>,
    pub x: f64,
    pub y: f64,
    pub mons: Vec<Rect>,
    pub error: Option<String>,
    pub addr: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct Snapshot {
    pub settings: Settings,
    pub my_os: &'static str,
    pub version: &'static str,
    pub paused: bool,
    pub offline: bool,
    /// Android: "active" / "enabled" / "off" for the Screen Hopper keyboard; empty elsewhere.
    pub kbd: String,
    pub keys_sent: u64,
    pub keys_injected: u64,
    pub mode: &'static str,
    pub mode_peer: Option<String>,
    pub peers: Vec<PeerView>,
    pub my_mons: Vec<Rect>,
    pub perms: Permissions,
    pub logs: Vec<String>,
    pub local_ip: Option<String>,
}

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Default)]
struct HotkeyState {
    mods: HashSet<&'static str>,
    /// Combo: the key we swallowed, so its key-up (and auto-repeat) is swallowed too.
    swallow: Option<u16>,
    /// Double-tap bookkeeping.
    key_down_at: Option<Instant>,
    clean: bool,
    last_tap: Option<Instant>,
}

fn mod_group(hid: u16) -> Option<&'static str> {
    match hid {
        keymap::HID_LCTRL | keymap::HID_RCTRL => Some("ctrl"),
        keymap::HID_LSHIFT | keymap::HID_RSHIFT => Some("shift"),
        keymap::HID_LALT | keymap::HID_RALT => Some("alt"),
        keymap::HID_LGUI | keymap::HID_RGUI => Some("meta"),
        _ => None,
    }
}

/// Same physical key, treating left/right modifiers as one.
fn same_key(a: u16, b: u16) -> bool {
    a == b || matches!((mod_group(a), mod_group(b)), (Some(x), Some(y)) if x == y)
}

static GLOBAL: OnceLock<Arc<Core>> = OnceLock::new();

/// The running app's core, for callbacks that arrive from outside Rust (Android JNI).
#[allow(dead_code)]
pub fn global() -> Option<Arc<Core>> {
    GLOBAL.get().cloned()
}

pub fn set_global(c: Arc<Core>) {
    let _ = GLOBAL.set(c);
}

pub struct Core {
    /// Set by the input hook when the pause shortcut fires; acted on by a background task.
    pub pause_toggle_req: std::sync::atomic::AtomicBool,
    /// True while Settings is recording a new shortcut (so it doesn't fire while you type it).
    pub hk_recording: std::sync::atomic::AtomicBool,
    hk: Mutex<HotkeyState>,
    /// Mouse buttons physically held on this computer (bit per button) while it is in control.
    pub local_buttons: std::sync::atomic::AtomicU8,
    /// User pressed Disconnect: no outgoing auto-dial until Connect.
    pub offline: std::sync::atomic::AtomicBool,
    /// Computers whose user pressed Disconnect: we don't auto-dial them.
    pub suppressed: Mutex<HashSet<String>>,
    pub peer_paused: Mutex<HashSet<String>>,
    pub dirty: std::sync::atomic::AtomicBool,
    pub keys_sent: std::sync::atomic::AtomicU64,
    pub keys_injected: std::sync::atomic::AtomicU64,
    pub paused: std::sync::atomic::AtomicBool,
    pub scroll_logged: std::sync::atomic::AtomicBool,
    pub peer_versions: Mutex<HashMap<String, String>>,
    pub settings: RwLock<Settings>,
    pub sessions: Mutex<HashMap<String, Arc<Session>>>,
    pub discovered: Mutex<HashMap<String, Discovered>>,
    pub connecting: Mutex<HashSet<String>>,
    pub mode: Mutex<Mode>,
    pub injector: Injector,
    pub app: OnceLock<AppHandle>,
    pub rt: tokio::runtime::Handle,
    pub logs: Mutex<VecDeque<String>>,
    pub held_mods: Mutex<HashSet<u16>>,
    /// This computer's monitors in raw OS coordinates.
    pub mons: RwLock<Vec<Rect>>,
    pub peer_mons: Mutex<HashMap<String, Vec<Rect>>>,
    pub clip: crate::clipboard::ClipState,
    pub recv: crate::transfer::RecvState,
}

impl Core {
    pub fn new(rt: tokio::runtime::Handle) -> Arc<Core> {
        Core::with_settings(rt, config::load())
    }

    pub fn with_settings(rt: tokio::runtime::Handle, mut settings: Settings) -> Arc<Core> {
        // Migrate the old per-pair layout into the shared layout.
        if settings.positions.is_empty() && settings.peers.iter().any(|p| p.placed) {
            let me = settings.device_id.clone();
            settings.positions.insert(me, (0.0, 0.0));
            for p in settings.peers.clone().iter().filter(|p| p.placed) {
                settings.positions.insert(p.id.clone(), (p.x, p.y));
            }
            settings.layout_rev = 1;
        }
        Arc::new(Core {
            pause_toggle_req: Default::default(),
            hk_recording: Default::default(),
            hk: Mutex::new(HotkeyState::default()),
            local_buttons: Default::default(),
            offline: Default::default(),
            suppressed: Mutex::new(HashSet::new()),
            peer_paused: Mutex::new(HashSet::new()),
            dirty: Default::default(),
            keys_sent: Default::default(),
            keys_injected: Default::default(),
            paused: Default::default(),
            scroll_logged: Default::default(),
            peer_versions: Mutex::new(HashMap::new()),
            settings: RwLock::new(settings),
            sessions: Mutex::new(HashMap::new()),
            discovered: Mutex::new(HashMap::new()),
            connecting: Mutex::new(HashSet::new()),
            mode: Mutex::new(Mode::Local),
            injector: Injector::new(),
            app: OnceLock::new(),
            rt,
            logs: Mutex::new(VecDeque::new()),
            held_mods: Mutex::new(HashSet::new()),
            mons: RwLock::new(platform::monitors()),
            peer_mons: Mutex::new(HashMap::new()),
            clip: Default::default(),
            recv: Default::default(),
        })
    }

    // ------------------------------------------------------------------ logging / UI

    pub fn log(&self, msg: impl Into<String>) {
        let msg = msg.into();
        log::info!("{msg}");
        let mut l = self.logs.lock();
        l.push_back(msg);
        while l.len() > 60 {
            l.pop_front();
        }
        drop(l);
        self.emit();
    }

    /// Mark the UI as stale. The actual push happens on a background task (see `emit_loop`),
    /// so input hooks never do heavy work or take extra locks.
    pub fn emit(&self) {
        self.dirty.store(true, std::sync::atomic::Ordering::Release);
    }

    pub fn emit_now(&self) {
        if let Some(app) = self.app.get() {
            let _ = app.emit("state", self.snapshot());
        }
    }

    /// Never hold two locks at once here: the input hook takes `mode` then `sessions`,
    /// so nesting them in the opposite order deadlocks (this froze Windows on hop-back).
    pub fn snapshot(&self) -> Snapshot {
        let (mode, mode_peer) = match &*self.mode.lock() {
            Mode::Local => ("local", None),
            Mode::Forwarding { peer, .. } => ("forwarding", Some(peer.clone())),
            Mode::Controlled { peer, .. } => ("controlled", Some(peer.clone())),
        };
        let settings = self.settings.read().clone();
        let sessions: HashMap<String, (String, String, String)> = self
            .sessions
            .lock()
            .iter()
            .map(|(k, v)| (k.clone(), (v.name.clone(), v.os.clone(), v.addr.clone())))
            .collect();
        let disc: HashMap<String, Discovered> = self.discovered.lock().clone();
        let versions = self.peer_versions.lock().clone();
        let pmons = self.peer_mons.lock().clone();
        let ppaused = self.peer_paused.lock().clone();
        let rel = rel_positions(&settings);
        let supp = self.suppressed.lock().clone();
        let mut peers: Vec<PeerView> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for p in &settings.peers {
            seen.insert(p.id.clone());
            let s = sessions.get(&p.id);
            let d = disc.get(&p.id);
            peers.push(PeerView {
                id: p.id.clone(),
                name: s.map(|s| s.0.clone()).or(d.map(|d| d.name.clone())).unwrap_or(p.name.clone()),
                os: s.map(|s| s.1.clone()).or(d.map(|d| d.os.clone())).unwrap_or_default(),
                connected: s.is_some(),
                discovered: d.is_some(),
                placed: rel.contains_key(&p.id),
                paused: ppaused.contains(&p.id),
                version: versions.get(&p.id).cloned(),
                x: rel.get(&p.id).map(|r| r.0).unwrap_or(0.0),
                y: rel.get(&p.id).map(|r| r.1).unwrap_or(0.0),
                mons: pmons.get(&p.id).cloned().unwrap_or_default(),
                error: if supp.contains(&p.id) && s.is_none() { Some("Disconnected by its user. Press Connect to reconnect.".into()) } else { d.and_then(|d| d.error.clone()) },
                addr: s.map(|s| s.2.clone()),
            });
        }
        for d in disc.values() {
            if seen.contains(&d.id) {
                continue;
            }
            seen.insert(d.id.clone());
            peers.push(PeerView {
                id: d.id.clone(),
                name: d.name.clone(),
                os: d.os.clone(),
                connected: sessions.contains_key(&d.id),
                discovered: true,
                placed: false,
                paused: false,
                version: None,
                x: 0.0,
                y: 0.0,
                mons: vec![],
                error: d.error.clone(),
                addr: None,
            });
        }
        Snapshot {
            settings,
            my_os: platform::os_name(),
            version: APP_VERSION,
            paused: self.paused.load(std::sync::atomic::Ordering::Relaxed),
            offline: self.offline.load(std::sync::atomic::Ordering::Relaxed),
            #[cfg(target_os = "android")]
            kbd: platform::keyboard_state(),
            #[cfg(not(target_os = "android"))]
            kbd: String::new(),
            keys_sent: self.keys_sent.load(std::sync::atomic::Ordering::Relaxed),
            keys_injected: self.keys_injected.load(std::sync::atomic::Ordering::Relaxed),
            mode,
            mode_peer,
            peers,
            my_mons: self.local_mons(),
            perms: platform::permissions(),
            logs: self.logs.lock().iter().rev().cloned().collect(),
            local_ip: local_ip(),
        }
    }

    // ------------------------------------------------------------------ helpers

    pub fn send_ctrl(&self, peer: &str, m: Msg) {
        if let Some(s) = self.sessions.lock().get(peer) {
            let _ = s.ctrl.send(m);
        }
    }

    pub fn peer_os(&self, peer: &str) -> Option<String> {
        self.sessions.lock().get(peer).map(|s| s.os.clone())
    }

    /// Every other computer's desktop top-left relative to ours, from the shared layout.
    pub fn rel_positions(&self) -> HashMap<String, (f64, f64)> {
        rel_positions(&self.settings.read())
    }

    fn layout_msg(&self) -> Msg {
        let s = self.settings.read();
        Msg::LayoutMap {
            rev: s.layout_rev,
            by: s.layout_by.clone(),
            positions: s.positions.iter().map(|(k, v)| (k.clone(), v.0, v.1)).collect(),
        }
    }

    fn broadcast_layout(&self, except: Option<&str>) {
        let m = self.layout_msg();
        let ids: Vec<String> = self.sessions.lock().keys().filter(|k| Some(k.as_str()) != except).cloned().collect();
        for id in ids {
            self.send_ctrl(&id, m.clone());
        }
    }

    fn bump_rev(s: &mut Settings) {
        s.layout_rev = now_ms().max(s.layout_rev + 1);
        s.layout_by = s.device_id.clone();
    }

    /// Width of a computer's desktop, if we know its monitors.
    fn desk_width(&self, id: &str, my_id: &str) -> f64 {
        if id == my_id {
            return platform::union_rect(&self.local_mons()).w;
        }
        self.peer_mons.lock().get(id).map(|m| platform::union_rect(m).w).unwrap_or(1920.0)
    }

    fn ensure_placed_all(&self) {
        let ids: Vec<String> = self.sessions.lock().keys().cloned().collect();
        for id in ids {
            self.ensure_placed(&id);
        }
    }

    /// Make sure both this computer and `peer` have a place in the shared layout.
    /// Only the lower id of the pair decides, to avoid two computers placing at once.
    fn ensure_placed(&self, peer: &str) {
        let my_id = self.settings.read().device_id.clone();
        if my_id.as_str() >= peer {
            return;
        }
        let (need_me, need_peer, ids) = {
            let s = self.settings.read();
            (!s.positions.contains_key(&my_id), !s.positions.contains_key(peer), s.positions.keys().cloned().collect::<Vec<_>>())
        };
        if !need_me && !need_peer {
            return;
        }
        // Right-most edge of everything already placed.
        let rights: Vec<(String, f64)> = ids.iter().map(|id| (id.clone(), self.desk_width(id, &my_id))).collect();
        {
            let mut s = self.settings.write();
            let right = rights
                .iter()
                .filter_map(|(id, w)| s.positions.get(id).map(|p| p.0 + w))
                .fold(f64::MIN, f64::max);
            let my_w = self.desk_width(&my_id, &my_id);
            let mut right = if s.positions.is_empty() { 0.0 } else { right };
            if need_me {
                s.positions.insert(my_id.clone(), (right, 0.0));
                right += my_w;
            }
            if need_peer {
                let y = s.positions.get(&my_id).map(|p| p.1).unwrap_or(0.0);
                s.positions.insert(peer.to_string(), (right, y));
            }
            Self::bump_rev(&mut s);
            config::save(&s);
        }
        self.broadcast_layout(None);
    }

    // ------------------------------------------------------------------ geometry

    fn origin(&self) -> (f64, f64) {
        let u = platform::union_rect(&self.mons.read());
        (u.x, u.y)
    }

    /// This computer's monitors relative to the desktop top-left.
    pub fn local_mons(&self) -> Vec<Rect> {
        let ms = self.mons.read();
        let (ox, oy) = {
            let u = platform::union_rect(&ms);
            (u.x, u.y)
        };
        geom::shifted(&ms, -ox, -oy)
    }

    /// Re-read the monitor list; tell peers if it changed.
    /// While this computer drives another one, check that the OS still lets us read the keyboard.
    /// Windows hides keys from normal apps while an administrator window is in front.
    pub fn check_keyboard_access(&self) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WARNED: AtomicBool = AtomicBool::new(false);
        let target = match &*self.mode.lock() {
            Mode::Forwarding { peer, .. } => Some(peer.clone()),
            _ => None,
        };
        let Some(peer) = target else {
            WARNED.store(false, Ordering::Relaxed);
            return;
        };
        if platform::foreground_blocks_keys() {
            if !WARNED.swap(true, Ordering::Relaxed) {
                let text = "an administrator window is in front on this Windows PC, so Windows blocks Screen Hopper from reading the keyboard. Click a normal window on Windows (or press the Windows key once), or run Screen Hopper as administrator.";
                self.log(format!("Warning: {text}"));
                self.send_ctrl(&peer, Msg::Notice { text: format!("Keyboard blocked: {text}") });
            }
        } else {
            WARNED.store(false, Ordering::Relaxed);
        }
    }

    /// macOS "Secure Event Input" (password fields etc.) hides keystrokes from every event tap.
    pub fn check_secure_input(&self) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static LAST: AtomicBool = AtomicBool::new(false);
        let now = platform::secure_input();
        if LAST.swap(now, Ordering::SeqCst) != now {
            if now {
                self.log("Warning: macOS Secure Input is on (a password field or secure terminal is open). Keystrokes can't be shared until it closes.");
            } else {
                self.log("Secure Input is off - keyboard sharing is available again");
            }
        }
    }

    pub fn refresh_monitors(&self) {
        let now = platform::monitors();
        if *self.mons.read() == now {
            return;
        }
        *self.mons.write() = now;
        let rects = self.local_mons();
        let ids: Vec<String> = self.sessions.lock().keys().cloned().collect();
        for id in ids {
            self.send_ctrl(&id, Msg::Monitors { rects: rects.clone() });
        }
        self.emit();
    }

    /// UI drag: `peer`'s desktop now sits at (x, y) relative to ours.
    pub fn set_layout(&self, peer: &str, x: f64, y: f64) {
        {
            let mut s = self.settings.write();
            let me = s.device_id.clone();
            let base = *s.positions.entry(me).or_insert((0.0, 0.0));
            s.positions.insert(peer.to_string(), (base.0 + x, base.1 + y));
            Self::bump_rev(&mut s);
            config::save(&s);
        }
        self.broadcast_layout(None);
        self.emit();
    }

    pub fn on_connected(&self, s: &Session) {
        {
            let mut st = self.settings.write();
            if let Some(p) = st.peers.iter_mut().find(|p| p.id == s.id) {
                p.name = s.name.clone();
            } else {
                st.peers.push(PeerLayout { id: s.id.clone(), name: s.name.clone(), x: 0.0, y: 0.0, placed: false });
            }
            config::save(&st);
        }
        self.send_ctrl(&s.id, Msg::AppVersion { version: APP_VERSION.to_string() });
        self.send_ctrl(&s.id, Msg::PauseState { on: self.paused.load(std::sync::atomic::Ordering::Relaxed) });
        self.send_ctrl(&s.id, Msg::Monitors { rects: self.local_mons() });
        self.send_ctrl(&s.id, self.layout_msg());
        self.log(format!("Connected to {} ({})", s.name, s.os));
    }

    pub fn on_disconnected(&self, id: &str, name: &str) {
        self.peer_mons.lock().remove(id);
        self.peer_versions.lock().remove(id);
        self.peer_paused.lock().remove(id);
        let mode = self.mode.lock().clone();
        match mode {
            Mode::Forwarding { peer, exit } if peer == id => {
                self.return_local(exit.0, exit.1);
            }
            Mode::Controlled { peer, x, y } if peer == id => {
                let (ox, oy) = self.origin();
                self.injector.release_all(x + ox, y + oy);
                *self.mode.lock() = Mode::Local;
            }
            _ => {}
        }
        self.log(format!("Disconnected from {name}"));
        self.update_dim();
    }

    /// Hand control back to this computer with the cursor at a raw OS point.
    fn return_local(&self, x: f64, y: f64) {
        *self.mode.lock() = Mode::Local;
        platform::on_back();
        self.local_buttons.store(0, std::sync::atomic::Ordering::Relaxed);
        platform::warp_cursor(x, y);
        self.held_mods.lock().clear();
        self.update_dim();
        self.emit();
    }

    /// Panic button / tray action: take control back right now.
    pub fn reclaim(&self) {
        let mode = self.mode.lock().clone();
        match mode {
            Mode::Forwarding { peer, exit } => {
                self.send_ctrl(&peer, Msg::Release);
                self.return_local(exit.0, exit.1);
                self.log("Control returned to this computer");
            }
            Mode::Controlled { x, y, .. } => {
                let (ox, oy) = self.origin();
                self.injector.release_all(x + ox, y + oy);
                *self.mode.lock() = Mode::Local;
                self.update_dim();
                self.emit();
            }
            Mode::Local => {}
        }
    }

    /// True when hopping to/from `peer` is paused on either side.
    pub fn hop_paused(&self, peer: &str) -> bool {
        self.paused.load(std::sync::atomic::Ordering::Relaxed) || self.peer_paused.lock().contains(peer)
    }

    /// Pause stops cursor hopping (both directions); clipboard and files keep working.
    pub fn set_paused(&self, on: bool) {
        use std::sync::atomic::Ordering;
        if on == self.paused.swap(on, Ordering::SeqCst) {
            return;
        }
        if on {
            self.reclaim();
        }
        let ids: Vec<String> = self.sessions.lock().keys().cloned().collect();
        for id in ids {
            self.send_ctrl(&id, Msg::PauseState { on });
        }
        #[cfg(desktop)]
        if let Some(app) = self.app.get() {
            crate::tray::refresh(app, on);
        }
        self.log(if on {
            "Paused: the cursor stays on each computer. Clipboard and files still work."
        } else {
            "Resumed: the cursor can hop between computers again"
        });
        self.emit();
    }

    /// Disconnect button: drop every connection and stop auto-reconnecting (others may still connect in).
    pub fn set_offline(self: &Arc<Self>, on: bool) {
        use std::sync::atomic::Ordering;
        if on == self.offline.swap(on, Ordering::SeqCst) {
            return;
        }
        if on {
            self.reclaim();
            let ids: Vec<String> = self.sessions.lock().keys().cloned().collect();
            for id in ids {
                self.send_ctrl(&id, Msg::GoingOffline);
                self.send_ctrl(&id, Msg::Bye);
            }
            self.log("Disconnected from all computers. Press Connect to reconnect.");
        } else {
            for d in self.discovered.lock().values_mut() {
                d.error = None;
                d.next_try = Instant::now();
            }
            let c = self.clone();
            self.rt.spawn(async move { crate::net::dial_all(&c, None) });
            self.log("Reconnecting to every computer on the network");
        }
        self.emit();
    }

    /// Remove a remembered computer that is not connected (it comes back if it shows up again).
    pub fn delete_peer(&self, id: &str) -> bool {
        if self.sessions.lock().contains_key(id) {
            return false;
        }
        {
            let mut s = self.settings.write();
            s.peers.retain(|p| p.id != id);
            if s.positions.remove(id).is_some() {
                Self::bump_rev(&mut s);
            }
            config::save(&s);
        }
        self.broadcast_layout(None);
        self.discovered.lock().remove(id);
        self.peer_mons.lock().remove(id);
        self.peer_versions.lock().remove(id);
        self.suppressed.lock().remove(id);
        self.emit();
        true
    }

    pub fn clear_offline(&self) -> usize {
        let connected: HashSet<String> = self.sessions.lock().keys().cloned().collect();
        let ids: Vec<String> = {
            let s = self.settings.read();
            let mut v: Vec<String> = s.peers.iter().filter(|p| !connected.contains(&p.id)).map(|p| p.id.clone()).collect();
            v.extend(self.discovered.lock().keys().filter(|k| !connected.contains(*k)).cloned());
            v.sort();
            v.dedup();
            v
        };
        for id in &ids {
            self.delete_peer(id);
        }
        ids.len()
    }

    pub fn update_dim(&self) {
        let (enabled, level) = {
            let s = self.settings.read();
            (s.dim_enabled, s.dim_level)
        };
        let dim = enabled && matches!(*self.mode.lock(), Mode::Forwarding { .. });
        #[cfg(desktop)]
        if let Some(app) = self.app.get() {
            crate::dim::set(app, dim, level);
        }
        #[cfg(not(desktop))]
        let _ = (dim, level);
    }

    fn swap_for(&self, peer: &str, hid: u16) -> u16 {
        let mac = |os: &str| os == "macOS";
        if self.settings.read().swap_cmd_ctrl && self.peer_os(peer).map(|o| mac(&o) != mac(platform::os_name())).unwrap_or(false) {
            keymap::swap_cmd_ctrl(hid)
        } else {
            hid
        }
    }

    // ------------------------------------------------------------------ capture side

    /// Watch every physical key for the pause shortcut. Returns true to swallow the key.
    pub fn hotkey_event(&self, hid: u16, down: bool) -> bool {
        use std::sync::atomic::Ordering;
        // The pause shortcut is not offered on Windows: never swallow or react to any key there.
        if cfg!(target_os = "windows") {
            return false;
        }
        let now = Instant::now();
        let mut st = self.hk.lock();
        if let Some(g) = mod_group(hid) {
            if down {
                st.mods.insert(g);
            } else {
                st.mods.remove(g);
            }
        }
        if self.hk_recording.load(Ordering::Relaxed) {
            return false;
        }
        let hk = self.settings.read().pause_hotkey.clone();
        if !hk.enabled || hk.key == 0 {
            return false;
        }
        if hk.double {
            // Two taps of one key with nothing else pressed in between. Generous timing so a normal,
            // unhurried double-tap counts: each press up to 0.5 s, and up to 0.5 s between the taps.
            const HOLD: Duration = Duration::from_millis(500);
            const GAP: Duration = Duration::from_millis(500);
            if down {
                if same_key(hid, hk.key) {
                    if st.key_down_at.is_none() {
                        if st.last_tap.map(|lt| now.duration_since(lt) > GAP).unwrap_or(false) {
                            st.last_tap = None; // too long since the first tap: start over
                        }
                        st.key_down_at = Some(now);
                        st.clean = true;
                    }
                } else {
                    st.clean = false;
                    st.last_tap = None;
                }
            } else if same_key(hid, hk.key) {
                if let Some(t) = st.key_down_at.take() {
                    if st.clean && now.duration_since(t) <= HOLD {
                        if st.last_tap.take().is_some() {
                            log::info!("pause shortcut: double-tap detected");
                            self.pause_toggle_req.store(true, Ordering::Release);
                        } else {
                            st.last_tap = Some(now);
                        }
                    } else {
                        st.last_tap = None;
                    }
                }
            }
            return false; // taps still reach apps
        }
        // Combination: exact modifier set + key.
        if let Some(sw) = st.swallow {
            if hid == sw {
                if !down {
                    st.swallow = None;
                }
                return true; // auto-repeat and key-up of the shortcut key
            }
        }
        if down && hid == hk.key && mod_group(hid).is_none() {
            let want: HashSet<&str> = hk.mods.iter().map(|m| m.as_str()).collect();
            let held: HashSet<&str> = st.mods.iter().copied().collect();
            if !want.is_empty() && want == held {
                st.swallow = Some(hid);
                log::info!("pause shortcut: combination detected");
                self.pause_toggle_req.store(true, Ordering::Release);
                return true;
            }
        }
        false
    }

    /// Called from the OS input hook for every physical input event. `true` = swallow.
    pub fn on_input(&self, ev: InputEvent) -> bool {
        if let InputEvent::Key { hid, down } = ev {
            if self.hotkey_event(hid, down) {
                return true;
            }
        }
        let mut mode = self.mode.lock();
        match &*mode {
            Mode::Controlled { .. } => false,
            Mode::Local => {
                if let InputEvent::Button { button, down } = ev {
                    let bit = 1u8 << button.min(7);
                    if down {
                        self.local_buttons.fetch_or(bit, std::sync::atomic::Ordering::Relaxed);
                    } else {
                        self.local_buttons.fetch_and(!bit, std::sync::atomic::Ordering::Relaxed);
                    }
                    return false;
                }
                let InputEvent::Move { x, y, dx, dy } = ev else { return false };
                if dx == 0.0 && dy == 0.0 || self.paused.load(std::sync::atomic::Ordering::Relaxed) {
                    return false;
                }
                // Never hop in the middle of a drag (resizing a window, selecting text...):
                // the button-up would land on the other computer and break the drag.
                if self.local_buttons.load(std::sync::atomic::Ordering::Relaxed) != 0 {
                    return false;
                }
                let (ox, oy) = self.origin();
                let (lx, ly) = (x - ox, y - oy);
                let mine = self.local_mons();
                let settings = self.settings.read();
                let sessions = self.sessions.lock();
                let pm = self.peer_mons.lock();
                // Probe a couple of pixels beyond the cursor in the direction of travel.
                let (sx, sy) = (dx.signum() * (dx != 0.0) as i32 as f64, dy.signum() * (dy != 0.0) as i32 as f64);
                let mut probes: Vec<(f64, f64)> = Vec::new();
                if sx != 0.0 {
                    probes.push((lx + sx * 2.0, ly));
                }
                if sy != 0.0 {
                    probes.push((lx, ly + sy * 2.0));
                }
                for (px, py) in probes {
                    if geom::rect_at(&mine, px, py).is_some() {
                        continue; // still on one of our own screens
                    }
                    let pp = self.peer_paused.lock().clone();
                    let rel = rel_positions(&settings);
                    for (pid, (rx, ry)) in rel.iter().filter(|(id, _)| !pp.contains(*id)) {
                        let (Some(sess), Some(rects)) = (sessions.get(pid), pm.get(pid)) else { continue };
                        let (qx, qy) = (px - rx, py - ry);
                        let Some(r) = geom::rect_at(rects, qx, qy) else { continue };
                        let (ex, ey) = geom::clamp_in(&r, lx - rx, ly - ry, 6.0);
                        let _ = sess.ctrl.send(Msg::Enter { x: ex, y: ey });
                        *mode = Mode::Forwarding { peer: pid.clone(), exit: (x, y) };
                        platform::on_away();
                        drop(pm);
                        drop(sessions);
                        drop(settings);
                        drop(mode);
                        self.held_mods.lock().clear();
                        self.update_dim();
                        self.emit();
                        return true;
                    }
                }
                false
            }
            Mode::Forwarding { peer, .. } => {
                let peer = peer.clone();
                let msg = match ev {
                    InputEvent::Move { dx, dy, .. } => {
                        if dx == 0.0 && dy == 0.0 {
                            return true;
                        }
                        Msg::MouseMove { dx: dx.round() as i32, dy: dy.round() as i32 }
                    }
                    InputEvent::Button { button, down } => Msg::MouseButton { button, down },
                    InputEvent::Wheel { dx, dy } => Msg::Wheel { dx, dy },
                    InputEvent::Key { hid, down } => {
                        // Panic hotkey: Ctrl+Alt+Shift+Esc returns control to this computer.
                        {
                            let mut m = self.held_mods.lock();
                            let is_mod = matches!(
                                hid,
                                keymap::HID_LCTRL | keymap::HID_RCTRL | keymap::HID_LSHIFT | keymap::HID_RSHIFT | keymap::HID_LALT | keymap::HID_RALT
                            );
                            if is_mod {
                                if down {
                                    m.insert(hid);
                                } else {
                                    m.remove(&hid);
                                }
                            }
                            let ctrl = m.contains(&keymap::HID_LCTRL) || m.contains(&keymap::HID_RCTRL);
                            let alt = m.contains(&keymap::HID_LALT) || m.contains(&keymap::HID_RALT);
                            let shift = m.contains(&keymap::HID_LSHIFT) || m.contains(&keymap::HID_RSHIFT);
                            if down && hid == keymap::HID_ESC && ctrl && alt && shift {
                                drop(m);
                                drop(mode);
                                self.reclaim();
                                return true;
                            }
                        }
                        let h = hid; // Cmd/Ctrl swap happens once, on the receiving computer
                        self.keys_sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        Msg::Key { hid: h, down }
                    }
                };
                if let Some(s) = self.sessions.lock().get(&peer) {
                    let _ = s.ctrl.send(msg);
                }
                true
            }
        }
    }

    // ------------------------------------------------------------------ message side

    pub fn handle(self: &Arc<Self>, peer: &str, m: Msg) {
        match m {
            Msg::LayoutMap { rev, by, positions } => {
                let newer = {
                    let mut s = self.settings.write();
                    if (rev, by.as_str()) > (s.layout_rev, s.layout_by.as_str()) {
                        // The newest map replaces ours wholesale, so every computer converges on the same map.
                        s.positions = positions.into_iter().map(|(id, x, y)| (id, (x, y))).collect();
                        s.layout_rev = rev;
                        s.layout_by = by;
                        config::save(&s);
                        true
                    } else {
                        false
                    }
                };
                if newer {
                    self.broadcast_layout(Some(peer));
                }
                self.ensure_placed_all();
                self.emit();
            }
            Msg::Monitors { rects } => {
                self.peer_mons.lock().insert(peer.to_string(), rects);
                self.ensure_placed_all();
                self.emit();
            }
            Msg::Handoff { to, x, y } => {
                // The computer we drive ran its cursor onto a third computer: drive that one instead.
                let ours = matches!(&*self.mode.lock(), Mode::Forwarding { peer: p, .. } if p == peer);
                if !ours {
                    return;
                }
                let ok = to != self.settings.read().device_id && !self.hop_paused(&to) && self.sessions.lock().contains_key(&to);
                if ok {
                    self.send_ctrl(&to, Msg::Enter { x, y });
                    if let Mode::Forwarding { peer: p, .. } = &mut *self.mode.lock() {
                        *p = to.clone();
                    }
                    self.emit();
                } else {
                    self.reclaim();
                }
            }
            Msg::Enter { x, y } => {
                // Paused here: refuse, and tell the controller so it takes its cursor back.
                if self.paused.load(std::sync::atomic::Ordering::Relaxed) {
                    self.send_ctrl(peer, Msg::PauseState { on: true });
                    return;
                }
                let mine = self.local_mons();
                let (x, y) = match geom::rect_at(&mine, x, y) {
                    Some(r) => geom::clamp_in(&r, x, y, 6.0),
                    None => geom::nearest(&mine, x, y),
                };
                *self.mode.lock() = Mode::Controlled { peer: peer.to_string(), x, y };
                let (ox, oy) = self.origin();
                // Start every visit with no keys or modifiers held, so nothing stale sticks.
                self.injector.release_all(x + ox, y + oy);
                self.injector.reset_modifiers();
                self.injector.move_to(x + ox, y + oy);
                self.update_dim();
                self.emit();
            }
            Msg::MouseMove { dx, dy } => {
                let mine = self.local_mons();
                // Every other computer whose screens we know, relative to us.
                let rel = self.rel_positions();
                let pm = self.peer_mons.lock().clone();
                let targets: Vec<(String, f64, f64, Vec<Rect>)> = rel
                    .into_iter()
                    .filter(|(id, _)| id == peer || !self.hop_paused(id))
                    .filter_map(|(id, (rx, ry))| pm.get(&id).map(|m| (id, rx, ry, m.clone())))
                    .collect();
                // (target id, x, y in the target's coordinates)
                let mut exit: Option<(String, f64, f64)> = None;
                let pos;
                {
                    let mut mode = self.mode.lock();
                    let Mode::Controlled { peer: p, x, y } = &mut *mode else { return };
                    if p.as_str() != peer {
                        return;
                    }
                    let (nx, ny) = (*x + dx as f64, *y + dy as f64);
                    let dragging = self.injector.buttons_held();
                    if geom::rect_at(&mine, nx, ny).is_some() {
                        *x = nx;
                        *y = ny;
                    } else if dragging {
                        // Mid-drag on this computer: keep the cursor here (slide along the edge).
                        let (cx, cy) = geom::nearest(&mine, nx, ny);
                        *x = cx;
                        *y = cy;
                    } else {
                        for (id, rx, ry, rects) in &targets {
                            let (cx, cy) = (nx - rx, ny - ry);
                            if let Some(r) = geom::rect_at(rects, cx, cy) {
                                let (ex, ey) = geom::clamp_in(&r, cx, cy, 6.0);
                                exit = Some((id.clone(), ex, ey));
                                break;
                            }
                        }
                        if exit.is_none() {
                            let (cx, cy) = geom::nearest(&mine, nx, ny);
                            *x = cx;
                            *y = cy;
                        }
                    }
                    pos = (*x, *y);
                }
                let (ox, oy) = self.origin();
                if let Some((to, ex, ey)) = exit {
                    self.injector.release_all(pos.0 + ox, pos.1 + oy);
                    *self.mode.lock() = Mode::Local;
                    if to == peer {
                        self.send_ctrl(peer, Msg::Leave { x: ex, y: ey });
                    } else {
                        self.send_ctrl(peer, Msg::Handoff { to, x: ex, y: ey });
                    }
                    self.update_dim();
                    self.emit();
                } else {
                    self.injector.move_to(pos.0 + ox, pos.1 + oy);
                }
            }
            Msg::MouseButton { button, down } => {
                if let Mode::Controlled { x, y, .. } = &*self.mode.lock() {
                    let (ox, oy) = self.origin();
                    self.injector.button(*x + ox, *y + oy, button, down);
                }
            }
            Msg::Wheel { dx, dy } => {
                if matches!(&*self.mode.lock(), Mode::Controlled { .. }) {
                    let flip = self.settings.read().reverse_scroll_here;
                    if !self.scroll_logged.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        self.log(format!("Scroll received - direction reversed on this computer: {}", if flip { "yes" } else { "no" }));
                    }
                    self.injector.wheel(dx, if flip { -dy } else { dy });
                }
            }
            Msg::Key { hid, down } => {
                if matches!(&*self.mode.lock(), Mode::Controlled { .. }) {
                    let h = self.swap_for(peer, hid);
                    self.keys_injected.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    self.injector.key(h, down);
                }
            }
            Msg::Leave { x, y } => {
                let ours = matches!(&*self.mode.lock(), Mode::Forwarding { peer: p, .. } if p == peer);
                if ours {
                    let mine = self.local_mons();
                    let (x, y) = match geom::rect_at(&mine, x, y) {
                        Some(r) => geom::clamp_in(&r, x, y, 6.0),
                        None => geom::nearest(&mine, x, y),
                    };
                    let (ox, oy) = self.origin();
                    self.return_local(x + ox, y + oy);
                }
            }
            Msg::Release => {
                let mode = self.mode.lock().clone();
                if let Mode::Controlled { peer: p, x, y } = mode {
                    if p == peer {
                        let (ox, oy) = self.origin();
                        self.injector.release_all(x + ox, y + oy);
                        *self.mode.lock() = Mode::Local;
                        self.update_dim();
                        self.emit();
                    }
                }
            }
            Msg::ClipChunk { id, total, last, data } => crate::clipboard::on_chunk(self, peer, id, total, last, data),
            Msg::FileStart { batch, file, path, size, count, clipboard } => {
                crate::transfer::on_start(self, peer, batch, file, path, size, count, clipboard)
            }
            Msg::FileChunk { batch, file, data } => crate::transfer::on_chunk(self, batch, file, data),
            Msg::FileEnd { batch, file } => crate::transfer::on_end(self, batch, file),
            Msg::AppVersion { version } => {
                self.peer_versions.lock().insert(peer.to_string(), version);
                self.emit();
            }
            Msg::PauseState { on } => {
                if on {
                    self.peer_paused.lock().insert(peer.to_string());
                    // If we were driving that computer, bring the cursor home.
                    let ours = matches!(&*self.mode.lock(), Mode::Forwarding { peer: p, .. } if p == peer);
                    if ours {
                        self.reclaim();
                    }
                    let controlled = matches!(&*self.mode.lock(), Mode::Controlled { peer: p, .. } if p == peer);
                    if controlled {
                        self.reclaim();
                    }
                    self.log("The other computer paused hopping");
                } else {
                    self.peer_paused.lock().remove(peer);
                    self.log("The other computer resumed hopping");
                }
                self.emit();
            }
            Msg::Notice { text } => {
                let name = self.sessions.lock().get(peer).map(|s| s.name.clone()).unwrap_or_default();
                self.log(format!("{name}: {text}"));
            }
            Msg::GoingOffline => {
                self.suppressed.lock().insert(peer.to_string());
                self.log("Another computer disconnected on purpose; it won't be reconnected automatically");
            }
            Msg::Hello { .. } | Msg::Ping | Msg::Pong | Msg::Bye | Msg::Frag { .. } => {}
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Other computers' desktop top-left relative to ours (only those placed alongside us).
pub fn rel_positions(s: &Settings) -> HashMap<String, (f64, f64)> {
    let Some(me) = s.positions.get(&s.device_id) else { return HashMap::new() };
    s.positions
        .iter()
        .filter(|(id, _)| **id != s.device_id)
        .map(|(id, p)| (id.clone(), (p.0 - me.0, p.1 - me.1)))
        .collect()
}

/// Best-effort LAN address (no packets are sent).
pub fn local_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    s.local_addr().ok().map(|a| a.ip().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A computer to our right; push the cursor into the shared edge.
    #[tokio::test]
    async fn no_hop_while_a_button_is_held() {
        let core = Core::with_settings(tokio::runtime::Handle::current(), Settings::default());
        let me = core.settings.read().device_id.clone();
        let w = platform::union_rect(&core.local_mons()).w;
        {
            let mut s = core.settings.write();
            s.positions.insert(me, (0.0, 0.0));
            s.positions.insert("peer".into(), (w, 0.0));
        }
        core.peer_mons.lock().insert("peer".into(), vec![Rect { x: 0.0, y: 0.0, w: 1000.0, h: 800.0 }]);
        let (ctrl, mut rx) = mpsc::unbounded_channel();
        let (bulk, _brx) = mpsc::channel(4);
        core.sessions.lock().insert(
            "peer".into(),
            Arc::new(Session { id: "peer".into(), name: "PC".into(), os: "Windows".into(), addr: String::new(), ctrl, bulk }),
        );
        let (ox, oy) = core.origin();
        let at_edge = InputEvent::Move { x: ox + w - 1.0, y: oy + 20.0, dx: 6.0, dy: 0.0 };

        // Dragging (left button held): the cursor must stay here.
        core.on_input(InputEvent::Button { button: 0, down: true });
        assert!(!core.on_input(at_edge), "hopped in the middle of a drag");
        assert!(matches!(*core.mode.lock(), Mode::Local));
        assert!(rx.try_recv().is_err(), "nothing may be sent to the other computer");

        // Button released: the same push hops over.
        core.on_input(InputEvent::Button { button: 0, down: false });
        assert!(core.on_input(at_edge), "should hop once the button is released");
        assert!(matches!(*core.mode.lock(), Mode::Forwarding { .. }));
        assert!(matches!(rx.try_recv(), Ok(Msg::Enter { .. })));
    }

    fn fired(c: &Core) -> bool {
        c.pause_toggle_req.swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    #[tokio::test]
    async fn pause_shortcut_combo() {
        let c = Core::with_settings(tokio::runtime::Handle::current(), Settings::default()); // Ctrl+Alt+P
        let k = |hid, down| c.hotkey_event(hid, down);
        // Ctrl+Alt+P fires once and the P is swallowed, including auto-repeat and key-up.
        assert!(!k(keymap::HID_LCTRL, true));
        assert!(!k(keymap::HID_RALT, true)); // either side counts
        assert!(k(0x13, true));
        assert!(fired(&c));
        assert!(k(0x13, true), "auto-repeat must be swallowed");
        assert!(!fired(&c), "auto-repeat must not toggle again");
        assert!(k(0x13, false));
        // Extra Shift: different shortcut, must not fire.
        assert!(!k(keymap::HID_LSHIFT, true));
        assert!(!k(0x13, true));
        assert!(!fired(&c));
        k(0x13, false);
        k(keymap::HID_LSHIFT, false);
        k(keymap::HID_RALT, false);
        k(keymap::HID_LCTRL, false);
        // Plain P never fires.
        assert!(!k(0x13, true));
        assert!(!fired(&c));
        k(0x13, false);
        // While Settings records a new shortcut it stays quiet.
        c.hk_recording.store(true, std::sync::atomic::Ordering::Relaxed);
        k(keymap::HID_LCTRL, true);
        k(keymap::HID_LALT, true);
        assert!(!k(0x13, true));
        assert!(!fired(&c));
    }

    #[tokio::test]
    async fn pause_shortcut_double_tap() {
        let mut st = Settings::default();
        st.pause_hotkey = crate::config::Hotkey { enabled: true, double: true, mods: vec![], key: keymap::HID_LCTRL };
        let c = Core::with_settings(tokio::runtime::Handle::current(), st);
        let tap = |hid| {
            c.hotkey_event(hid, true);
            c.hotkey_event(hid, false);
        };
        tap(keymap::HID_LCTRL);
        assert!(!fired(&c), "one tap is not enough");
        tap(keymap::HID_RCTRL); // right Ctrl counts as the same key
        assert!(fired(&c), "double-tap should toggle");
        // Ctrl+C then Ctrl: something else was pressed, so no toggle.
        c.hotkey_event(keymap::HID_LCTRL, true);
        c.hotkey_event(0x06, true);
        c.hotkey_event(0x06, false);
        c.hotkey_event(keymap::HID_LCTRL, false);
        tap(keymap::HID_LCTRL);
        assert!(!fired(&c), "a shortcut like Ctrl+C must not count as a tap");
        // A relaxed human double-tap: each press held 180 ms, 280 ms between taps.
        std::thread::sleep(std::time::Duration::from_millis(520));
        c.hotkey_event(keymap::HID_LCTRL, true);
        std::thread::sleep(std::time::Duration::from_millis(180));
        c.hotkey_event(keymap::HID_LCTRL, false);
        std::thread::sleep(std::time::Duration::from_millis(280));
        c.hotkey_event(keymap::HID_LCTRL, true);
        std::thread::sleep(std::time::Duration::from_millis(180));
        c.hotkey_event(keymap::HID_LCTRL, false);
        assert!(fired(&c), "a normal-speed double-tap must toggle");
        // Too slow between taps (start from a clean slate first).
        std::thread::sleep(std::time::Duration::from_millis(520));
        tap(keymap::HID_LCTRL);
        assert!(!fired(&c));
        std::thread::sleep(std::time::Duration::from_millis(520));
        tap(keymap::HID_LCTRL);
        assert!(!fired(&c), "taps too far apart");
    }
}
