//! Native format boundary. Only the processing thread touches `audio` while active.
//! The controller exchanges parameter values through atomics; state serialization
//! uses a separate control-side instance and never locks the audio callback.
use ag_plugin::AudioGridder;
use openplugin::prelude::{ParamInfo, Plugin, Unit};
use std::{
    cell::UnsafeCell,
    ffi::{c_char, c_void},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        Mutex,
    },
};

struct Audio {
    position: ag_protocol::PositionInfo,
    plugin: AudioGridder,
    scratch: Vec<Vec<f32>>,
    scratch64: Vec<Vec<f64>>,
    applied: Vec<u32>,
}
struct Native {
    refs: AtomicUsize,
    host_alive: AtomicBool,
    audio: UnsafeCell<Audio>,
    values: Vec<AtomicU32>,
    control: Mutex<AudioGridder>,
    meters: [AtomicU32; 4],
    latency: AtomicU32,
    revision: AtomicU64,
    framebuffer: Mutex<goodlookindri_shell::Framebuffer>,
}
// The native wrapper follows VST3's lifecycle: prepare/reset/process are exclusive.
// Controller calls only access `values` and `control`, never `audio`.
unsafe impl Sync for Native {}
fn params() -> &'static [ParamInfo] {
    static INFO: std::sync::OnceLock<&'static openplugin::prelude::PluginInfo> =
        std::sync::OnceLock::new();
    INFO.get_or_init(|| AudioGridder::new().info()).params
}
unsafe fn native<'a>(p: *mut c_void) -> Option<&'a Native> {
    (p as *const Native).as_ref()
}
fn apply(n: &Native, a: &mut Audio) {
    for (i, param) in params().iter().enumerate() {
        let bits = n.values[i].load(Ordering::Relaxed);
        if a.applied[i] != bits {
            a.plugin.set_param(param.id, f32::from_bits(bits));
            a.applied[i] = bits;
        }
    }
    n.latency.store(a.plugin.latency() as u32, Ordering::Relaxed);
}
#[no_mangle]
pub extern "C" fn ag_insert_create() -> *mut c_void {
    let plugin = AudioGridder::new();
    let values = params()
        .iter()
        .map(|p| AtomicU32::new(p.default.to_bits()))
        .collect();
    Box::into_raw(Box::new(Native {
        refs: AtomicUsize::new(1),
        host_alive: AtomicBool::new(true),
        audio: UnsafeCell::new(Audio {
            position: ag_protocol::PositionInfo::default(),
            plugin,
            scratch: vec![vec![0.0; 8192]; 2],
            scratch64: vec![vec![0.0; 8192]; 2],
            applied: vec![u32::MAX; params().len()],
        }),
        values,
        control: Mutex::new(AudioGridder::new()),
        meters: std::array::from_fn(|_| AtomicU32::new((-100.0f32).to_bits())),
        latency: AtomicU32::new(ag_plugin::profile_samples(1) as u32),
        revision: AtomicU64::new(1),
        framebuffer: Mutex::new(goodlookindri_shell::Framebuffer::new(720, 420)),
    })) as *mut c_void
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_destroy(p: *mut c_void) {
    if let Some(n) = native(p) {
        n.host_alive.store(false, Ordering::Release);
        ag_insert_release(p);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_retain(p: *mut c_void) -> bool {
    let Some(n) = native(p) else {
        return false;
    };
    if !n.host_alive.load(Ordering::Acquire) {
        return false;
    }
    n.refs.fetch_add(1, Ordering::Relaxed);
    true
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_alive(p: *mut c_void) -> bool {
    native(p).is_some_and(|n| n.host_alive.load(Ordering::Acquire))
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_release(p: *mut c_void) {
    if let Some(n) = native(p) {
        if n.refs.fetch_sub(1, Ordering::Release) == 1 {
            std::sync::atomic::fence(Ordering::Acquire);
            drop(Box::from_raw(p as *mut Native));
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_prepare(
    p: *mut c_void,
    rate: f64,
    frames: u32,
    channels: u32,
) -> bool {
    if !rate.is_finite()
        || !(8000.0..=768000.0).contains(&rate)
        || !(1..=8192).contains(&frames)
        || !(1..=2).contains(&channels)
    {
        return false;
    }
    let Some(n) = native(p) else {
        return false;
    };
    let a = &mut *n.audio.get();
    apply(n, a);
    a.scratch.resize_with(channels as usize, || vec![0.0; 8192]);
    a.scratch64
        .resize_with(channels as usize, || vec![0.0; 8192]);
    a.plugin
        .prepare(rate as f32, frames as usize, channels as usize);
    true
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_reset(p: *mut c_void) {
    if let Some(n) = native(p) {
        (*n.audio.get()).plugin.reset();
    }
}
#[no_mangle]
pub extern "C" fn ag_insert_param_count() -> u32 {
    params().len() as u32
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_param_info(
    i: u32,
    name: *mut c_char,
    lo: *mut f64,
    hi: *mut f64,
    default: *mut f64,
    steps: *mut i32,
) -> bool {
    let Some(p) = params().get(i as usize) else {
        return false;
    };
    if !name.is_null() {
        let dest = std::slice::from_raw_parts_mut(name as *mut u8, 128);
        dest.fill(0);
        let bytes = p.name.as_bytes();
        let len = bytes.len().min(127);
        dest[..len].copy_from_slice(&bytes[..len]);
    }
    if !lo.is_null() {
        *lo = p.min as f64;
    }
    if !hi.is_null() {
        *hi = p.max as f64;
    }
    if !default.is_null() {
        *default = p.to_normalized(p.default) as f64;
    }
    if !steps.is_null() {
        *steps = if matches!(p.unit, Unit::Choice | Unit::Toggle) {
            (p.max - p.min) as i32
        } else {
            0
        };
    }
    true
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_get(p: *mut c_void, i: u32) -> f64 {
    let Some(n) = native(p) else {
        return 0.0;
    };
    let Some(param) = params().get(i as usize) else {
        return 0.0;
    };
    param.to_normalized(f32::from_bits(n.values[i as usize].load(Ordering::Relaxed))) as f64
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_set(p: *mut c_void, i: u32, normalized: f64) -> bool {
    let Some(n) = native(p) else {
        return false;
    };
    let Some(param) = params().get(i as usize) else {
        return false;
    };
    if !normalized.is_finite() {
        return false;
    }
    let value = param.from_normalized(normalized as f32).to_bits();
    if n.values[i as usize].swap(value, Ordering::Relaxed) != value {
        n.revision.fetch_add(1, Ordering::Relaxed);
    }
    true
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_latency(p: *mut c_void) -> u32 {
    native(p).map(|n| n.latency.load(Ordering::Relaxed)).unwrap_or(0)
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_process(
    p: *mut c_void,
    inputs: *const *const f32,
    outputs: *const *mut f32,
    channels: u32,
    frames: u32,
    offline: bool,
) -> bool {
    let Some(n) = native(p) else {
        return false;
    };
    if inputs.is_null() || outputs.is_null() || frames > 8192 || !(1..=2).contains(&channels) {
        return false;
    }
    let a = &mut *n.audio.get();
    if a.scratch.len() != channels as usize {
        return false;
    }
    apply(n, a);
    a.plugin.set_offline(offline);
    for ch in 0..channels as usize {
        let input = *inputs.add(ch);
        if input.is_null() {
            a.scratch[ch][..frames as usize].fill(0.0);
        } else {
            a.scratch[ch][..frames as usize]
                .copy_from_slice(std::slice::from_raw_parts(input, frames as usize));
        }
    }
    a.plugin.process(&mut a.scratch, frames as usize);
    publish_meters(n, &a.plugin);
    for ch in 0..channels as usize {
        let output = *outputs.add(ch);
        if !output.is_null() {
            std::ptr::copy_nonoverlapping(a.scratch[ch].as_ptr(), output, frames as usize);
        }
    }
    true
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_process64(
    p: *mut c_void,
    inputs: *const *const f64,
    outputs: *const *mut f64,
    channels: u32,
    frames: u32,
    offline: bool,
) -> bool {
    let Some(n) = native(p) else {
        return false;
    };
    if inputs.is_null() || outputs.is_null() || frames > 8192 || !(1..=2).contains(&channels) {
        return false;
    }
    let a = &mut *n.audio.get();
    if a.scratch64.len() != channels as usize {
        return false;
    }
    apply(n, a);
    a.plugin.set_offline(offline);
    for ch in 0..channels as usize {
        let input = *inputs.add(ch);
        if input.is_null() {
            a.scratch64[ch][..frames as usize].fill(0.0);
        } else {
            a.scratch64[ch][..frames as usize]
                .copy_from_slice(std::slice::from_raw_parts(input, frames as usize));
        }
    }
    a.plugin.process_f64(&mut a.scratch64, frames as usize);
    publish_meters(n, &a.plugin);
    for ch in 0..channels as usize {
        let output = *outputs.add(ch);
        if !output.is_null() {
            std::ptr::copy_nonoverlapping(a.scratch64[ch].as_ptr(), output, frames as usize);
        }
    }
    true
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_save(p: *mut c_void, out: *mut u8, capacity: u32) -> u32 {
    let Some(n) = native(p) else {
        return 0;
    };
    let Ok(mut control) = n.control.lock() else {
        return 0;
    };
    for (i, param) in params().iter().enumerate() {
        control.set_param(
            param.id,
            f32::from_bits(n.values[i].load(Ordering::Relaxed)),
        );
    }
    let Some(bytes) = control.save_state() else {
        return 0;
    };
    if !out.is_null() && capacity as usize >= bytes.len() {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len());
    }
    bytes.len() as u32
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_load(p: *mut c_void, data: *const u8, length: u32) -> bool {
    let Some(n) = native(p) else {
        return false;
    };
    if data.is_null() || length > 16 * 1024 * 1024 {
        return false;
    }
    let Ok(mut control) = n.control.lock() else {
        return false;
    };
    if !control.load_state(std::slice::from_raw_parts(data, length as usize)) {
        return false;
    }
    for (i, param) in params().iter().enumerate() {
        if let Some(value) = control.param(param.id) {
            n.values[i].store(value.to_bits(), Ordering::Relaxed);
        }
    }
    n.revision.fetch_add(1, Ordering::Relaxed);
    true
}

fn publish_meters(n: &Native, p: &AudioGridder) {
    let levels = [
        p.input_vu_db(0),
        p.input_vu_db(1),
        p.output_vu_db(0),
        p.output_vu_db(1),
    ];
    for (i, value) in levels.into_iter().enumerate() {
        let old = f32::from_bits(n.meters[i].swap(value.to_bits(), Ordering::Relaxed));
        if (old * 2.0).round() != (value * 2.0).round() {
            n.revision.fetch_add(1, Ordering::Relaxed);
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_revision(p: *mut c_void) -> u64 {
    native(p)
        .map(|n| n.revision.load(Ordering::Relaxed))
        .unwrap_or(0)
}
#[no_mangle]
pub unsafe extern "C" fn ag_insert_render(
    p: *mut c_void,
    pixels: *mut u32,
    width: u32,
    height: u32,
) -> bool {
    if pixels.is_null() || !(720..=2160).contains(&width) || height != width * 420 / 720 {
        return false;
    }
    let Some(n) = native(p) else {
        return false;
    };
    let Ok(mut control) = n.control.lock() else {
        return false;
    };
    for (i, param) in params().iter().enumerate() {
        control.set_param(
            param.id,
            f32::from_bits(n.values[i].load(Ordering::Relaxed)),
        );
    }
    let mut list = openplugin::prelude::DrawList::new();
    control.render_editor_with_levels(
        &mut list,
        std::array::from_fn(|i| f32::from_bits(n.meters[i].load(Ordering::Relaxed))),
    );
    use openplugin::dri::{
        canvas::DrawCmd,
        goodlookindri::{
            color::Color,
            scene::{Cmd, Point, Rect},
        },
    };
    // Procedural widgets already publish their complete scene, including gradients.
    // Add the plugin's standalone labels and chain rectangles without duplicating widgets.
    for command in &list.commands {
        match command {
            DrawCmd::Text {
                text,
                x,
                y,
                size,
                color,
                align_center,
            } => {
                let point = Point::new(*x, *y);
                if !list
                    .scene
                    .cmds
                    .iter()
                    .any(|c| matches!(c,Cmd::Text{at,text:t,..}if *at==point&&t==text))
                {
                    list.scene.text(
                        point,
                        text.clone(),
                        Color(
                            ((color.a * 255.0) as u32) << 24
                                | ((color.r * 255.0) as u32) << 16
                                | ((color.g * 255.0) as u32) << 8
                                | (color.b * 255.0) as u32,
                        ),
                        *size,
                        *align_center,
                    );
                }
            }
            DrawCmd::FillRoundedRect {
                x,
                y,
                w,
                h,
                radius,
                color,
            } => {
                let rect = Rect::new(*x, *y, *w, *h);
                if !list
                    .scene
                    .cmds
                    .iter()
                    .any(|c| matches!(c,Cmd::FillRoundRect{rect:r,..}if *r==rect))
                {
                    let c = Color(
                        ((color.a * 255.0) as u32) << 24
                            | ((color.r * 255.0) as u32) << 16
                            | ((color.g * 255.0) as u32) << 8
                            | (color.b * 255.0) as u32,
                    );
                    list.scene.fill_round(rect, *radius, c, c, true);
                }
            }
            _ => {}
        }
    }
    let Ok(mut fb) = n.framebuffer.lock() else {
        return false;
    };
    if fb.width != width as usize || fb.height != height as usize {
        *fb = goodlookindri_shell::Framebuffer::new(width as usize, height as usize);
    }
    fb.clear(Color(0xff101010));
    fb.replay(&list.scene, width as f32 / 720.0);
    std::ptr::copy_nonoverlapping(fb.pixels.as_ptr(), pixels, fb.pixels.len());
    true
}

/// Audio-thread MIDI entry; offsets are relative to the segment being rendered.
#[no_mangle]
pub unsafe extern "C" fn ag_insert_midi(
    p: *mut c_void,
    offset: u32,
    status: u8,
    key: u8,
    velocity: u8,
) {
    if let Some(n) = native(p) {
        (*n.audio.get())
            .plugin
            .midi(offset as usize, [status, key, velocity]);
    }
}
/// Complete VST3 block position retained for the live audio request.
#[no_mangle]
pub unsafe extern "C" fn ag_insert_position(
    p: *mut c_void,
    bpm: f64,
    num: i32,
    den: i32,
    sample: i64,
    seconds: f64,
    ppq: f64,
    playing: bool,
    recording: bool,
    looping: bool,
) {
    if let Some(n) = native(p) {
        let a = &mut *n.audio.get();
        a.position = ag_protocol::PositionInfo {
            bpm,
            time_sig_numerator: num,
            time_sig_denominator: den,
            time_in_samples: sample,
            time_in_seconds: seconds,
            ppq_position: ppq,
            playing,
            recording,
            looping,
        };
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    #[test]
    fn midi_and_full_position_reach_the_audio_owned_instance() {
        unsafe {
            let raw = ag_insert_create();
            ag_insert_position(raw, 173.0, 7, 8, 999, 1.5, 3.25, true, true, true);
            ag_insert_midi(raw, 7, 0x92, 60, 100);
            let n = native(raw).unwrap();
            let a = &mut *n.audio.get();
            assert_eq!(a.position.time_sig_numerator, 7);
            assert_eq!(a.position.time_sig_denominator, 8);
            assert_eq!(a.position.time_in_seconds, 1.5);
            assert!(a.position.recording && a.position.looping);
            let mut samples = vec![vec![0.; 32]; 2];
            a.plugin.process(&mut samples, 32);
            assert_eq!(
                a.plugin.block_midi(),
                &[ag_plugin::QueuedMidi {
                    sample_offset: 7,
                    data: [0x92, 60, 100]
                }]
            );
            ag_insert_destroy(raw);
        }
    }
}
