//! Wire protocol. Every message is bincode-encoded, then Noise-encrypted and length-framed.
use crate::platform::Rect;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 11;
pub const CHUNK: usize = 32 * 1024;
/// Largest message sent in one encrypted frame (Noise allows 65535 bytes including its tag).
pub const FRAG_MAX: usize = 60_000;

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

#[allow(dead_code)]
impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Msg {
    Hello {
        id: String,
        name: String,
        os: String,
        version: u32,
    },
    /// The shared desk layout: every computer's desktop top-left in one common coordinate space.
    /// Newest `(rev, by)` wins everywhere; receivers pass newer maps on to their other peers.
    LayoutMap { rev: u64, by: String, positions: Vec<(String, f64, f64)> },
    /// Sent by a controlled computer to its controller: the cursor ran off our screens onto
    /// computer `to`, at (x, y) in `to`'s own desktop coordinates. The controller re-targets.
    Handoff { to: String, x: f64, y: f64 },
    /// The sender's monitors, relative to the top-left of its whole desktop.
    Monitors { rects: Vec<Rect> },
    /// Controller reclaims control (panic hotkey or disconnect).
    Release,
    /// Control moves to the receiver, cursor appearing at (x, y) in the receiver's desktop coordinates.
    Enter { x: f64, y: f64 },
    /// Control returns to the sender, cursor appearing at (x, y) in the sender's desktop coordinates.
    Leave { x: f64, y: f64 },
    MouseMove { dx: i32, dy: i32 },
    MouseButton { button: u8, down: bool },
    /// Wheel in 1/120-notch units (Windows native).
    Wheel { dx: i32, dy: i32 },
    Key { hid: u16, down: bool },
    /// Clipboard payload (bincode of `ClipPayload`), split into chunks.
    ClipChunk { id: u64, total: u64, last: bool, data: Vec<u8> },
    FileStart { batch: u64, file: u32, path: String, size: u64, count: u32, clipboard: bool },
    FileChunk { batch: u64, file: u32, data: Vec<u8> },
    FileEnd { batch: u64, file: u32 },
    /// The sender's app version string, for display.
    AppVersion { version: String },
    /// Sender paused (or resumed) cursor hopping. The connection stays up.
    PauseState { on: bool },
    /// Sender's user pressed Disconnect: don't auto-reconnect to it (manual Connect still works).
    GoingOffline,
    /// A message to show in the other computer's Activity log (e.g. a warning about its keyboard).
    Notice { text: String },
    Ping,
    Pong,
    Bye,
    /// One piece of a message too big for a single encrypted frame (pieces arrive in order, back to back).
    Frag { id: u32, idx: u16, total: u16, data: Vec<u8> },
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ClipPayload {
    pub text: Option<String>,
    pub html: Option<String>,
    pub rtf: Option<String>,
    pub image_png: Option<Vec<u8>>,
}

pub fn encode(m: &Msg) -> Vec<u8> {
    bincode::serialize(m).expect("encode")
}

pub fn decode(b: &[u8]) -> anyhow::Result<Msg> {
    Ok(bincode::deserialize(b)?)
}
