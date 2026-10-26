#[cfg_attr(target_os = "android", path = "clipboard_android.rs")]
mod clipboard;
mod config;
mod crashlog;
mod core;
#[cfg(desktop)]
mod dim;
mod geom;
mod keymap;
mod net;
mod platform;
mod protocol;
mod transfer;
#[cfg(desktop)]
mod tray;

use crate::config::Settings;
use crate::core::{Core, Snapshot};
use crate::protocol::Msg;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(desktop)]
use tauri::menu::{Menu, MenuItem};
#[cfg(desktop)]
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, State};

type CoreState<'a> = State<'a, Arc<Core>>;

#[tauri::command]
fn get_state(core: CoreState) -> Snapshot {
    core.snapshot()
}

#[derive(Deserialize)]
struct SettingsPatch {
    device_name: String,
    pairing_code: String,
    swap_cmd_ctrl: bool,
    launch_at_login: bool,
    pause_hotkey: config::Hotkey,
    reverse_scroll_here: bool,
    clipboard_sync: bool,
    clipboard_files: bool,
    file_transfer: bool,
    dim_enabled: bool,
    dim_level: f64,
    receive_dir: String,
}

#[tauri::command]
fn save_settings(core: CoreState, patch: SettingsPatch) {
    let code_changed;
    {
        let mut s = core.settings.write();
        code_changed = s.pairing_code != patch.pairing_code;
        s.device_name = patch.device_name;
        s.pairing_code = patch.pairing_code;
        s.swap_cmd_ctrl = patch.swap_cmd_ctrl;
        s.launch_at_login = patch.launch_at_login;
        s.pause_hotkey = patch.pause_hotkey;
        s.reverse_scroll_here = patch.reverse_scroll_here;
        s.clipboard_sync = patch.clipboard_sync;
        s.clipboard_files = patch.clipboard_files;
        s.file_transfer = patch.file_transfer;
        s.dim_enabled = patch.dim_enabled;
        s.dim_level = patch.dim_level;
        s.receive_dir = patch.receive_dir;
        config::save(&s);
    }
    if code_changed {
        let ids: Vec<String> = core.sessions.lock().keys().cloned().collect();
        for id in ids {
            core.send_ctrl(&id, Msg::Bye);
        }
        for d in core.discovered.lock().values_mut() {
            d.error = None;
            d.next_try = std::time::Instant::now();
        }
        core.log("Pairing code changed - reconnecting");
    }
    core.update_dim();
    apply_login_item(&core);
    core.emit();
}

#[tauri::command]
fn regenerate_code(core: CoreState) -> String {
    let fresh = Settings::default().pairing_code;
    {
        let mut s = core.settings.write();
        s.pairing_code = fresh.clone();
        config::save(&s);
    }
    let ids: Vec<String> = core.sessions.lock().keys().cloned().collect();
    for id in ids {
        core.send_ctrl(&id, Msg::Bye);
    }
    core.emit();
    fresh
}

#[tauri::command]
fn set_layout(core: CoreState, id: String, x: f64, y: f64) {
    core.set_layout(&id, x.round(), y.round());
}

