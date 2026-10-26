//! Android clipboard sync (text only).
//!  * computer -> phone: the text is placed on the Android clipboard at once.
//!  * phone -> computer: Android only lets the app in front read the clipboard, so the Kotlin side
//!    reports a change when the app comes to the front, and the "Send clipboard" button reads it on demand.
use crate::core::Core;
use crate::platform;
use crate::protocol::{ClipPayload, Msg, CHUNK};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const MAX_PAYLOAD: usize = 8 * 1024 * 1024;
static LAST_TEXT: Mutex<String> = Mutex::new(String::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub struct ClipState {
    rx: Mutex<HashMap<u64, Vec<u8>>>,
}

impl ClipState {
    /// (Desktop uses this to ignore its own clipboard writes; Android filters by text instead.)
    pub fn suppress(&self) {}
}

pub fn start(_core: Arc<Core>) {}

pub fn on_chunk(core: &Core, _peer: &str, id: u64, total: u64, last: bool, data: Vec<u8>) {
    if !core.settings.read().clipboard_sync || total as usize > MAX_PAYLOAD {
        return;
    }
    let done = {
        let mut rx = core.clip.rx.lock();
        let buf = rx.entry(id).or_default();
        buf.extend_from_slice(&data);
        if buf.len() > MAX_PAYLOAD {
            rx.remove(&id);
            return;
        }
        if last {
            rx.remove(&id)
        } else {
            None
        }
    };
    let Some(bytes) = done else { return };
    let Ok(p) = bincode::deserialize::<ClipPayload>(&bytes) else { return };
    if let Some(text) = p.text.filter(|t| !t.is_empty()) {
        *LAST_TEXT.lock() = text.clone(); // so the change listener doesn't send it straight back
        platform::set_clipboard_text(&text);
    }
}

/// Send the phone's clipboard text to every connected computer.
fn send_text(core: &Arc<Core>, text: String) {
    if !core.settings.read().clipboard_sync || text.is_empty() {
        return;
    }
    let p = ClipPayload { text: Some(text), ..Default::default() };
    let Ok(bytes) = bincode::serialize(&p) else { return };
    if bytes.len() > MAX_PAYLOAD {
        return;
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let c = core.clone();
    core.rt.spawn(async move {
        let sessions: Vec<_> = c.sessions.lock().values().cloned().collect();
        let total = bytes.len() as u64;
        let n = bytes.chunks(CHUNK).count();
        for s in sessions {
            for (i, ch) in bytes.chunks(CHUNK).enumerate() {
                let m = Msg::ClipChunk { id, total, last: i + 1 == n, data: ch.to_vec() };
                if s.bulk.send(m).await.is_err() {
                    break;
                }
            }
        }
    });
}

/// Kotlin reports the clipboard changed (app came to the front, or text was copied while it was in front).
pub fn local_text_changed(core: &Arc<Core>, text: String) {
    {
        let mut last = LAST_TEXT.lock();
        if *last == text {
            return;
        }
        *last = text.clone();
    }
    send_text(core, text);
}

/// The "Send clipboard" button.
pub fn send_now(core: &Arc<Core>) -> bool {
    match platform::read_clipboard_text() {
        Some(t) if !t.is_empty() => {
            *LAST_TEXT.lock() = t.clone();
            send_text(core, t);
            true
        }
        _ => false,
    }
}
