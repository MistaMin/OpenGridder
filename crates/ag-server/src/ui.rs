//! Plugin editor windows (native/ui_mac.mm), AudioGridder's ProcessorWindow model:
//! the plugin's real editor in a real window on the server, captured and streamed to the
//! client, with the client's mouse and keys delivered to that window.
//!
//! An editor is identified by a nonzero id. Window and plugin-view work runs on the main
//! thread, which runs the AppKit loop (`run`). `capture` may run on any thread.

use std::ffi::{c_char, c_void, CString};

unsafe extern "C" {
    fn ag_ui_run();
    fn ag_ui_loop_running() -> bool;
    fn ag_ui_main_call(func: extern "C" fn(*mut c_void), ctx: *mut c_void);
    fn ag_ui_open_au(unit: *mut c_void, title: *const c_char, x: i32, y: i32) -> u64;
    fn ag_ui_open_native(native: *mut c_void, title: *const c_char, x: i32, y: i32) -> u64;
    fn ag_ui_close(id: u64);
    fn ag_ui_is_open(id: u64) -> bool;
    fn ag_ui_size(id: u64, width: *mut i32, height: *mut i32) -> bool;
    fn ag_ui_capture(id: u64, buffer: *mut u8, capacity: usize, width: *mut i32, height: *mut i32) -> i32;
    fn ag_ui_mouse(id: u64, kind: u8, x: f32, y: f32, shift: bool, ctrl: bool, alt: bool, dx: f32, dy: f32);
    fn ag_ui_key(id: u64, code: u16, shift: bool, ctrl: bool, alt: bool, cmd: bool);
    fn ag_ui_encode_png(rgba: *const u8, width: i32, height: i32, out_len: *mut usize) -> *mut u8;
    fn ag_ui_encode_jpeg(rgba: *const u8, width: i32, height: i32, quality: f64, out_len: *mut usize) -> *mut u8;
    fn ag_ui_free(p: *mut c_void);
    fn ag_ui_advertise(port: u16, txt: *const c_char) -> bool;
    fn ag_au_get_state(unit: *mut c_void, out_len: *mut usize) -> *mut u8;
    fn ag_au_set_state(unit: *mut c_void, bytes: *const u8, len: usize) -> bool;
    fn ag_native_get_state(native: *mut c_void, out_len: *mut usize) -> *mut u8;
    fn ag_native_set_state(native: *mut c_void, bytes: *const u8, len: usize) -> i32;
    fn ag_native_free(p: *mut c_void);
}

/// Run the AppKit loop on this (the main) thread. Never returns.
pub fn run() -> ! {
    unsafe { ag_ui_run() };
    std::process::exit(0)
}

/// True once `run` has started. Without a loop (tests, command-line modes) no editor
/// window can be shown and main-thread work runs inline.
pub fn loop_running() -> bool {
    unsafe { ag_ui_loop_running() }
}

/// Run `work` on the main thread and wait for it. Plugins are created and destroyed
/// there, as AudioGridder does, because frameworks such as JUCE bind their message
/// handling to the creating thread.
pub fn on_main<R, F: FnOnce() -> R>(work: F) -> R {
    struct Job<F, R> {
        work: Option<F>,
        result: Option<R>,
    }
    extern "C" fn trampoline<R, F: FnOnce() -> R>(ctx: *mut c_void) {
        let job = unsafe { &mut *(ctx as *mut Job<F, R>) };
        if let Some(work) = job.work.take() {
            job.result = Some(work());
        }
    }
    let mut job = Job { work: Some(work), result: None };
    unsafe { ag_ui_main_call(trampoline::<R, F>, &mut job as *mut Job<F, R> as *mut c_void) };
    job.result.expect("main-thread job did not run")
}

fn title_cstr(title: &str) -> CString {
    CString::new(title.replace('\0', "")).unwrap_or_default()
}

/// Show the editor of `target`. Returns the editor id, 0 when no window can be shown.
pub fn open_target(target: crate::host::EditorTarget, title: &str, x: i32, y: i32) -> u64 {
    match target {
        crate::host::EditorTarget::Au(unit) => open_au(unit as *mut c_void, title, x, y),
        crate::host::EditorTarget::Native(raw) => open_native(raw as *mut c_void, title, x, y),
    }
}

/// Show an AudioUnit's editor (its Cocoa view, AUv3 view controller, or Apple's generic
/// view). Returns the editor id, 0 when no window can be shown.
pub fn open_au(unit: *mut c_void, title: &str, x: i32, y: i32) -> u64 {
    if !loop_running() {
        return 0;
    }
    let title = title_cstr(title);
    unsafe { ag_ui_open_au(unit, title.as_ptr(), x, y) }
}

/// Show a VST3 or CLAP plugin's own editor. Returns 0 when it has none.
pub fn open_native(native: *mut c_void, title: &str, x: i32, y: i32) -> u64 {
    if !loop_running() {
        return 0;
    }
    let title = title_cstr(title);
    unsafe { ag_ui_open_native(native, title.as_ptr(), x, y) }
}

/// Detach the plugin view and close the window. Must happen before the plugin is destroyed.
pub fn close(id: u64) {
    if id != 0 {
        unsafe { ag_ui_close(id) }
    }
}

