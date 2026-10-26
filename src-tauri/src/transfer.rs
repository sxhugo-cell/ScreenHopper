//! File transfer (drag & drop, "Send files…", and copy-files-then-paste across computers).
use crate::core::Core;
use crate::protocol::{Msg, CHUNK};
#[cfg(not(target_os = "android"))]
use clipboard_rs::{Clipboard, ClipboardContext};
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tauri::Emitter;

const MAX_CLIPBOARD_FILES_BYTES: u64 = 1024 * 1024 * 1024;

struct RecvFile {
    f: File,
}

struct Batch {
    count: u32,
    done: u32,
    clipboard: bool,
    root: PathBuf,
    tops: HashMap<String, String>,
    top_order: Vec<String>,
    from: String,
}

#[derive(Default)]
pub struct RecvState {
    files: Mutex<HashMap<(u64, u32), RecvFile>>,
    batches: Mutex<HashMap<u64, Batch>>,
}

#[derive(Serialize, Clone)]
pub struct Progress {
    pub dir: &'static str,
    pub name: String,
    pub done: u64,
    pub total: u64,
    pub finished: bool,
}

fn emit_progress(core: &Core, p: Progress) {
    if let Some(app) = core.app.get() {
        let _ = app.emit("transfer", p);
    }
}

fn collect(paths: &[PathBuf]) -> Vec<(PathBuf, String)> {
    fn walk(base: &Path, rel: &str, out: &mut Vec<(PathBuf, String)>) {
        if base.is_dir() {
            if let Ok(rd) = std::fs::read_dir(base) {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    walk(&e.path(), &format!("{rel}/{name}"), out);
                }
            }
        } else if base.is_file() {
            out.push((base.to_path_buf(), rel.to_string()));
        }
    }
    let mut out = Vec::new();
    for p in paths {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        walk(p, &name, &mut out);
    }
    out
}

