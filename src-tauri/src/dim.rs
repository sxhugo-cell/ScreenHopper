//! Screen dimming: a click-through translucent overlay on every monitor of the computer that is
//! currently *not* in control, so it is obvious which screen the keyboard and mouse are driving.
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

pub fn set(app: &AppHandle, on: bool, level: f64) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if !on {
            for (label, w) in handle.webview_windows() {
                if label.starts_with("dim") {
                    let _ = w.hide();
                }
            }
            return;
        }
        let monitors = handle.available_monitors().unwrap_or_default();
        for (i, m) in monitors.iter().enumerate() {
            let label = format!("dim{i}");
            let w = match handle.get_webview_window(&label) {
                Some(w) => w,
                None => match WebviewWindowBuilder::new(&handle, &label, WebviewUrl::App("dim.html".into()))
                    .decorations(false)
                    .transparent(true)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .resizable(false)
                    .shadow(false)
                    .focused(false)
                    .visible(false)
                    .build()
                {
                    Ok(w) => w,
                    Err(e) => {
                        log::warn!("dim window: {e}");
                        continue;
                    }
                },
            };
            let p = m.position();
            let s = m.size();
            let _ = w.set_position(PhysicalPosition::new(p.x, p.y));
            let _ = w.set_size(PhysicalSize::new(s.width, s.height));
            let _ = w.set_ignore_cursor_events(true);
            let _ = w.eval(&format!("document.documentElement.style.setProperty('--dim','{:.2}')", level.clamp(0.1, 0.9)));
            let _ = w.show();
        }
    });
}