/// False once the window was closed, including by the user on the server's screen.
pub fn is_open(id: u64) -> bool {
    id != 0 && unsafe { ag_ui_is_open(id) }
}

pub fn size(id: u64) -> Option<(i32, i32)> {
    let (mut width, mut height) = (0, 0);
    (id != 0 && unsafe { ag_ui_size(id, &mut width, &mut height) }).then_some((width, height))
}

/// Capture the editor's content area into `buffer` as RGBA. Returns the image size.
pub fn capture(id: u64, buffer: &mut Vec<u8>) -> Option<(i32, i32)> {
    let (mut width, mut height) = (0, 0);
    let mut status = unsafe { ag_ui_capture(id, buffer.as_mut_ptr(), buffer.len(), &mut width, &mut height) };
    if status == -1 {
        buffer.resize(width.max(0) as usize * height.max(0) as usize * 4, 0);
        status = unsafe { ag_ui_capture(id, buffer.as_mut_ptr(), buffer.len(), &mut width, &mut height) };
    }
    (status == 1).then_some((width, height))
}

/// Mouse event in image coordinates (top-left origin), AudioGridder's types 0..=10.
#[allow(clippy::too_many_arguments)]
pub fn mouse(id: u64, kind: u8, x: f32, y: f32, shift: bool, ctrl: bool, alt: bool, dx: f32, dy: f32) {
    unsafe { ag_ui_mouse(id, kind, x, y, shift, ctrl, alt, dx, dy) }
}

/// AudioGridder `Worker::handleMessage(Key)`: modifier codes set flags, the copy, paste,
/// cut and select-all codes become Command+C/V/X/A, the remaining code is pressed.
pub fn keys(id: u64, codes: &[u16]) {
    let (mut shift, mut ctrl, mut alt, mut cmd) = (false, false, false, false);
    let mut key: Option<u16> = None;
    for &code in codes {
        match code {
            0x38 => shift = true,
            0x3B => ctrl = true,
            0x3A => alt = true,
            0xF0 => (key, cmd) = (Some(0x08), true), // C
            0xF1 => (key, cmd) = (Some(0x09), true), // V
            0xF2 => (key, cmd) = (Some(0x07), true), // X
            0xF3 => (key, cmd) = (Some(0x00), true), // A
            other => key = Some(other),
        }
    }
    if let Some(code) = key {
        unsafe { ag_ui_key(id, code, shift, ctrl, alt, cmd) }
    }
}

/// Register the server with Bonjour (`_audiogridder._tcp`) so clients on the network
/// list it. `txt` is the INFO record.
pub fn advertise(port: u16, txt: &str) -> bool {
    let txt = CString::new(txt).unwrap_or_default();
    unsafe { ag_ui_advertise(port, txt.as_ptr()) }
}

/// The plugin's own state, as a DAW saves it (AU ClassInfo, VST3 component and controller
/// state, CLAP state). `None` when it has none. Runs on the main thread (some frameworks
/// expect their state calls there) and needs no chain lock.
pub fn get_state(target: crate::host::EditorTarget) -> Option<Vec<u8>> {
    on_main(move || {
        let mut len = 0usize;
        let (raw, free): (*mut u8, unsafe extern "C" fn(*mut c_void)) = unsafe {
            match target {
                crate::host::EditorTarget::Au(unit) => (ag_au_get_state(unit as *mut c_void, &mut len), ag_ui_free),
                crate::host::EditorTarget::Native(raw) => (ag_native_get_state(raw as *mut c_void, &mut len), ag_native_free),
            }
        };
        if raw.is_null() {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(raw, len) }.to_vec();
        unsafe { free(raw as *mut c_void) };
        Some(bytes)
    })
}

/// Restore state saved by `get_state`. False when the plugin refused it.
pub fn set_state(target: crate::host::EditorTarget, bytes: &[u8]) -> bool {
    on_main(move || unsafe {
        match target {
            crate::host::EditorTarget::Au(unit) => ag_au_set_state(unit as *mut c_void, bytes.as_ptr(), bytes.len()),
            crate::host::EditorTarget::Native(raw) => ag_native_set_state(raw as *mut c_void, bytes.as_ptr(), bytes.len()) == 1,
        }
    })
}

/// Compressed PNG via ImageIO.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    encode(width, height, rgba, None)
}

/// JPEG via ImageIO, quality 0..=1. Opaque: alpha is dropped.
pub fn encode_jpeg(width: u32, height: u32, rgba: &[u8], quality: f64) -> Option<Vec<u8>> {
    encode(width, height, rgba, Some(quality))
}

fn encode(width: u32, height: u32, rgba: &[u8], jpeg_quality: Option<f64>) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || rgba.len() < width as usize * height as usize * 4 {
        return None;
    }
    let mut len = 0usize;
    let raw = unsafe {
        match jpeg_quality {
            Some(quality) => ag_ui_encode_jpeg(rgba.as_ptr(), width as i32, height as i32, quality, &mut len),
            None => ag_ui_encode_png(rgba.as_ptr(), width as i32, height as i32, &mut len),
        }
    };
    if raw.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(raw, len) }.to_vec();
    unsafe { ag_ui_free(raw as *mut c_void) };
    Some(bytes)
}
