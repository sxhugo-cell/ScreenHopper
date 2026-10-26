//! Android backend. Android does not let an app capture global input, so the phone is always the
//! *receiving* side: a Kotlin AccessibilityService draws a pointer and performs taps, swipes,
//! typing and navigation. Everything OS-specific lives in Kotlin (`Bridge.kt`, the accessibility
//! service); Rust reaches it through the single `Bridge` object registered in `nativeInit`.
use super::{CaptureCb, Permissions, Rect};
use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jfloat};
use jni::{JNIEnv, JavaVM};
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct BridgeRef {
    vm: JavaVM,
    obj: GlobalRef,
}

static BRIDGE: Mutex<Option<Arc<BridgeRef>>> = Mutex::new(None);
static MODEL: Mutex<String> = Mutex::new(String::new());
/// Screen size in dp (density-independent), so cursor speed feels like it does on a computer.
static SCREEN: Mutex<(f64, f64)> = Mutex::new((360.0, 800.0));
static A11Y: AtomicBool = AtomicBool::new(false);

pub fn bridge_ready() -> bool {
    BRIDGE.lock().is_some()
}

pub fn device_model() -> String {
    MODEL.lock().clone()
}

/// Run `f` with a JNI env attached to the current thread and the Kotlin bridge object.
/// A Java exception is cleared and reported as `None`.
fn with_env<R>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<R>) -> Option<R> {
    let b = BRIDGE.lock().clone()?;
    let mut env = b.vm.attach_current_thread_permanently().ok()?;
    let r = f(&mut env, b.obj.as_obj());
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        return None;
    }
    r.ok()
}

fn call_void(name: &str, sig: &str, args: &[JValue]) -> bool {
    with_env(|env, obj| env.call_method(obj, name, sig, args).map(|_| ())).is_some()
}

fn call_bool(name: &str, sig: &str, args: &[JValue]) -> Option<bool> {
    with_env(|env, obj| env.call_method(obj, name, sig, args)?.z())
}

fn call_string(name: &str, sig: &str, args: &[JValue]) -> Option<String> {
    with_env(|env, obj| {
        let o = env.call_method(obj, name, sig, args)?.l()?;
        if o.is_null() {
            return Ok(None);
        }
        let js = JString::from(o);
        let s: String = env.get_string(&js)?.into();
        Ok(Some(s))
    })
    .flatten()
}

fn call_string_array(name: &str, sig: &str, args: &[JValue]) -> Vec<String> {
    with_env(|env, obj| {
        let o = env.call_method(obj, name, sig, args)?.l()?;
        if o.is_null() {
            return Ok(Vec::new());
        }
        let arr = jni::objects::JObjectArray::from(o);
        let n = env.get_array_length(&arr)?;
        let mut out = Vec::new();
        for i in 0..n {
            let e = env.get_object_array_element(&arr, i)?;
            let js = JString::from(e);
            out.push(env.get_string(&js)?.into());
        }
        Ok(out)
    })
    .unwrap_or_default()
}

fn call_with_string(name: &str, text: &str) -> Option<bool> {
    with_env(|env, obj| {
        let js = env.new_string(text)?;
        env.call_method(obj, name, "(Ljava/lang/String;)Z", &[JValue::Object(&js)])?.z()
    })
}

// ---------------------------------------------------------------------------------------------
// Kotlin -> Rust entry points (class com.screenhopper.app.Bridge)
// ---------------------------------------------------------------------------------------------

#[no_mangle]
pub extern "system" fn Java_com_screenhopper_app_Bridge_nativeInit(mut env: JNIEnv, _c: JClass, bridge: JObject, model: JString) {
    if let (Ok(vm), Ok(obj)) = (env.get_java_vm(), env.new_global_ref(&bridge)) {
        *BRIDGE.lock() = Some(Arc::new(BridgeRef { vm, obj }));
    }
    if let Ok(m) = env.get_string(&model) {
        *MODEL.lock() = m.into();
    }
    log::info!("android bridge ready");
}

#[no_mangle]
pub extern "system" fn Java_com_screenhopper_app_Bridge_nativeScreen(_env: JNIEnv, _c: JClass, w: jfloat, h: jfloat) {
    *SCREEN.lock() = (w as f64, h as f64);
}

#[no_mangle]
pub extern "system" fn Java_com_screenhopper_app_Bridge_nativeA11y(_env: JNIEnv, _c: JClass, on: jboolean) {
    A11Y.store(on != 0, Ordering::Relaxed);
    if let Some(core) = crate::core::global() {
        core.emit();
    }
}

#[no_mangle]
pub extern "system" fn Java_com_screenhopper_app_Bridge_nativeClipboard(mut env: JNIEnv, _c: JClass, text: JString) {
    if let (Ok(t), Some(core)) = (env.get_string(&text), crate::core::global()) {
        crate::clipboard::local_text_changed(&core, t.into());
    }
}