#[tauri::command]
async fn connect_addr(core: CoreState<'_>, addr: String) -> Result<(), String> {
    let port = core.settings.read().port;
    core.offline.store(false, std::sync::atomic::Ordering::SeqCst);
    let addr = addr.trim();
    let sock: SocketAddr = if let Ok(s) = addr.parse::<SocketAddr>() {
        s
    } else if let Ok(ip) = addr.parse::<IpAddr>() {
        SocketAddr::new(ip, port)
    } else {
        return Err("Enter an IP address like 192.168.1.20".into());
    };
    net::connect(core.inner().clone(), sock).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn disconnect(core: CoreState, id: String) {
    core.send_ctrl(&id, Msg::Bye);
}

#[tauri::command]
fn forget_peer(core: CoreState, id: String) -> Result<(), String> {
    if core.delete_peer(&id) {
        Ok(())
    } else {
        Err("Disconnect this computer before deleting it".into())
    }
}

#[tauri::command]
fn clear_offline(core: CoreState) -> usize {
    core.clear_offline()
}

#[tauri::command]
async fn send_files(core: CoreState<'_>, peer: Option<String>, paths: Vec<String>) -> Result<(), String> {
    if !core.settings.read().file_transfer {
        return Err("File transfer is turned off in Settings".into());
    }
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    transfer::send_paths(core.inner().clone(), peer, paths, false).await;
    Ok(())
}

#[tauri::command]
fn set_paused(core: CoreState, paused: bool) {
    log::info!("pause requested from the app window: {paused}");
    core.set_paused(paused);
}

#[tauri::command]
fn set_offline(core: CoreState, offline: bool) {
    core.inner().set_offline(offline);
}

#[tauri::command]
fn connect_peer(core: CoreState, id: String) {
    core.suppressed.lock().remove(&id);
    if let Some(d) = core.discovered.lock().get_mut(&id) {
        d.error = None;
    }
    let c = core.inner().clone();
    core.rt.spawn(async move { net::dial_all(&c, Some(&id)) });
    core.emit();
}

#[tauri::command]
fn request_permissions() {
    platform::request_permissions();
}

#[tauri::command]
fn open_privacy_settings(pane: String) {
    let _ = &pane;
    #[cfg(target_os = "macos")]
    {
        let key = match pane.as_str() {
            "input" => "Privacy_ListenEvent",
            _ => "Privacy_Accessibility",
        };
        let _ = std::process::Command::new("open")
            .arg(format!("x-apple.systempreferences:com.apple.preference.security?{key}"))
            .spawn();
    }
    #[cfg(target_os = "android")]
    platform::request_permissions();
    #[cfg(not(any(target_os = "macos", target_os = "android")))]
    let _ = pane;
}

/// Android: let the user choose files with the system picker; returns private copies we can send.
#[tauri::command]
async fn pick_files() -> Vec<String> {
    #[cfg(target_os = "android")]
    {
        return tokio::task::spawn_blocking(platform::pick_files).await.unwrap_or_default();
    }
    #[cfg(not(target_os = "android"))]
    Vec::new()
}

/// Android: "enable" opens the keyboard settings, "pick" shows the keyboard chooser.
#[tauri::command]
fn keyboard_action(action: String) {
    #[cfg(target_os = "android")]
    platform::keyboard_action(&action);
    #[cfg(not(target_os = "android"))]
    let _ = action;
}

/// Android: send the phone's current clipboard text to the other computers.
#[tauri::command]
fn send_clipboard_now(core: CoreState) -> bool {
    #[cfg(target_os = "android")]
    {
        return clipboard::send_now(core.inner());
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = core;
        false
    }
}

#[tauri::command]
fn open_receive_dir(core: CoreState) {
    #[cfg(target_os = "android")]
    {
        let _ = core;
        platform::open_downloads();
        return;
    }
    #[cfg(not(target_os = "android"))]
    {
        let d = core.settings.read().receive_dir.clone();
        let _ = std::fs::create_dir_all(&d);
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&d).spawn();
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("explorer").arg(&d).spawn();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(&d).spawn();
    }
}

#[tauri::command]
fn open_log_dir() {
    let d = config::dir();
    let _ = &d;
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&d).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(&d).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(&d).spawn();
}

#[tauri::command]
fn reclaim(core: CoreState) {
    core.reclaim();
}

// ---------------------------------------------------------------------------------------------
// Menu-bar popover (macOS). On Windows the app keeps a normal window.
// ---------------------------------------------------------------------------------------------

#[allow(dead_code)]
const POP_W: f64 = 820.0;
#[allow(dead_code)]
const POP_H: f64 = 640.0;

