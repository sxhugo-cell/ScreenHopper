use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PeerLayout {
    pub id: String,
    pub name: String,
    /// Where this peer's desktop top-left sits, in *my* desktop coordinates.
    pub x: f64,
    pub y: f64,
    pub placed: bool,
}

/// Pause/resume shortcut. Either a combination (mods + key) or a double-tap of one key.
/// Keys are USB HID codes; `mods` are side-less names: "ctrl", "alt", "shift", "meta" (Cmd / Win).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Hotkey {
    pub enabled: bool,
    pub double: bool,
    pub mods: Vec<String>,
    pub key: u16,
}

impl Default for Hotkey {
    fn default() -> Self {
        // Ctrl + Alt (Option) + P
        Hotkey { enabled: true, double: false, mods: vec!["ctrl".into(), "alt".into()], key: 0x13 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub device_id: String,
    pub device_name: String,
    pub pairing_code: String,
    pub port: u16,
    pub swap_cmd_ctrl: bool,
    /// macOS: start automatically when you log in (menu-bar app).
    pub launch_at_login: bool,
    pub pause_hotkey: Hotkey,
    /// Flip the vertical wheel on *this* computer while another computer controls it.
    pub reverse_scroll_here: bool,
    pub clipboard_sync: bool,
    pub clipboard_files: bool,
    pub file_transfer: bool,
    pub dim_enabled: bool,
    pub dim_level: f64,
    pub receive_dir: String,
    pub peers: Vec<PeerLayout>,
    /// Shared layout: each computer's desktop top-left in a common space (synced to all peers).
    pub positions: HashMap<String, (f64, f64)>,
    pub layout_rev: u64,
    pub layout_by: String,
}

impl Default for Settings {
    fn default() -> Self {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let id: [u8; 8] = rng.gen();
        const ALPHA: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
        let code: String = (0..8).map(|_| ALPHA[rng.gen_range(0..ALPHA.len())] as char).collect();
        // Android keeps received files in the app folder first, then copies them into Downloads.
        #[cfg(target_os = "android")]
        let recv = android_files_dir().join("received");
        #[cfg(not(target_os = "android"))]
        let recv = dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")).join("Screen Hopper");
        Settings {
            device_id: hex::encode(id),
            // On Android the Kotlin side sets this to the phone model before the library loads.
            device_name: std::env::var("SCREEN_HOPPER_DEVICE_NAME")
                .ok()
                .filter(|n| !n.is_empty())
                .or_else(|| hostname::get().ok().and_then(|h| h.into_string().ok()))
                .unwrap_or_else(|| "My Computer".into()),
            pairing_code: format!("{}-{}", &code[..4], &code[4..]),
            port: 24987,
            swap_cmd_ctrl: true,
            launch_at_login: true,
            pause_hotkey: Hotkey::default(),
            reverse_scroll_here: false,
            clipboard_sync: true,
            clipboard_files: true,
            file_transfer: true,
            dim_enabled: true,
            dim_level: 0.55,
            receive_dir: recv.to_string_lossy().into_owned(),
            peers: vec![],
            positions: HashMap::new(),
            layout_rev: 0,
            layout_by: String::new(),
        }
    }
}

/// The app's private files folder on Android: /data/data/<package>/files (package = process name).
#[cfg(target_os = "android")]
fn android_files_dir() -> PathBuf {
    let pkg = std::fs::read("/proc/self/cmdline")
        .ok()
        .and_then(|b| b.split(|c| *c == 0).next().map(|p| String::from_utf8_lossy(p).into_owned()))
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "com.screenhopper.app".into());
    PathBuf::from(format!("/data/data/{pkg}/files"))
}

pub fn dir() -> PathBuf {
    #[cfg(target_os = "android")]
    let base = android_files_dir().join("ScreenHopper");
    #[cfg(not(target_os = "android"))]
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("ScreenHopper");
    let d = std::env::var_os("SCREEN_HOPPER_CONFIG_DIR").map(PathBuf::from).unwrap_or(base);
    let _ = std::fs::create_dir_all(&d);
    d
}

fn path() -> PathBuf {
    dir().join("settings.json")
}

pub fn load() -> Settings {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| {
            let s = Settings::default();
            save(&s);
            s
        })
}

pub fn save(s: &Settings) {
    if let Ok(j) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(path(), j);
    }
}