// ---------------------------------------------------------------------------------------------
// Platform API used by the core
// ---------------------------------------------------------------------------------------------

pub fn permissions() -> Permissions {
    Permissions { accessibility: A11Y.load(Ordering::Relaxed), input_monitoring: true, required: true }
}

/// Opens Android's Accessibility settings, where the user switches Screen Hopper on.
pub fn request_permissions() {
    call_void("openAccessibilitySettings", "()V", &[]);
}

pub fn monitors() -> Vec<Rect> {
    let (w, h) = *SCREEN.lock();
    vec![Rect { x: 0.0, y: 0.0, w, h }]
}

#[allow(dead_code)]
pub fn cursor_pos() -> (f64, f64) {
    (0.0, 0.0)
}

/// The phone never owns the physical cursor, so there is nothing to move.
pub fn warp_cursor(_x: f64, _y: f64) {}

/// Android cannot capture other apps' input, so the phone never starts a hop.
pub fn start_capture(_cb: CaptureCb) {}

pub fn secure_input() -> bool {
    false
}
pub fn foreground_blocks_keys() -> bool {
    false
}
pub fn reset_privacy_on_update(_dir: &std::path::Path, _version: &str) {}
pub fn play_toggle_sound(_paused: bool) {}

pub struct Injector {
    held: Mutex<HashSet<u8>>,
}

impl Injector {
    pub fn new() -> Self {
        Injector { held: Mutex::new(HashSet::new()) }
    }

    /// Move the on-screen pointer (dp coordinates).
    pub fn move_to(&self, x: f64, y: f64) {
        call_void("cursorTo", "(FF)V", &[JValue::Float(x as f32), JValue::Float(y as f32)]);
    }

    pub fn button(&self, x: f64, y: f64, button: u8, down: bool) {
        {
            let mut h = self.held.lock();
            if down {
                h.insert(button);
            } else {
                h.remove(&button);
            }
        }
        call_void(
            "button",
            "(IZFF)V",
            &[JValue::Int(button as i32), JValue::Bool(down as u8), JValue::Float(x as f32), JValue::Float(y as f32)],
        );
    }

    pub fn wheel(&self, dx: i32, dy: i32) {
        call_void("wheel", "(II)V", &[JValue::Int(dx), JValue::Int(dy)]);
    }

    pub fn key(&self, hid: u16, down: bool) {
        call_void("key", "(IZ)V", &[JValue::Int(hid as i32), JValue::Bool(down as u8)]);
    }

    pub fn reset_modifiers(&self) {
        call_void("resetKeys", "()V", &[]);
    }

    pub fn buttons_held(&self) -> bool {
        !self.held.lock().is_empty()
    }

    /// Lift everything and hide the pointer: called whenever control leaves this phone.
    pub fn release_all(&self, x: f64, y: f64) {
        let held: Vec<u8> = self.held.lock().iter().copied().collect();
        for b in held {
            self.button(x, y, b, false);
        }
        call_void("resetKeys", "()V", &[]);
        call_void("hideCursor", "()V", &[]);
    }
}

// ---------------------------------------------------------------------------------------------
// Clipboard and file helpers (all implemented in Kotlin)
// ---------------------------------------------------------------------------------------------

pub fn set_clipboard_text(text: &str) {
    call_with_string("setClipboard", text);
}

pub fn read_clipboard_text() -> Option<String> {
    call_string("readClipboard", "()Ljava/lang/String;", &[])
}

/// Blocks until the user finishes the system file picker; returns private copies of the chosen files.
pub fn pick_files() -> Vec<String> {
    call_string_array("pickFiles", "()[Ljava/lang/String;", &[])
}

/// Copy a received file or folder into the phone's Downloads/Screen Hopper folder.
pub fn publish_to_downloads(path: &str) -> bool {
    call_with_string("publishToDownloads", path).unwrap_or(false)
}

/// "active", "enabled" or "off": whether the Screen Hopper keyboard is selected.
pub fn keyboard_state() -> String {
    call_string("keyboardState", "()Ljava/lang/String;", &[]).unwrap_or_else(|| "off".into())
}

pub fn keyboard_action(action: &str) {
    match action {
        "enable" => call_void("openKeyboardSettings", "()V", &[]),
        _ => call_void("pickKeyboard", "()V", &[]),
    };
}

pub fn open_downloads() {
    call_void("openDownloads", "()V", &[]);
}

pub fn log_line(level: i32, tag: &str, msg: &str) {
    use std::ffi::CString;
    if let (Ok(t), Ok(m)) = (CString::new(tag), CString::new(msg)) {
        unsafe {
            android_log_sys::__android_log_write(level, t.as_ptr(), m.as_ptr());
        }
    }
}

/// Windows only: keyboard focus while the cursor is on another computer.
pub fn on_away() {}
pub fn on_back() {}
