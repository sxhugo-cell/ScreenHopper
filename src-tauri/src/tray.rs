//! Menu-bar / notification-area icon that shows whether this computer is paused.
use tauri::image::Image;
use tauri::AppHandle;

#[cfg(target_os = "macos")]
const NORMAL: &[u8] = include_bytes!("../icons/tray/tray-template.png");
#[cfg(target_os = "macos")]
const PAUSED: &[u8] = include_bytes!("../icons/tray/tray-paused-template.png");
#[cfg(not(target_os = "macos"))]
const NORMAL: &[u8] = include_bytes!("../icons/tray/win-normal.png");
#[cfg(not(target_os = "macos"))]
const PAUSED: &[u8] = include_bytes!("../icons/tray/win-paused.png");

pub fn refresh(app: &AppHandle, paused: bool) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(t) = a.tray_by_id("main") else { return };
        if let Ok(img) = Image::from_bytes(if paused { PAUSED } else { NORMAL }) {
            let _ = t.set_icon(Some(img));
            #[cfg(target_os = "macos")]
            let _ = t.set_icon_as_template(true);
        }
        let _ = t.set_tooltip(Some(if paused { "Screen Hopper (paused)" } else { "Screen Hopper" }));
    });
}
