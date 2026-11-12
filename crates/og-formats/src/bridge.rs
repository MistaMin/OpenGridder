//! Native format boundary. Only the processing thread touches `audio` while active.
//! The controller exchanges parameter values through atomics; state serialization
//! uses a separate control-side instance and never locks the audio callback.
use og_plugin::AudioGridder;
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
    position: og_protocol::PositionInfo,
    plugin: AudioGridder,
    live: og_plugin::live::LiveAudio,
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
    live_ctrl: og_plugin::live::LiveController,
    meters: [AtomicU32; 4],
    latency: AtomicU32,
    revision: AtomicU64,
    framebuffer: Mutex<goodlookindri_shell::Framebuffer>,
    // Last ag_insert_prepare format, so a server picked later handshakes with the host's real
    // rate, block size and channel count. Zero until prepared.
    prepared_rate: AtomicU64,
    prepared_frames: AtomicU32,
    prepared_channels: AtomicU32,
    // The server this insert uses (picked in the editor or restored from the session), and the
    // format its connection was opened with, so prepare reconnects only when either changes.
    endpoint: Mutex<String>,
    connected_format: Mutex<(u64, u32, u32)>,
    // A/B: two complete versions of this insert (console, server, chain with each plugin's
    // settings). Switching saves the side being left, so edits on either side are kept.
    ab: Mutex<AbSides>,
}

#[derive(Default)]
struct AbSides {
    a: Vec<u8>,
    b: Vec<u8>,
    on_b: bool,
}

const DEFAULT_ENDPOINT: &str = "127.0.0.1:55056";