/// Keep the popover open while a file dialog is showing, or when the user pinned it.
static KEEP_OPEN: AtomicBool = AtomicBool::new(false);
static PINNED: AtomicBool = AtomicBool::new(false);
#[allow(dead_code)]
static LAST_HIDDEN_MS: AtomicU64 = AtomicU64::new(0);

#[allow(dead_code)]
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn build_main_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    let url = tauri::WebviewUrl::App("index.html".into());
    #[cfg(target_os = "macos")]
    let w = tauri::WebviewWindowBuilder::new(app, "main", url)
        .title(format!("Screen Hopper v{}", core::APP_VERSION))
        .inner_size(POP_W, POP_H)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()?;
    #[cfg(target_os = "android")]
    let w = tauri::WebviewWindowBuilder::new(app, "main", url).build()?;
    #[cfg(not(any(target_os = "macos", target_os = "android")))]
    let w = tauri::WebviewWindowBuilder::new(app, "main", url)
        .title(format!("Screen Hopper v{}", core::APP_VERSION))
        .inner_size(960.0, 700.0)
        .min_inner_size(800.0, 560.0)
        .center()
        .build()?;
    let _ = w;
    Ok(())
}

/// Show the popover under the menu-bar icon (macOS) or just raise the window (Windows).
#[cfg(desktop)]
fn show_popover(app: &tauri::AppHandle, icon: Option<(f64, f64, f64, f64)>) {
    let Some(w) = app.get_webview_window("main") else { return };
    #[cfg(target_os = "macos")]
    {
        let scale = w.scale_factor().unwrap_or(2.0);
        if let Some((ix, iy, iw, ih)) = icon {
            let win_w = POP_W * scale;
            let mut x = ix + iw / 2.0 - win_w / 2.0;
            if let Ok(Some(m)) = w.current_monitor().or_else(|_| w.primary_monitor()) {
                let (mx, mw) = (m.position().x as f64, m.size().width as f64);
                x = x.clamp(mx + 8.0 * scale, mx + mw - win_w - 8.0 * scale);
            }
            let _ = w.set_position(tauri::PhysicalPosition::new(x.round() as i32, (iy + ih + 2.0 * scale).round() as i32));
            // Point the little notch at the icon.
            let notch = (ix + iw / 2.0 - x) / scale;
            let _ = w.eval(&format!("document.documentElement.style.setProperty('--notch','{notch:.0}px')"));
        }
    }
    let _ = icon;
    let _ = w.show();
    let _ = w.unminimize();
    let _ = w.set_focus();
}

#[cfg(desktop)]
fn show_main(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        // Anchor to the tray icon when we can find it.
        if let Some(t) = app.tray_by_id("main") {
            if let Ok(Some(r)) = t.rect() {
                let scale = app.get_webview_window("main").and_then(|w| w.scale_factor().ok()).unwrap_or(2.0);
                let p = r.position.to_physical::<f64>(scale);
                let s = r.size.to_physical::<f64>(scale);
                show_popover(app, Some((p.x, p.y, s.width, s.height)));
                return;
            }
        }
    }
    show_popover(app, None);
}

#[cfg(desktop)]
fn toggle_popover(app: &tauri::AppHandle, icon: (f64, f64, f64, f64)) {
    let Some(w) = app.get_webview_window("main") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        LAST_HIDDEN_MS.store(now_ms(), Ordering::Relaxed);
    } else if now_ms().saturating_sub(LAST_HIDDEN_MS.load(Ordering::Relaxed)) > 250 {
        // (A click on the icon also blurs the popover; the guard stops it re-opening at once.)
        show_popover(app, Some(icon));
    }
}

fn apply_login_item(core: &Core) {
    if std::env::var_os("SCREEN_HOPPER_CONFIG_DIR").is_some() {
        return; // test runs must not register themselves as a login item
    }
    #[cfg(target_os = "macos")]
    if let Some(app) = core.app.get() {
        use tauri_plugin_autostart::ManagerExt;
        let want = core.settings.read().launch_at_login;
        let al = app.autolaunch();
        let is = al.is_enabled().unwrap_or(false);
        if want && !is {
            let _ = al.enable();
        } else if !want && is {
            let _ = al.disable();
        }
    }
    let _ = core;
}

