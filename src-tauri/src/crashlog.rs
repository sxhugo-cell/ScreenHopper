//! File logging + panic capture, so crashes leave evidence behind.
//! Files live next to settings.json: `screenhopper.log` (rolling) and `crash.log` (panics only).
use parking_lot::Mutex;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

const MAX_LOG: u64 = 2 * 1024 * 1024;

struct FileLogger {
    file: Mutex<Option<File>>,
    path: PathBuf,
}

fn stamp() -> String {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}.{:03}Z", (s / 3600) % 24, (s / 60) % 60, s % 60, d.subsec_millis())
}

impl log::Log for FileLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info || m.target().starts_with("screen_hopper")
    }
    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("{} {:5} {}: {}\n", stamp(), r.level(), r.target(), r.args());
        eprint!("{line}");
        #[cfg(target_os = "android")]
        crate::platform::log_line(4, "ScreenHopper", line.trim_end());
        let mut f = self.file.lock();
        if let Some(file) = f.as_mut() {
            let _ = file.write_all(line.as_bytes());
            if file.metadata().map(|m| m.len() > MAX_LOG).unwrap_or(false) {
                let _ = std::fs::rename(&self.path, self.path.with_extension("old.log"));
                *f = OpenOptions::new().create(true).append(true).open(&self.path).ok();
            }
        }
    }
    fn flush(&self) {
        if let Some(f) = self.file.lock().as_mut() {
            let _ = f.flush();
        }
    }
}

pub fn init(dir: PathBuf) {
    let path = dir.join("screenhopper.log");
    let file = OpenOptions::new().create(true).append(true).open(&path).ok();
    let logger = Box::leak(Box::new(FileLogger { file: Mutex::new(file), path }));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    let crash = dir.join("crash.log");
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let msg = format!(
            "\n==== {} v{} panic in thread '{}' ====\n{}\n{}\n",
            stamp(),
            env!("CARGO_PKG_VERSION"),
            thread,
            info,
            std::backtrace::Backtrace::force_capture()
        );
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&crash) {
            let _ = f.write_all(msg.as_bytes());
        }
        log::error!("PANIC in '{thread}': {info}");
        default_hook(info);
    }));
    log::info!("==== Screen Hopper v{} starting ({}) ====", env!("CARGO_PKG_VERSION"), crate::platform::os_name());
}

/// Run `f`, turning a panic into `fallback` instead of tearing down the process.
/// Needed wherever Rust is called from an OS callback (input hooks), where a panic would abort.
pub fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(fallback)
}