/// Connect (or reconnect, keeping the chain) to the insert's server with the host's format.
fn connect_now(n: &Native) -> bool {
    let rate = f64::from_bits(n.prepared_rate.load(Ordering::Relaxed));
    let frames = n.prepared_frames.load(Ordering::Relaxed);
    let channels = n.prepared_channels.load(Ordering::Relaxed);
    let prepared = rate > 0.0 && frames > 0 && channels > 0;
    let request = og_protocol::HandshakeRequest {
        samples_per_block: if prepared { frames as i32 } else { 64 },
        sample_rate: if prepared { rate } else { 48000.0 },
        channels_in: if prepared { channels as i32 } else { 2 },
        channels_out: if prepared { channels as i32 } else { 2 },
        ..Default::default()
    };
    if let Ok(mut format) = n.connected_format.lock() {
        *format = (request.sample_rate.to_bits(), request.samples_per_block as u32, request.channels_in as u32);
    }
    let endpoint = n.endpoint.lock().map(|e| e.clone()).unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string());
    n.live_ctrl.connect(&endpoint, request)
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
    let (live, live_ctrl) = og_plugin::live::LiveAudio::new();
    let values = params()
        .iter()
        .map(|p| AtomicU32::new(p.default.to_bits()))
        .collect();
    Box::into_raw(Box::new(Native {
        refs: AtomicUsize::new(1),
        host_alive: AtomicBool::new(true),
        audio: UnsafeCell::new(Audio {
            position: og_protocol::PositionInfo::default(),
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
        latency: AtomicU32::new(og_plugin::profile_samples(1) as u32),
        revision: AtomicU64::new(1),
        framebuffer: Mutex::new(goodlookindri_shell::Framebuffer::new(720, 420)),
        prepared_rate: AtomicU64::new(0),
        prepared_frames: AtomicU32::new(0),
        prepared_channels: AtomicU32::new(0),
        endpoint: Mutex::new(DEFAULT_ENDPOINT.to_string()),
        connected_format: Mutex::new((0, 0, 0)),
        ab: Mutex::new(AbSides::default()),
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
    n.prepared_rate.store(rate.to_bits(), Ordering::Relaxed);
    n.prepared_frames.store(frames, Ordering::Relaxed);
    n.prepared_channels.store(channels, Ordering::Relaxed);
    // (Re)connect to this insert's server when not connected yet or when the host's format
    // changed since the connection was opened (the server's block size follows the handshake).
    let format = (rate.to_bits(), frames, channels);
    let changed = n.connected_format.lock().map(|f| *f != format).unwrap_or(true);
    if !n.live_ctrl.connected() || changed {
        connect_now(n);
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
unsafe fn write_param_cstr(dest: *mut c_char, capacity: u32, text: &str) -> bool {
    if dest.is_null() || capacity == 0 {
        return false;
    }
    let dest = std::slice::from_raw_parts_mut(dest as *mut u8, capacity as usize);
    let bytes = text.as_bytes();
    let len = bytes.len().min(dest.len() - 1);
    dest[..len].copy_from_slice(&bytes[..len]);
    dest[len] = 0;
    true
}
/// Stable string identifier of parameter `i` (the `ParamInfo::id`, e.g. "input_gain").
/// Hosts that key parameters by string (AAX) use it so automation survives reordering.
#[no_mangle]
pub unsafe extern "C" fn ag_insert_param_id(i: u32, out: *mut c_char, capacity: u32) -> bool {
    params().get(i as usize).is_some_and(|p| write_param_cstr(out, capacity, p.id))
}
/// Display text for parameter `i` at plain (unnormalized) value `plain`, with units
/// and choice labels exactly as the other formats show them ("-3.0 dB", "Tube").
#[no_mangle]
pub unsafe extern "C" fn ag_insert_param_text(i: u32, plain: f64, out: *mut c_char, capacity: u32) -> bool {
    params().get(i as usize).is_some_and(|p| write_param_cstr(out, capacity, &p.format(plain as f32)))
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
    let Some(bytes) = save_bytes(n) else {
        return 0;
    };
    if !out.is_null() && capacity as usize >= bytes.len() {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len());
    }
    bytes.len() as u32
}

/// The whole insert state, with every remote slot's settings read fresh from the server.
fn save_bytes(n: &Native) -> Option<Vec<u8>> {
    let Ok(mut control) = n.control.lock() else {
        return None;
    };
    for (i, param) in params().iter().enumerate() {
        control.set_param(
            param.id,
            f32::from_bits(n.values[i].load(Ordering::Relaxed)),
        );
    }
    // The remote chain lives in the network worker. Read every slot's current settings and
    // plugin state now (at most 3 s); with no session yet, keep the chain restored by load.
    // The control lock is released meanwhile so the worker is never blocked by it.
    drop(control);
    n.live_ctrl.sync_settings(std::time::Duration::from_secs(3));
    let Ok(mut control) = n.control.lock() else {
        return None;
    };
    let (connected, slots) = n.live_ctrl.with_info(|info| (info.connected, info.slots.clone()));
    if connected || !slots.is_empty() {
        control.set_loaded_plugins(slots.iter().map(slot_to_chain).collect());
    }
    control.save_state()
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
    // Rebuild the saved remote chain on the server (now, or as soon as one answers).
    let chain: Vec<og_plugin::LoadedSlot> = control.loaded_plugins().iter().map(chain_to_slot).collect();
    let server = control.active_server().to_string();
    drop(control);
    n.live_ctrl.restore(chain);
    // Reopen on the server the session was saved with.
    if let Ok(mut endpoint) = n.endpoint.lock() {
        *endpoint = server;
    }
    connect_now(n);
    n.revision.fetch_add(1, Ordering::Relaxed);
    true
}

/// A/B switch. The side being left is saved (so its edits are kept), then the other side is
/// applied. A side never visited starts as a copy of the current one.
#[no_mangle]
pub unsafe extern "C" fn ag_insert_toggle_ab(p: *mut c_void) -> bool {
    let Some(n) = native(p) else {
        return false;
    };
    let Some(current) = save_bytes(n) else {
        return false;
    };
    let target = {
        let Ok(mut ab) = n.ab.lock() else {
            return false;
        };
        if ab.on_b {
            ab.b = current;
        } else {
            ab.a = current;
        }
        ab.on_b = !ab.on_b;
        if ab.on_b { ab.b.clone() } else { ab.a.clone() }
    };
    n.revision.fetch_add(1, Ordering::Relaxed);
    if target.is_empty() {
        return true;
    }
    apply_side(n, &target)
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_is_b(p: *mut c_void) -> bool {
    native(p).and_then(|n| n.ab.lock().ok().map(|ab| ab.on_b)).unwrap_or(false)
}

/// Load one A/B side. With the same plugins on the same server, each slot's settings, bypass
/// and oversampling are applied to the running plugins (no reload, audio keeps going);
/// otherwise the side's chain is rebuilt like a session load.
fn apply_side(n: &Native, data: &[u8]) -> bool {
    let Ok(mut control) = n.control.lock() else {
        return false;
    };
    let server_before = control.active_server().to_string();
    if !control.load_state(data) {
        return false;
    }
    for (i, param) in params().iter().enumerate() {
        if let Some(value) = control.param(param.id) {
            n.values[i].store(value.to_bits(), Ordering::Relaxed);
        }
    }
    let chain: Vec<og_plugin::ChainPlugin> = control.loaded_plugins().to_vec();
    let server = control.active_server().to_string();
    drop(control);
    let live: Vec<og_plugin::LoadedSlot> = n.live_ctrl.with_info(|info| info.slots.clone());
    let same_plugins = chain.len() == live.len() && chain.iter().zip(&live).all(|(saved, running)| saved.id == running.id);
    if server == server_before && same_plugins && n.live_ctrl.connected() {
        use og_plugin::live::LiveCommand;
        for (index, (saved, running)) in chain.iter().zip(&live).enumerate() {
            let index = index as i32;
            if !saved.settings.is_empty() && saved.settings != running.settings {
                n.live_ctrl.send(LiveCommand::ApplySettings(index, saved.settings.clone()));
            }
            if saved.bypassed != running.bypassed {
                n.live_ctrl.send(LiveCommand::Bypass(index, saved.bypassed));
            }
            let factor = saved.oversampling.max(1);
            if factor as u32 != running.oversampling.max(1) {
                n.live_ctrl.send(LiveCommand::Oversampling(index, factor));
            }
        }
    } else {
        n.live_ctrl.restore(chain.iter().map(chain_to_slot).collect());
        if let Ok(mut endpoint) = n.endpoint.lock() {
            *endpoint = server;
        }
        connect_now(n);
    }
    n.revision.fetch_add(1, Ordering::Relaxed);
    true
}

fn slot_to_chain(slot: &og_plugin::LoadedSlot) -> og_plugin::ChainPlugin {
    og_plugin::ChainPlugin {
        id: slot.id.clone(),
        settings: slot.settings.clone(),
        bypassed: slot.bypassed,
        layout: slot.layout.clone(),
        mono_channels: slot.mono,
        latency: 0,
        sidechain_disabled: false,
        oversampling: slot.oversampling as i32,
        params: Vec::new(),
    }
}

fn chain_to_slot(plugin: &og_plugin::ChainPlugin) -> og_plugin::LoadedSlot {
    og_plugin::LoadedSlot {
        name: plugin.id.clone(),
        id: plugin.id.clone(),
        settings: plugin.settings.clone(),
        layout: plugin.layout.clone(),
        mono: plugin.mono_channels,
        bypassed: plugin.bypassed,
        oversampling: plugin.oversampling.max(1) as u32,
        ..Default::default()
    }
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
        a.position = og_protocol::PositionInfo {
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
        n.live_ctrl.send(og_plugin::live::LiveCommand::Add { id, settings: String::new() })
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
    n.live_ctrl.send(og_plugin::live::LiveCommand::Delete(loaded_idx as i32))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_set_plugin_bypassed(p: *mut c_void, loaded_idx: u32, bypassed: bool) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(og_plugin::live::LiveCommand::Bypass(loaded_idx as i32, bypassed))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_set_plugin_oversampling(p: *mut c_void, loaded_idx: u32, factor: u32) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(og_plugin::live::LiveCommand::Oversampling(loaded_idx as i32, factor as i32))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_exchange_plugins(p: *mut c_void, idx1: u32, idx2: u32) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(og_plugin::live::LiveCommand::Move(idx1 as i32, idx1 < idx2))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_edit_plugin(p: *mut c_void, loaded_idx: u32) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(og_plugin::live::LiveCommand::Edit(loaded_idx as i32))
}

/// Edit with the server window's top-left at (x, y), top-left-origin screen coordinates.
#[no_mangle]
pub unsafe extern "C" fn ag_insert_edit_plugin_at(p: *mut c_void, loaded_idx: u32, x: i32, y: i32) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.set_editor_open(true);
    n.live_ctrl.send(og_plugin::live::LiveCommand::EditAt(loaded_idx as i32, x, y))
}

/// Hands the oldest waiting editor update to `sink`: an image (PNG or JPEG) for the
/// rectangle x, y, rect_w, rect_h of a width x height picture.
/// The bytes are valid only during the call. Returns false when none waits.
#[no_mangle]
pub unsafe extern "C" fn ag_insert_next_screen(
    p: *mut c_void,
    sink: Option<
        unsafe extern "C" fn(ctx: *mut c_void, image: *const u8, len: u32, width: i32, height: i32, x: i32, y: i32, rect_w: i32, rect_h: i32),
    >,
    ctx: *mut c_void,
) -> bool {
    let Some(n) = native(p) else { return false };
    let Some(sink) = sink else { return false };
    let Some(frame) = n.live_ctrl.pop_screen() else { return false };
    // rect_w == 0: a whole-picture PNG drawn over the previous picture (AudioGridder diff).
    let (x, y, w, h) = frame.rect.unwrap_or((0, 0, 0, 0));
    sink(ctx, frame.pixels.as_ptr(), frame.pixels.len() as u32, frame.width, frame.height, x, y, w, h);
    true
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_editor_open(p: *mut c_void) -> bool {
    native(p).map(|n| n.live_ctrl.editor_open()).unwrap_or(false)
}

/// Server on this machine: its editor windows are on this screen (AudioGridder's local
/// mode), so the client shows no streamed copy.
#[no_mangle]
pub unsafe extern "C" fn ag_insert_server_is_local(p: *mut c_void) -> bool {
    let Some(n) = native(p) else { return true };
    let endpoint = n.live_ctrl.with_info(|info| info.endpoint.clone());
    let (host, _) = og_plugin::parse_server(&endpoint);
    let host = host.trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase();
    host.is_empty() || host == "localhost" || host == "::1" || host.starts_with("127.")
}

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn ag_insert_send_mouse(p: *mut c_void, kind: u8, x: f32, y: f32, shift: bool, ctrl: bool, alt: bool, dx: f32, dy: f32) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(og_plugin::live::LiveCommand::Mouse { kind, x, y, shift, ctrl, alt, dx, dy })
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_send_key(p: *mut c_void, codes: *const u16, count: u32) -> bool {
    let Some(n) = native(p) else { return false };
    if codes.is_null() || count == 0 {
        return false;
    }
    let codes = std::slice::from_raw_parts(codes, count as usize).to_vec();
    n.live_ctrl.send(og_plugin::live::LiveCommand::Key(codes))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_hide_plugin(p: *mut c_void) -> bool {
    let Some(n) = native(p) else { return false };
    n.live_ctrl.send(og_plugin::live::LiveCommand::Hide)
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
    n.live_ctrl.send(og_plugin::live::LiveCommand::Preset(slot_idx as i32, preset_idx as i32))
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
    n.live_ctrl.send(og_plugin::live::LiveCommand::Parameter(slot_idx as i32, param_idx as i32, val))
}

#[no_mangle]
pub unsafe extern "C" fn ag_insert_connect(p: *mut c_void, endpoint: *const c_char) -> bool {
    let Some(n) = native(p) else { return false };
    if endpoint.is_null() { return false };
    let ep = std::ffi::CStr::from_ptr(endpoint).to_string_lossy().trim().to_string();
    if ep.is_empty() {
        return false;
    }
    let (host, port) = og_plugin::parse_server(&ep);
    let ep = format!("{host}:{port}");
    // Remember it in the insert's state, so the session reopens on this server.
    if let Ok(mut control) = n.control.lock() {
        control.set_server(&ep);
    }
    if let Ok(mut endpoint) = n.endpoint.lock() {
        *endpoint = ep;
    }
    n.revision.fetch_add(1, Ordering::Relaxed);
    connect_now(n)
}

/// Re-fetch the server's plugin list, at most every 5 s (the plugin browser is opening).
#[no_mangle]
pub unsafe extern "C" fn ag_insert_refresh_catalog(p: *mut c_void) -> bool {
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    let Some(n) = native(p) else { return false };
    if let Ok(mut last) = LAST.lock() {
        if last.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(5)) {
            return false;
        }
        *last = Some(std::time::Instant::now());
    }
    n.live_ctrl.send(og_plugin::live::LiveCommand::RefreshCatalog)
}

/// The last error from the server connection (refused plugin, lost connection), or
/// false when there is none. Shown in the editor so a failure is never silent.
#[no_mangle]
pub unsafe extern "C" fn ag_insert_error(p: *mut c_void, buf: *mut c_char, cap: u32) -> bool {
    let Some(n) = native(p) else { return false };
    let (error, connected) = n.live_ctrl.with_info(|info| (info.error.clone(), info.connected));
    match error {
        Some(text) if !text.is_empty() => {
            copy_cstr(&text, buf, cap);
            true
        }
        _ if !connected => {
            copy_cstr("Not connected", buf, cap);
            true
        }
        _ => false,
    }
}

/// Connected to its server right now.
#[no_mangle]
pub unsafe extern "C" fn ag_insert_connected(p: *mut c_void) -> bool {
    native(p).map(|n| n.live_ctrl.connected()).unwrap_or(false)
}

/// The insert's server as host:port.
#[no_mangle]
pub unsafe extern "C" fn ag_insert_endpoint(p: *mut c_void, buf: *mut c_char, cap: u32) -> bool {
    let Some(n) = native(p) else { return false };
    let endpoint = n.endpoint.lock().map(|e| e.clone()).unwrap_or_default();
    copy_cstr(&endpoint, buf, cap);
    true
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
                &[og_plugin::QueuedMidi {
                    sample_offset: 7,
                    data: [0x92, 60, 100]
                }]
            );
            ag_insert_destroy(raw);
        }
    }
}
