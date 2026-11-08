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
    live: ag_plugin::live::LiveAudio,
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
    live_ctrl: ag_plugin::live::LiveController,
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
    let total_lat = a.plugin.latency().max(n.live_ctrl.latency());
    n.latency.store(total_lat as u32, Ordering::Relaxed);
}
#[no_mangle]
pub extern "C" fn ag_insert_create() -> *mut c_void {
    let plugin = AudioGridder::new();
    let (live, live_ctrl) = ag_plugin::live::LiveAudio::new();
    live_ctrl.connect(
        "127.0.0.1:55056",
        ag_protocol::HandshakeRequest {
            samples_per_block: 64,
            sample_rate: 48000.0,
            channels_in: 2,
            channels_out: 2,
            ..Default::default()
        },
    );
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
            live,
            scratch: vec![vec![0.0; 8192]; 2],
            scratch64: vec![vec![0.0; 8192]; 2],
            applied: vec![u32::MAX; params().len()],
        }),
        values,
        control: Mutex::new(AudioGridder::new()),
        live_ctrl,
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
    if !n.live_ctrl.connected() {
        n.live_ctrl.connect(
            "127.0.0.1:55056",
            ag_protocol::HandshakeRequest {
                samples_per_block: frames as i32,
                sample_rate: rate,
                channels_in: channels as i32,
                channels_out: channels as i32,
                ..Default::default()
            },
        );
    }
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
    for ch in 0..channels as usize {
        let input = *inputs.add(ch);
        if input.is_null() {
            a.scratch[ch][..frames as usize].fill(0.0);
        } else {
            a.scratch[ch][..frames as usize]
                .copy_from_slice(std::slice::from_raw_parts(input, frames as usize));
        }
    }
    a.live.process(&mut a.plugin, &mut a.scratch[..channels as usize], frames as usize, a.position.clone(), offline);
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
    for ch in 0..channels as usize {
        let input = *inputs.add(ch);
        if input.is_null() {
            a.scratch64[ch][..frames as usize].fill(0.0);
        } else {
            a.scratch64[ch][..frames as usize]
                .copy_from_slice(std::slice::from_raw_parts(input, frames as usize));
        }
    }
    a.live.process_f64(&mut a.plugin, &mut a.scratch64[..channels as usize], frames as usize, a.position.clone(), offline);
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