/// Send files/folders to one peer (`Some(id)`) or every connected peer (`None`).
pub async fn send_paths(core: Arc<Core>, peer: Option<String>, paths: Vec<PathBuf>, clipboard: bool) {
    let list = collect(&paths);
    if list.is_empty() {
        return;
    }
    let total: u64 = list.iter().map(|(p, _)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
    if clipboard && total > MAX_CLIPBOARD_FILES_BYTES {
        core.log("Copied files are over 1 GB, so they were not synced automatically. Use Send files instead.");
        return;
    }
    let sessions: Vec<_> = core
        .sessions
        .lock()
        .values()
        .filter(|s| peer.as_ref().map(|p| *p == s.id).unwrap_or(true))
        .cloned()
        .collect();
    if sessions.is_empty() {
        core.log("No connected computer to send to");
        return;
    }
    let batch: u64 = rand::random();
    let label = list[0].1.split('/').next().unwrap_or("files").to_string();
    for s in sessions {
        let mut sent = 0u64;
        for (i, (path, rel)) in list.iter().enumerate() {
            let Ok(mut f) = File::open(path) else { continue };
            let size = f.metadata().map(|m| m.len()).unwrap_or(0);
            if s
                .bulk
                .send(Msg::FileStart { batch, file: i as u32, path: rel.clone(), size, count: list.len() as u32, clipboard })
                .await
                .is_err()
            {
                return;
            }
            let mut buf = vec![0u8; CHUNK];
            loop {
                let n = match f.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                sent += n as u64;
                if s.bulk.send(Msg::FileChunk { batch, file: i as u32, data: buf[..n].to_vec() }).await.is_err() {
                    return;
                }
                emit_progress(&core, Progress { dir: "send", name: label.clone(), done: sent, total, finished: false });
            }
            let _ = s.bulk.send(Msg::FileEnd { batch, file: i as u32 }).await;
        }
        emit_progress(&core, Progress { dir: "send", name: label.clone(), done: total, total, finished: true });
        core.log(format!("Sent {} file(s) to {}", list.len(), s.name));
    }
}

/// Reject anything that could escape the destination folder.
fn safe_rel(rel: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    for c in Path::new(&rel.replace('\\', "/")).components() {
        match c {
            Component::Normal(s) => {
                let s = s.to_string_lossy().into_owned();
                if s.contains(':') {
                    return None;
                }
                parts.push(s);
            }
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

fn unique(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.into());
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 2.. {
        let c = format!("{stem} ({n}){ext}");
        if !dir.join(&c).exists() {
            return c;
        }
    }
    unreachable!()
}

#[allow(clippy::too_many_arguments)]
pub fn on_start(core: &Core, peer: &str, batch: u64, file: u32, path: String, _size: u64, count: u32, clipboard: bool) {
    {
        let s = core.settings.read();
        if clipboard && (!s.clipboard_files || cfg!(target_os = "android")) {
            return; // Android does not take file lists on its clipboard
        }
        if !clipboard && !s.file_transfer {
            return;
        }
    }
    let Some(parts) = safe_rel(&path) else { return };
    let mut batches = core.recv.batches.lock();
    let b = batches.entry(batch).or_insert_with(|| {
        let root = if clipboard {
            std::env::temp_dir().join("ScreenHopper").join(format!("{batch:x}"))
        } else {
            PathBuf::from(core.settings.read().receive_dir.clone())
        };
        let _ = std::fs::create_dir_all(&root);
        Batch { count, done: 0, clipboard, root, tops: HashMap::new(), top_order: vec![], from: peer.to_string() }
    });
    let top = parts[0].clone();
    if !b.tops.contains_key(&top) {
        let final_name = if clipboard { top.clone() } else { unique(&b.root, &top) };
        b.tops.insert(top.clone(), final_name.clone());
        b.top_order.push(final_name);
    }
    let mut dest = b.root.join(&b.tops[&top]);
    for p in &parts[1..] {
        dest = dest.join(p);
    }
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(f) = File::create(&dest) {
        core.recv.files.lock().insert((batch, file), RecvFile { f });
    }
}

pub fn on_chunk(core: &Core, batch: u64, file: u32, data: Vec<u8>) {
    if let Some(rf) = core.recv.files.lock().get_mut(&(batch, file)) {
        let _ = rf.f.write_all(&data);
    }
}

pub fn on_end(core: &Core, batch: u64, file: u32) {
    if let Some(mut rf) = core.recv.files.lock().remove(&(batch, file)) {
        let _ = rf.f.flush();
    }
    let finished = {
        let mut batches = core.recv.batches.lock();
        let Some(b) = batches.get_mut(&batch) else { return };
        b.done += 1;
        if b.done >= b.count {
            batches.remove(&batch)
        } else {
            None
        }
    };
    let Some(b) = finished else { return };
    let name = core
        .sessions
        .lock()
        .get(&b.from)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "another computer".into());
    if b.clipboard {
        let paths: Vec<String> = b.top_order.iter().map(|t| b.root.join(t).to_string_lossy().into_owned()).collect();
        core.clip.suppress();
        #[cfg(not(target_os = "android"))]
        if let Ok(ctx) = ClipboardContext::new() {
            let _ = ctx.set_files(paths);
        }
        #[cfg(target_os = "android")]
        let _ = paths;
        core.clip.suppress();
        core.log(format!("Files from {name} are ready to paste"));
    } else {
        emit_progress(core, Progress { dir: "recv", name: name.clone(), done: 1, total: 1, finished: true });
        #[cfg(target_os = "android")]
        {
            // Hand each received item to Android's Downloads folder (visible in the Files app).
            let ok = b.top_order.iter().all(|t| crate::platform::publish_to_downloads(&b.root.join(t).to_string_lossy()));
            core.log(if ok {
                format!("Received {} item(s) from {name}. Find them in Downloads/Screen Hopper.", b.top_order.len())
            } else {
                format!("Received {} item(s) from {name}, but could not copy them to Downloads. They are in the app folder.", b.top_order.len())
            });
        }
        #[cfg(not(target_os = "android"))]
        core.log(format!("Received {} item(s) from {name} into {}", b.top_order.len(), b.root.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::safe_rel;

    #[test]
    fn rejects_path_escape() {
        assert!(safe_rel("../evil").is_none());
        assert!(safe_rel("a/../../evil").is_none());
        assert!(safe_rel("/etc/passwd").is_none());
        assert!(safe_rel("C:/Windows/x").is_none());
        assert!(safe_rel("").is_none());
        assert_eq!(safe_rel("dir/sub/file.txt").unwrap(), vec!["dir", "sub", "file.txt"]);
        assert_eq!(safe_rel("dir\\sub\\f").unwrap(), vec!["dir", "sub", "f"]);
    }
}
