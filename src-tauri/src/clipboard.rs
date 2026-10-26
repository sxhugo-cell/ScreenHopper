//! Clipboard sync: text, rich text, HTML, images (and file lists, via the file-transfer path).
use crate::core::Core;
use crate::protocol::{self, ClipPayload, Msg, CHUNK};
use clipboard_rs::common::RustImage;
use clipboard_rs::{
    Clipboard, ClipboardContent, ClipboardContext, ClipboardHandler, ClipboardWatcher, ClipboardWatcherContext,
    ContentFormat, RustImageData,
};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_PAYLOAD: usize = 40 * 1024 * 1024;

#[derive(Default)]
pub struct ClipState {
    last: Mutex<Vec<u8>>,
    suppress_until: Mutex<Option<Instant>>,
    rx: Mutex<HashMap<u64, Vec<u8>>>,
    next_id: AtomicU64,
}

impl ClipState {
    /// Ignore watcher callbacks for a moment (we just wrote the clipboard ourselves).
    pub fn suppress(&self) {
        *self.suppress_until.lock() = Some(Instant::now() + Duration::from_millis(1500));
    }
    fn suppressed(&self) -> bool {
        self.suppress_until.lock().map(|t| Instant::now() < t).unwrap_or(false)
    }
}

struct Handler(Arc<Core>);

impl ClipboardHandler for Handler {
    fn on_clipboard_change(&mut self) {
        on_change(&self.0);
    }
}

pub fn start(core: Arc<Core>) {
    std::thread::Builder::new()
        .name("sh-clipboard".into())
        .spawn(move || {
            let Ok(mut w) = ClipboardWatcherContext::new() else {
                log::warn!("clipboard watcher unavailable");
                return;
            };
            w.add_handler(Handler(core));
            w.start_watch();
        })
        .expect("spawn clipboard thread");
}

fn on_change(core: &Arc<Core>) {
    let (sync, files_on) = {
        let s = core.settings.read();
        (s.clipboard_sync, s.clipboard_files)
    };
    if !sync || core.clip.suppressed() || core.sessions.lock().is_empty() {
        return;
    }
    let Ok(ctx) = ClipboardContext::new() else { return };

    if ctx.has(ContentFormat::Files) {
        if !files_on {
            return;
        }
        if let Ok(files) = ctx.get_files() {
            let paths: Vec<std::path::PathBuf> = files.into_iter().map(Into::into).collect();
            let mut h = Sha256::new();
            for p in &paths {
                h.update(p.to_string_lossy().as_bytes());
            }
            let sig = h.finalize().to_vec();
            {
                let mut last = core.clip.last.lock();
                if *last == sig {
                    return;
                }
                *last = sig;
            }
            let c = core.clone();
            core.rt.spawn(async move {
                crate::transfer::send_paths(c, None, paths, true).await;
            });
            return;
        }
    }

    let mut p = ClipPayload::default();
    p.text = ctx.get_text().ok().filter(|s| !s.is_empty());
    p.html = ctx.get_html().ok().filter(|s| !s.is_empty());
    p.rtf = ctx.get_rich_text().ok().filter(|s| !s.is_empty());
    if ctx.has(ContentFormat::Image) {
        if let Ok(img) = ctx.get_image() {
            if let Ok(png) = img.to_png() {
                p.image_png = Some(png.get_bytes().to_vec());
            }
        }
    }
    if p.text.is_none() && p.html.is_none() && p.rtf.is_none() && p.image_png.is_none() {
        return;
    }
    let bytes = bincode::serialize(&p).unwrap_or_default();
    if bytes.is_empty() || bytes.len() > MAX_PAYLOAD {
        return;
    }
    let sig = Sha256::digest(&bytes).to_vec();
    {
        let mut last = core.clip.last.lock();
        if *last == sig {
            return;
        }
        *last = sig;
    }
    let id = core.clip.next_id.fetch_add(1, Ordering::Relaxed) + 1;
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
    *core.clip.last.lock() = Sha256::digest(&bytes).to_vec();
    core.clip.suppress();
    let Ok(ctx) = ClipboardContext::new() else { return };
    let mut items: Vec<ClipboardContent> = Vec::new();
    if let Some(png) = p.image_png {
        if let Ok(img) = RustImageData::from_bytes(&png) {
            items.push(ClipboardContent::Image(img));
        }
    }
    if let Some(t) = p.text {
        items.push(ClipboardContent::Text(t));
    }
    if let Some(h) = p.html {
        items.push(ClipboardContent::Html(h));
    }
    if let Some(r) = p.rtf {
        items.push(ClipboardContent::Rtf(r));
    }
    if !items.is_empty() {
        let _ = ctx.set(items);
        core.clip.suppress();
    }
}

#[allow(dead_code)]
pub fn encode_for_test(p: &ClipPayload) -> Vec<u8> {
    protocol::encode(&Msg::ClipChunk { id: 0, total: 0, last: true, data: bincode::serialize(p).unwrap() })
}