unsafe fn copy_cstr(src: &str, dest: *mut c_char, cap: u32) {
    if dest.is_null() || cap == 0 {
        return;
    }
    let bytes = src.as_bytes();
    let len = bytes.len().min(cap as usize - 1);
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest as *mut u8, len);
    *dest.add(len) = 0;
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_status(p: *mut c_void) -> i32 {
    let Some(n) = native(p) else { return 0 };
    if n.live_ctrl.connected() { 1 } else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_num_available_plugins(p: *mut c_void) -> u32 {
    let Some(n) = native(p) else { return 0 };
    n.live_ctrl.with_info(|info| info.catalog.len() as u32)
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_get_available_plugin_ext(
    p: *mut c_void,
    idx: u32,
    name_buf: *mut c_char,
    name_cap: u32,
    id_buf: *mut c_char,
    id_cap: u32,
    type_buf: *mut c_char,
    type_cap: u32,
    company_buf: *mut c_char,
    company_cap: u32,
    cat_buf: *mut c_char,
    cat_cap: u32,
) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.with_info(|info| {
        let Some(item) = info.catalog.get(idx as usize) else { return false };
        copy_cstr(&item.name, name_buf, name_cap);
        copy_cstr(&item.id, id_buf, id_cap);
        copy_cstr(&item.format, type_buf, type_cap);
        copy_cstr(&item.company, company_buf, company_cap);
        copy_cstr(&item.category, cat_buf, cat_cap);
        true
    })
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_load_plugin(p: *mut c_void, available_idx: u32) -> bool {
    let Some(n) = native(p) else { return false };
    let id_opt = n.live_ctrl.with_info(|info| {
        info.catalog.get(available_idx as usize).map(|item| item.id.clone())
    });
    if let Some(id) = id_opt {
        n.live_ctrl.send(ag_plugin::live::LiveCommand::Add { id, settings: String::new() })
    } else {
        false
    }
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_num_loaded_plugins(p: *mut c_void) -> u32 {
    let Some(n) = native(p) else { return 0 };
    n.live_ctrl.with_info(|info| info.slots.len() as u32)
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_get_loaded_plugin(
    p: *mut c_void,
    idx: u32,
    name_buf: *mut c_char,
    name_cap: u32,
    bypassed: *mut bool,
    oversampling: *mut u32,
) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.with_info(|info| {
        let Some(slot) = info.slots.get(idx as usize) else { return false };
        let display_name = if slot.name.is_empty() { &slot.id } else { &slot.name };
        copy_cstr(display_name, name_buf, name_cap);
        if !bypassed.is_null() { *bypassed = slot.bypassed; }
        if !oversampling.is_null() { *oversampling = slot.oversampling; }
        true
    })
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_unload_plugin(p: *mut c_void, loaded_idx: u32) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(ag_plugin::live::LiveCommand::Delete(loaded_idx as i32))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_set_plugin_bypassed(p: *mut c_void, loaded_idx: u32, bypassed: bool) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(ag_plugin::live::LiveCommand::Bypass(loaded_idx as i32, bypassed))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_set_plugin_oversampling(p: *mut c_void, loaded_idx: u32, factor: u32) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(ag_plugin::live::LiveCommand::Oversampling(loaded_idx as i32, factor as i32))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_exchange_plugins(p: *mut c_void, idx1: u32, idx2: u32) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(ag_plugin::live::LiveCommand::Move(idx1 as i32, idx1 < idx2))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_edit_plugin(p: *mut c_void, loaded_idx: u32) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(ag_plugin::live::LiveCommand::Edit(loaded_idx as i32))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_hide_plugin(p: *mut c_void) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(ag_plugin::live::LiveCommand::Hide)
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_num_presets(p: *mut c_void, slot_idx: u32) -> u32 {
    let Some(n) = native(p) else { return 0 };
    n.live_ctrl.with_info(|info| {
        info.slots.get(slot_idx as usize).map(|s| s.presets.len() as u32).unwrap_or(0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_get_preset_name(
    p: *mut c_void,
    slot_idx: u32,
    preset_idx: u32,
    buf: *mut c_char,
    cap: u32,
) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.with_info(|info| {
        let Some(slot) = info.slots.get(slot_idx as usize) else { return false };
        let Some(name) = slot.presets.get(preset_idx as usize) else { return false };
        copy_cstr(name, buf, cap);
        true
    })
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_set_preset(p: *mut c_void, slot_idx: u32, preset_idx: u32) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(ag_plugin::live::LiveCommand::Preset(slot_idx as i32, preset_idx as i32))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_num_plugin_params(p: *mut c_void, slot_idx: u32) -> u32 {
    let Some(n) = native(p) else { return 0 };
    n.live_ctrl.with_info(|info| {
        info.slots.get(slot_idx as usize).map(|s| s.params.len() as u32).unwrap_or(0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_get_plugin_param_info(
    p: *mut c_void,
    slot_idx: u32,
    param_idx: u32,
    name_buf: *mut c_char,
    name_cap: u32,
    default_val: *mut f32,
    current_val: *mut f32,
) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.with_info(|info| {
        let Some(slot) = info.slots.get(slot_idx as usize) else { return false };
        let Some(param) = slot.params.get(param_idx as usize) else { return false };
        copy_cstr(&param.name, name_buf, name_cap);
        if !default_val.is_null() { *default_val = param.default_value; }
        if !current_val.is_null() { *current_val = param.current; }
        true
    })
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_set_plugin_param(p: *mut c_void, slot_idx: u32, param_idx: u32, val: f32) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(ag_plugin::live::LiveCommand::Parameter(slot_idx as i32, param_idx as i32, val))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_connect(p: *mut c_void, endpoint: *const c_char) -> bool {
    let Some(n) = native(p) else { return false };
    if endpoint.is_null() { return false };
    let ep = std::ffi::CStr::from_ptr(endpoint).to_string_lossy().to_string();
    n.live_ctrl.connect(&ep, ag_protocol::HandshakeRequest {
        samples_per_block: 64,
        sample_rate: 48000.0,
        channels_in: 2,
        channels_out: 2,
        ..Default::default()
    })
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