#[tauri::command]
fn set_hotkey_recording(core: CoreState, on: bool) {
    core.hk_recording.store(on, Ordering::Relaxed);
}

#[tauri::command]
fn set_keep_open(on: bool) {
    KEEP_OPEN.store(on, Ordering::Relaxed);
}

#[tauri::command]
fn set_pinned(on: bool) {
    PINNED.store(on, Ordering::Relaxed);
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[cfg(target_os = "macos")]
fn autostart_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--hidden"]))
}

#[cfg(not(target_os = "macos"))]
fn autostart_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("noop-autostart").build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    crashlog::init(config::dir());
    let rt = Arc::new(tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio"));
    let core = Core::new(rt.handle().clone());

    let core_setup = core.clone();
    let rt_setup = rt.clone();
    core::set_global(core.clone());
    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_dialog::init()).plugin(autostart_plugin());
    let app = builder
        .manage(core.clone())
        .manage(rt.clone())
        .setup(move |app| {
            let core = core_setup.clone();
            let _ = core.app.set(app.handle().clone());

            // Menu-bar app: never shows in the Dock or the Cmd-Tab switcher.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            build_main_window(app.handle())?;

            #[cfg(desktop)]
            {
                // Tray
                let show = MenuItem::with_id(app, "show", "Open Screen Hopper", true, None::<&str>)?;
                let pause = MenuItem::with_id(app, "pause", "Pause / resume hopping", true, None::<&str>)?;
                let back = MenuItem::with_id(app, "reclaim", "Return control to this computer", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show, &pause, &back, &quit])?;
                let c2 = core.clone();
                let mut tray = TrayIconBuilder::with_id("main").menu(&menu).tooltip("Screen Hopper").show_menu_on_left_click(false).on_tray_icon_event(|tray, ev| {
                    if let tauri::tray::TrayIconEvent::Click { button: tauri::tray::MouseButton::Left, button_state: tauri::tray::MouseButtonState::Up, rect, .. } = ev {
                        let app = tray.app_handle();
                        let scale = app.get_webview_window("main").and_then(|w| w.scale_factor().ok()).unwrap_or(2.0);
                        let p = rect.position.to_physical::<f64>(scale);
                        let s = rect.size.to_physical::<f64>(scale);
                        toggle_popover(app, (p.x, p.y, s.width, s.height));
                    }
                }).on_menu_event(move |app, ev| {
                    match ev.id.as_ref() {
                        "show" => show_main(app),
                        "reclaim" => c2.reclaim(),
                        "pause" => { log::info!("pause requested from the tray menu"); c2.set_paused(!c2.paused.load(std::sync::atomic::Ordering::Relaxed)) }
                        ,"quit" => app.exit(0),
                        _ => {}
                    }
                });
                #[cfg(target_os = "macos")]
                {
                    // White outline, drawn as a template so macOS tints it for light and dark menu bars.
                    let img = tauri::image::Image::from_bytes(include_bytes!("../icons/tray/tray-template.png"))?;
                    tray = tray.icon(img).icon_as_template(true);
                }
                #[cfg(not(target_os = "macos"))]
                if let Some(icon) = app.default_window_icon() {
                    tray = tray.icon(icon.clone());
                }
                tray.build(app)?;

                apply_login_item(&core);

                // First launch (not from the login item): open the popover once so it is obvious where the app lives.
                let hidden = std::env::args().any(|a| a == "--hidden");
                if !hidden {
                    let h = app.handle().clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(900));
                        let h2 = h.clone();
                        let _ = h.run_on_main_thread(move || show_main(&h2));
                    });
                }
            }

            // Network
            let _guard = rt_setup.enter();
            let daemon = net::advertise(&core);
            rt_setup.spawn(net::listen(core.clone()));
            rt_setup.spawn(net::auto_connect(core.clone()));
            #[cfg(desktop)]
            rt_setup.spawn(net::scan_lan(core.clone()));
            match daemon {
                Some(d) => {
                    rt_setup.spawn(net::discover(core.clone(), d.clone()));
                    app.manage(d);
                }
                None => core.log("Automatic discovery is unavailable. Use Connect by IP."),
            }

            // Linux (Wayland): apps cannot ask the OS for screens directly; GTK knows them, in logical pixels.
            #[cfg(target_os = "linux")]
            {
                let h = app.handle().clone();
                std::thread::spawn(move || loop {
                    if let Ok(ms) = h.available_monitors() {
                        let rects: Vec<platform::Rect> = ms
                            .iter()
                            .map(|m| {
                                let k = m.scale_factor().max(0.1);
                                platform::Rect {
                                    x: m.position().x as f64 / k,
                                    y: m.position().y as f64 / k,
                                    w: m.size().width as f64 / k,
                                    h: m.size().height as f64 / k,
                                }
                            })
                            .collect();
                        platform::set_monitors(rects);
                    }
                    std::thread::sleep(std::time::Duration::from_secs(5));
                });
            }
            platform::reset_privacy_on_update(&config::dir(), core::APP_VERSION);

            // Input + clipboard
            // macOS shows its own prompts at startup; on Android the banner button opens Settings instead.
            #[cfg(not(target_os = "android"))]
            if !platform::permissions().accessibility || !platform::permissions().input_monitoring {
                platform::request_permissions();
            }
            let c3 = core.clone();
            platform::start_capture(Box::new(move |ev| crashlog::guard(false, || c3.on_input(ev))));
            clipboard::start(core.clone());
            let c5 = core.clone();
            rt_setup.spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if c5.pause_toggle_req.swap(false, std::sync::atomic::Ordering::AcqRel) {
                        let on = !c5.paused.load(std::sync::atomic::Ordering::Relaxed);
                        c5.set_paused(on);
                        platform::play_toggle_sound(on);
                    }
                    if c5.dirty.swap(false, std::sync::atomic::Ordering::AcqRel) {
                        c5.emit_now();
                    }
                }
            });
            let c6 = core.clone();
            rt_setup.spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
                    c6.check_keyboard_access();
                }
            });
            let c4 = core.clone();
            rt_setup.spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    c4.refresh_monitors();
                    c4.check_secure_input();
                }
            });

            // Drag & drop of files onto the window is handled in the UI; hide overlay windows at start.
            core.log("Screen Hopper is running");
            Ok(())
        })
        .on_window_event(|w, ev| {
            if w.label() == "main" {
                match ev {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        let _ = w.hide();
                        api.prevent_close();
                    }
                    #[cfg(target_os = "macos")]
                    tauri::WindowEvent::Focused(false) => {
                        // Click anywhere else and the popover closes, like other menu-bar apps.
                        if !KEEP_OPEN.load(Ordering::Relaxed) && !PINNED.load(Ordering::Relaxed) {
                            let _ = w.hide();
                            LAST_HIDDEN_MS.store(now_ms(), Ordering::Relaxed);
                        }
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            save_settings,
            regenerate_code,
            set_layout,
            connect_addr,
            disconnect,
            forget_peer,
            clear_offline,
            send_files,
            request_permissions,
            set_paused,
            set_offline,
            connect_peer,
            open_privacy_settings,
            open_receive_dir,
            reclaim,
            open_log_dir,
            pick_files,
            keyboard_action,
            send_clipboard_now,
            set_keep_open,
            set_pinned,
            quit_app,
            set_hotkey_recording
        ])
        .build(tauri::generate_context!())
        .expect("error while building Screen Hopper");

    app.run(move |app, ev| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = ev {
            show_main(app);
        }
        let _ = (app, ev);
    });
}
