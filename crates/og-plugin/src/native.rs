//! SDK-independent native FX runtime. All network operations and chain changes
//! run on a worker. The host callback only transfers preallocated audio slots
//! and reads/writes a timestamped output timeline; it never waits for the worker.
use crate::spsc::{self, Consumer, Producer};
use crate::{PluginMode, PluginProcessor};
use og_common::message::{MidiEvent, PositionInfo};
use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::ffi::{c_char, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct OgMidiEvent {
    pub sample_offset: i32,
    pub len: u8,
    pub data: [u8; 3],
}

pub enum NativeCommand {
    LoadPlugin { available_idx: usize, layout: String, x: i32, y: i32 },
    UnloadPlugin { loaded_idx: usize },
    SetPluginOversampling { loaded_idx: usize, factor: i32 },
    SetPluginBypassed { loaded_idx: usize, bypassed: bool },
    ExchangePlugins { idx_a: usize, idx_b: usize },
    SetPreset { loaded_idx: usize, preset_idx: usize },
    SetPluginParam { loaded_idx: usize, param_idx: i32, val: f32 },
    StoreA,
    StoreB,
    ToggleAB,
    ReconnectServer { host: String, port: u16 },
    FetchState { reply: mpsc::SyncSender<Vec<u8>> },
    RestoreState { data: Vec<u8> },
    EditPlugin { loaded_idx: usize, x: i32, y: i32 },
    HidePlugin,
    Mouse {
        ev_type: u8,
        x: f32,
        y: f32,
        shift: bool,
        ctrl: bool,
        alt: bool,
        delta_x: f32,
        delta_y: f32,
        is_smooth: bool,
    },
    Key {
        key_codes: Vec<u16>,
    },
}

#[derive(Clone, Default)]
pub struct ServerPluginMeta {
    pub id: String,
    pub name: String,
    pub category: String,
    pub manufacturer: String,
    pub is_instrument: bool,
    pub plugin_type: String,
}

#[derive(Clone, Default)]
pub struct LoadedPluginParamMeta {
    pub idx: i32,
    pub name: String,
    pub default_value: f32,
    pub current_value: f32,
}

#[derive(Clone, Default)]
pub struct LoadedPluginMeta {
    pub id: String,
    pub name: String,
    pub bypassed: bool,
    pub oversampling_factor: i32,
    pub latency: i32,
    pub ok: bool,
    pub presets: Vec<String>,
    pub params: Vec<LoadedPluginParamMeta>,
}

const POOL: usize = 8;
const MAX_MIDI_EVENTS: usize = 256;
const MAX_PLUGIN_LATENCY: usize = 65536;
const MAX_TRANSPORT_LATENCY: usize = 8192;
const TRANSPORT_BITS: u32 = 14;
const TRANSPORT_MASK: u32 = (1 << TRANSPORT_BITS) - 1;
const EMPTY: u64 = u64::MAX;

// One atomic word publishes a coherent transport/plugin latency pair. Use a
// 32-bit atomic so the same synchronization design is suitable for legacy x86.
fn latency_pair(transport: u32, plugin: u32) -> u32 {
    (plugin << TRANSPORT_BITS) | transport
}
fn transport_part(pair: u32) -> u32 {
    pair & TRANSPORT_MASK
}
fn plugin_part(pair: u32) -> u32 {
    pair >> TRANSPORT_BITS
}
fn total_latency(pair: u32) -> u32 {
    transport_part(pair) + plugin_part(pair)
}

struct Chunk {
    start: u64,
    frames: usize,
    audio: Vec<Vec<f32>>,
    midi_count: usize,
    midi: [OgMidiEvent; MAX_MIDI_EVENTS],
    valid: bool,
    selection: u32,
    latency: u32,
}

struct Shared {
    stop: AtomicBool,
    selected: AtomicU32,
    bypass: AtomicBool,
    profile: AtomicU32,
    input_gain: AtomicU32,
    wet_dry_mix: AtomicU32,
    drive: AtomicU32,
    drive_quality: AtomicU32,
    drive_char: AtomicU32,
    output_gain: AtomicU32,
    automation_slots: [AtomicU32; 16],
    meter_l: AtomicU32,
    meter_r: AtomicU32,
    meter_in_l: AtomicU32,
    meter_in_r: AtomicU32,
    active_is_b: AtomicBool,
    active_latency: AtomicU32,
    requested_latency: AtomicU32,
    /// Transport budget chosen by the latency profile, before any block-size headroom.
    base_transport: AtomicU32,
    processed: AtomicU32,
    status: AtomicI32,
    underruns: AtomicU32,
    names: Mutex<Vec<String>>,
    available_plugins: Mutex<Vec<ServerPluginMeta>>,
    loaded_plugins: Mutex<Vec<LoadedPluginMeta>>,
    active_server: Mutex<String>,
    discovered_servers: Mutex<Vec<String>>,
    cached_state: Mutex<Vec<u8>>,
    commands: Mutex<VecDeque<NativeCommand>>,
    chain_epoch: AtomicU32,
    screen_frame: Arc<parking_lot::Mutex<og_client::ScreenFrame>>,
    is_editor_open: AtomicBool,
    /// Set by the host adapter when it renders faster than real time (bounce).
    offline: AtomicBool,
    server_local_mode: AtomicBool,
}

impl Shared {
    // Worker-only update. Preserve any transport-budget request from the host callback.
    fn request_plugin_latency(&self, plugin: u32) {
        let _ =
            self.requested_latency
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    Some(latency_pair(
                        transport_part(current),
                        plugin.min(MAX_PLUGIN_LATENCY as u32),
                    ))
                });
    }

    fn restore_drive_modes(&self, j: &serde_json::Value) {
        if let Some(v) = j.get("driveQuality").and_then(|v| v.as_u64()) {
            self.drive_quality.store((v as u32).min(2), Ordering::Release);
        }
        if let Some(v) = j.get("driveCharacter").and_then(|v| v.as_u64()) {
            self.drive_char.store((v as u32).min(2), Ordering::Release);
        }
    }

    fn build_full_state(&self, processor: &mut PluginProcessor) -> Vec<u8> {
        let raw = processor.get_state_information();
        let mut root: serde_json::Value = serde_json::from_slice(&raw)
            .unwrap_or_else(|_| serde_json::json!({ "version": 7, "loadedPlugins": [] }));
        if let Some(obj) = root.as_object_mut() {
            obj.insert("bypass".into(), serde_json::json!(self.bypass.load(Ordering::Acquire)));
            obj.insert("selectedPlugin".into(), serde_json::json!(self.selected.load(Ordering::Acquire)));
            obj.insert("latencyProfile".into(), serde_json::json!(self.profile.load(Ordering::Acquire)));
            obj.insert("inputGain".into(), serde_json::json!(f32::from_bits(self.input_gain.load(Ordering::Acquire))));
            obj.insert("wetDryMix".into(), serde_json::json!(f32::from_bits(self.wet_dry_mix.load(Ordering::Acquire))));
            obj.insert("drive".into(), serde_json::json!(f32::from_bits(self.drive.load(Ordering::Acquire))));
            obj.insert("driveQuality".into(), serde_json::json!(self.drive_quality.load(Ordering::Acquire)));
            obj.insert("driveCharacter".into(), serde_json::json!(self.drive_char.load(Ordering::Acquire)));
            obj.insert("outputGain".into(), serde_json::json!(f32::from_bits(self.output_gain.load(Ordering::Acquire))));
        }
        let bytes = serde_json::to_vec(&root).unwrap_or(raw);
        *self.cached_state.lock().unwrap_or_else(|e| e.into_inner()) = bytes.clone();
        bytes
    }

    fn sync_loaded_meta(&self, processor: &PluginProcessor) {
        *self.loaded_plugins.lock().unwrap_or_else(|e| e.into_inner()) = processor
            .loaded_plugins
            .iter()
            .map(|p| {
                let mut params: Vec<LoadedPluginParamMeta> = p
                    .params
                    .values()
                    .map(|prm| LoadedPluginParamMeta {
                        idx: prm.idx,
                        name: prm.name.clone(),
                        default_value: prm.default_value,
                        current_value: prm.current_value,
                    })
                    .collect();
                params.sort_by_key(|prm| prm.idx);
                LoadedPluginMeta {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    bypassed: p.bypassed,
                    oversampling_factor: p.oversampling_factor,
                    latency: p.latency,
                    ok: p.ok,
                    presets: p.presets.clone(),
                    params,
                }
            })
            .collect();
    }
}

struct Callback {
    send: Producer<Box<Chunk>>,
    receive: Consumer<Box<Chunk>>,
    free: Vec<Box<Chunk>>,
    clock: u64,
    dry: Vec<Vec<f32>>,
    wet: Vec<Vec<f32>>,
    dry_tags: Vec<u64>,
    wet_tags: Vec<u64>,
    pending_midi_count: usize,
    pending_midi: [OgMidiEvent; MAX_MIDI_EVENTS],
    selection: u32,
    plugin_latency: usize,
    transport_latency: usize,
    drive: crate::drive::DriveStage,
    drive_latency: usize,
    smooth_a: f32,
    sm_in: f32,
    sm_mix: f32,
    sm_drive: f32,
    sm_out: f32,
    g_in: Vec<f32>,
    g_mix: Vec<f32>,
    g_drive: Vec<f32>,
    g_out: Vec<f32>,
    /// Clock value at which the current wet/dry alignment started (reset on any re-alignment).
    epoch_start: u64,
    last_callback: Option<std::time::Instant>,
    fast_callbacks: u32,
    slow_callbacks: u32,
    auto_offline: bool,
    wait_timeouts: u32,
    /// Largest block seen since the profile last changed (block-size headroom never shrinks mid-stream).
    seen_frames: usize,
    seen_base: u32,
}

/// Opaque handle. Only one audio thread may process it at a time; control
/// functions only access atomics/metadata. Destruction requires processing to stop.
pub struct NativePlugin {
    shared: Arc<Shared>,
    audio: UnsafeCell<Callback>,
    worker: Option<JoinHandle<()>>,
    channels: usize,
    max_frames: usize,
    rate: f64,
}

impl NativePlugin {
    fn new(
        rate: f64,
        max_frames: usize,
        channels: usize,
        latency: usize,
        endpoint: Option<(String, u16)>,
    ) -> Self {
        let (send, mut incoming) = spsc::channel::<Box<Chunk>>(POOL);
        let (mut outgoing, receive) = spsc::channel::<Box<Chunk>>(POOL);
        let free = (0..POOL)
            .map(|_| {
                Box::new(Chunk {
                    start: 0,
                    frames: 0,
                    audio: vec![vec![0.0; max_frames]; channels],
                    midi_count: 0,
                    midi: [OgMidiEvent::default(); MAX_MIDI_EVENTS],
                    valid: false,
                    selection: 0,
                    latency: 0,
                })
            })
            .collect();
        let initial_srv = endpoint
            .as_ref()
            .map(|(h, p)| format!("{}:{}", h, p))
            .unwrap_or_else(|| "127.0.0.1:55056".to_string());
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            selected: AtomicU32::new(0),
            bypass: AtomicBool::new(false),
            profile: AtomicU32::new(1),
            input_gain: AtomicU32::new(0.5f32.to_bits()),
            wet_dry_mix: AtomicU32::new(1.0f32.to_bits()),
            drive: AtomicU32::new(0.0f32.to_bits()),
            drive_quality: AtomicU32::new(crate::drive::QUALITY_ADAA),
            drive_char: AtomicU32::new(crate::drive::CHAR_CLEAN),
            output_gain: AtomicU32::new(0.5f32.to_bits()),
            automation_slots: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            meter_l: AtomicU32::new(0.0f32.to_bits()),
            meter_r: AtomicU32::new(0.0f32.to_bits()),
            meter_in_l: AtomicU32::new(0.0f32.to_bits()),
            meter_in_r: AtomicU32::new(0.0f32.to_bits()),
            active_is_b: AtomicBool::new(false),
            active_latency: AtomicU32::new(latency_pair(latency as u32, 0)),
            requested_latency: AtomicU32::new(latency_pair(latency as u32, 0)),
            base_transport: AtomicU32::new((latency as u32).clamp(32, 8192)),
            processed: AtomicU32::new(0),
            status: AtomicI32::new(0),
            underruns: AtomicU32::new(0),
            names: Mutex::new(Vec::new()),
            available_plugins: Mutex::new(Vec::new()),
            loaded_plugins: Mutex::new(Vec::new()),
            active_server: Mutex::new(initial_srv.clone()),
            discovered_servers: Mutex::new(vec![initial_srv]),
            cached_state: Mutex::new(Vec::new()),
            commands: Mutex::new(VecDeque::new()),
            chain_epoch: AtomicU32::new(0),
            screen_frame: Arc::new(parking_lot::Mutex::new(og_client::ScreenFrame::default())),
            is_editor_open: AtomicBool::new(false),
            offline: AtomicBool::new(false),
            server_local_mode: AtomicBool::new(false),
        });
        let capacity = MAX_PLUGIN_LATENCY + MAX_TRANSPORT_LATENCY + max_frames + 1;
        let mut this = Self {
            shared: shared.clone(),
            channels,
            max_frames,
            rate,
            audio: UnsafeCell::new(Callback {
                send,
                receive,
                free,
                clock: 0,
                dry: vec![vec![0.0; capacity]; channels],
                wet: vec![vec![0.0; capacity]; channels],
                dry_tags: vec![EMPTY; capacity],
                wet_tags: vec![EMPTY; capacity],
                pending_midi_count: 0,
                pending_midi: [OgMidiEvent::default(); MAX_MIDI_EVENTS],
                selection: 0,
                plugin_latency: 0,
                transport_latency: latency,
                drive: crate::drive::DriveStage::new(channels, rate),
                drive_latency: 0,
                smooth_a: 1.0 - (-1.0 / (0.010 * rate.max(8000.0))).exp() as f32,
                sm_in: 1.0,
                sm_mix: 1.0,
                sm_drive: 0.0,
                sm_out: 1.0,
                g_in: vec![1.0; max_frames],
                g_mix: vec![1.0; max_frames],
                g_drive: vec![0.0; max_frames],
                g_out: vec![1.0; max_frames],
                epoch_start: 0,
                last_callback: None,
                fast_callbacks: 0,
                slow_callbacks: 0,
                auto_offline: false,
                wait_timeouts: 0,
                seen_frames: 0,
                seen_base: 0,
            }),
            worker: None,
        };
        if let Some((mut host, mut port)) = endpoint {
            this.worker = Some(
                thread::Builder::new()
                    .name("OpenGridder native IO".into())
                    .spawn(move || {
                        og_common::realtime::prioritize_audio_thread();
                        let mut processor = PluginProcessor::new(PluginMode::Fx);
                        processor.prepare_to_play(rate, max_frames.min(1024));
                        let mut cfg = processor.client.config();
                        cfg.channels_in = channels as i32;
                        cfg.channels_out = channels as i32;
                        cfg.active_channels = (1 << channels) - 1;
                        processor.client.set_config(cfg);
                        processor.client.set_screen_frame_store(shared.screen_frame.clone());
                        let _ = shared.build_full_state(&mut processor);
                        let mut retry = Instant::now();
                        let mut applied = u32::MAX;
                        let mut stream_errors = 0;
                        while !shared.stop.load(Ordering::Acquire) {
                            if !processor.client.is_ready() && Instant::now() >= retry {
                                shared.status.store(1, Ordering::Release);
                                match processor.connect_to_server(&host, port) {
                                    Ok(_) => {
                                        processor.client.set_screen_frame_store(shared.screen_frame.clone());
                                        shared.server_local_mode.store(processor.client.is_server_local_mode(), Ordering::Release);
                                        let srv_plugins = processor.client.available_plugins();
                                        eprintln!("[OpenGridder native IO] Connected to {}:{}! Available plugins: {}", host, port, srv_plugins.len());
                                        *shared.names.lock().unwrap_or_else(|e| e.into_inner()) =
                                            srv_plugins.iter().map(|p| p.name.clone()).collect();
                                        *shared.available_plugins.lock().unwrap_or_else(|e| e.into_inner()) =
                                            srv_plugins.iter().map(|p| ServerPluginMeta {
                                                id: p.id.clone(),
                                                name: p.name.clone(),
                                                category: p.category.clone(),
                                                manufacturer: p.company.clone(),
                                                is_instrument: p.is_instrument,
                                                plugin_type: p.plugin_type.clone(),
                                            }).collect();
                                        applied = if shared.selected.load(Ordering::Acquire) > 0 { u32::MAX } else { 0 };
                                        shared.status.store(2, Ordering::Release);
                                    }
                                    Err(e) => {
                                        eprintln!("[OpenGridder native IO] Connect to {}:{} failed: {}", host, port, e);
                                        shared.status.store(0, Ordering::Release);
                                    }
                                }
                                retry = Instant::now() + Duration::from_secs(1);
                            }

                            // Process queued commands from UI/host
                            let cmds: Vec<NativeCommand> = {
                                let mut q = shared.commands.lock().unwrap_or_else(|e| e.into_inner());
                                q.drain(..).collect()
                            };
                            let mut chain_changed = false;
                            for cmd in cmds {
                                match cmd {
                                    NativeCommand::LoadPlugin { available_idx, layout, x, y } => {
                                        let list = processor.client.available_plugins();
                                        if let Some(desc) = list.get(available_idx) {
                                            match processor.load_plugin(desc, &layout) {
                                                Ok(loaded_slot) => {
                                                    shared.request_plugin_latency(processor.latency_samples.max(0) as u32);
                                                    chain_changed = true;
                                                    let _ = processor.client.edit_plugin(loaded_slot as i32, x, y);
                                                }
                                                Err(e) => {
                                                    eprintln!("[OpenGridder native IO] load_plugin({:?}) failed: {}", desc.name, e);
                                                }
                                            }
                                        }
                                    }
                                    NativeCommand::UnloadPlugin { loaded_idx } => {
                                        if processor.unload_plugin(loaded_idx).is_ok() {
                                            shared.request_plugin_latency(processor.latency_samples.max(0) as u32);
                                            chain_changed = true;
                                        }
                                    }
                                    NativeCommand::SetPluginOversampling { loaded_idx, factor } => {
                                        if processor.set_plugin_oversampling(loaded_idx, factor).is_ok() {
                                            shared.request_plugin_latency(processor.latency_samples.max(0) as u32);
                                            chain_changed = true;
                                        }
                                    }
                                    NativeCommand::SetPluginBypassed { loaded_idx, bypassed } => {
                                        if bypassed {
                                            let _ = processor.bypass_plugin(loaded_idx);
                                        } else {
                                            let _ = processor.unbypass_plugin(loaded_idx);
                                        }
                                        chain_changed = true;
                                    }
                                    NativeCommand::ExchangePlugins { idx_a, idx_b } => {
                                        if processor.exchange_plugins(idx_a, idx_b).is_ok() {
                                            chain_changed = true;
                                        }
                                    }
                                    NativeCommand::SetPreset { loaded_idx, preset_idx } => {
                                        let _ = processor.client.set_preset(loaded_idx as i32, preset_idx as i32);
                                        chain_changed = true;
                                    }
                                    NativeCommand::SetPluginParam { loaded_idx, param_idx, val } => {
                                        if let Some(lp) = processor.loaded_plugins.get_mut(loaded_idx) {
                                            if let Some(prm) = lp.params.get_mut(&param_idx) {
                                                prm.current_value = val;
                                            }
                                        }
                                        let _ = processor.client.set_parameter_value(loaded_idx as i32, param_idx, val);
                                        chain_changed = true;
                                    }
                                    NativeCommand::StoreA => {
                                        processor.settings_a = String::from_utf8_lossy(&shared.build_full_state(&mut processor)).into_owned();
                                    }
                                    NativeCommand::StoreB => {
                                        processor.settings_b = String::from_utf8_lossy(&shared.build_full_state(&mut processor)).into_owned();
                                    }
                                    NativeCommand::ToggleAB => {
                                        if processor.active_is_b {
                                            if processor.settings_b.is_empty() {
                                                processor.settings_b = String::from_utf8_lossy(&shared.build_full_state(&mut processor)).into_owned();
                                            }
                                            processor.active_is_b = false;
                                            let target = processor.settings_a.clone();
                                            if !target.is_empty() {
                                                let _ = processor.set_state_information(target.as_bytes());
                                                if let Ok(j) = serde_json::from_str::<serde_json::Value>(&target) {
                                                    if let Some(v) = j.get("inputGain").and_then(|v| v.as_f64()) {
                                                        shared.input_gain.store((v as f32).clamp(0.0, 1.0).to_bits(), Ordering::Release);
                                                    }
                                                    if let Some(v) = j.get("wetDryMix").and_then(|v| v.as_f64()) {
                                                        shared.wet_dry_mix.store((v as f32).clamp(0.0, 1.0).to_bits(), Ordering::Release);
                                                    }
                                                    if let Some(v) = j.get("drive").and_then(|v| v.as_f64()) {
                                                        shared.drive.store((v as f32).clamp(0.0, 1.0).to_bits(), Ordering::Release);
                                                    }
                                                    shared.restore_drive_modes(&j);
if let Some(v) = j.get("outputGain").and_then(|v| v.as_f64()) {
                                                        shared.output_gain.store((v as f32).clamp(0.0, 1.0).to_bits(), Ordering::Release);
                                                    }
                                                }
                                            }
                                        } else {
                                            if processor.settings_a.is_empty() {
                                                processor.settings_a = String::from_utf8_lossy(&shared.build_full_state(&mut processor)).into_owned();
                                            }
                                            processor.active_is_b = true;
                                            let target = processor.settings_b.clone();
                                            if !target.is_empty() {
                                                let _ = processor.set_state_information(target.as_bytes());
                                                if let Ok(j) = serde_json::from_str::<serde_json::Value>(&target) {
                                                    if let Some(v) = j.get("inputGain").and_then(|v| v.as_f64()) {
                                                        shared.input_gain.store((v as f32).clamp(0.0, 1.0).to_bits(), Ordering::Release);
                                                    }
                                                    if let Some(v) = j.get("wetDryMix").and_then(|v| v.as_f64()) {
                                                        shared.wet_dry_mix.store((v as f32).clamp(0.0, 1.0).to_bits(), Ordering::Release);
                                                    }
                                                    if let Some(v) = j.get("drive").and_then(|v| v.as_f64()) {
                                                        shared.drive.store((v as f32).clamp(0.0, 1.0).to_bits(), Ordering::Release);
                                                    }
                                                    shared.restore_drive_modes(&j);
if let Some(v) = j.get("outputGain").and_then(|v| v.as_f64()) {
                                                        shared.output_gain.store((v as f32).clamp(0.0, 1.0).to_bits(), Ordering::Release);
                                                    }
                                                }
                                            }
                                        }
                                        shared.active_is_b.store(processor.active_is_b, Ordering::Release);
                                        shared.request_plugin_latency(processor.latency_samples.max(0) as u32);
                                        chain_changed = true;
                                    }
                                    NativeCommand::ReconnectServer { host: new_host, port: new_port } => {
                                        processor.client.disconnect();
                                        host = new_host;
                                        port = new_port;
                                        processor.active_server = format!("{}:{}", host, port);
                                        *shared.active_server.lock().unwrap_or_else(|e| e.into_inner()) = processor.active_server.clone();
                                        retry = Instant::now();
                                        shared.status.store(1, Ordering::Release);
                                    }
                                    NativeCommand::FetchState { reply } => {
                                        let bytes = shared.build_full_state(&mut processor);
                                        let _ = reply.send(bytes);
                                    }
                                    NativeCommand::RestoreState { data } => {
                                        if processor.set_state_information(&data).is_ok() {
                                            shared.request_plugin_latency(processor.latency_samples.max(0) as u32);
                                            chain_changed = true;
                                        }
                                    }
                                    NativeCommand::EditPlugin { loaded_idx, x, y } => {
                                        eprintln!("[OpenGridder native IO] executing EditPlugin idx={} x={} y={}", loaded_idx, x, y);
                                        shared.is_editor_open.store(true, Ordering::Release);
                                        match processor.client.edit_plugin(loaded_idx as i32, x, y) {
                                            Ok(_) => eprintln!("[OpenGridder native IO] client.edit_plugin sent successfully!"),
                                            Err(e) => eprintln!("[OpenGridder native IO] client.edit_plugin failed: {}", e),
                                        }
                                    }
                                    NativeCommand::HidePlugin => {
                                        shared.is_editor_open.store(false, Ordering::Release);
                                        let _ = processor.client.hide_plugin();
                                    }
                                    NativeCommand::Mouse {
                                        ev_type,
                                        x,
                                        y,
                                        shift,
                                        ctrl,
                                        alt,
                                        delta_x,
                                        delta_y,
                                        is_smooth,
                                    } => {
                                        let _ = processor.client.send_mouse_event(
                                            og_common::key_and_mouse::MouseEvType::from_u8(ev_type),
                                            x,
                                            y,
                                            shift,
                                            ctrl,
                                            alt,
                                            delta_x,
                                            delta_y,
                                            is_smooth,
                                        );
                                    }
                                    NativeCommand::Key { key_codes } => {
                                        let _ = processor.client.send_key_event(&key_codes);
                                    }
                                }
                            }

                            let selected = shared.selected.load(Ordering::Acquire);
                            if processor.client.is_ready() && applied != selected && selected > 0 {
                                let list = processor.client.available_plugins();
                                if let Some(desc) = list.get(selected as usize - 1) {
                                    let _ = processor.load_plugin(
                                        desc,
                                        if channels == 1 { "Mono" } else { "Stereo" },
                                    );
                                    shared.request_plugin_latency(
                                        processor.latency_samples.max(0) as u32
                                    );
                                    chain_changed = true;
                                }
                                applied = selected;
                            }

                            if chain_changed {
                                shared.chain_epoch.fetch_add(1, Ordering::Release);
                                shared.sync_loaded_meta(&processor);
                                let _ = shared.build_full_state(&mut processor);
                                shared.status.store(
                                    if processor.loaded_plugins.is_empty() { 2 } else { 3 },
                                    Ordering::Release,
                                );
                            }

                            if let Some(mut chunk) = incoming.pop() {
                                chunk.valid = false;
                                let is_ready = processor.client.is_ready();
                                let has_plugins = !processor.loaded_plugins.is_empty();
                                if is_ready && has_plugins {
                                    for channel in &mut chunk.audio {
                                        channel.truncate(chunk.frames);
                                    }
                                    let mut midi = Vec::with_capacity(chunk.midi_count);
                                    for i in 0..chunk.midi_count {
                                        let ev = &chunk.midi[i];
                                        let n = (ev.len as usize).min(3);
                                        if n > 0 {
                                            midi.push(MidiEvent {
                                                sample_number: ev.sample_offset,
                                                data: ev.data[..n].to_vec(),
                                            });
                                        }
                                    }
                                    match processor.client.process_audio_f32(
                                        &mut chunk.audio,
                                        &mut midi,
                                        &PositionInfo::default(),
                                    ) {
                                        Ok(delay)
                                            if delay >= 0
                                                && (delay as usize) <= MAX_PLUGIN_LATENCY =>
                                        {
                                            shared.request_plugin_latency(delay as u32);
                                            chunk.valid = true;
                                            shared.processed.fetch_add(1, Ordering::Relaxed);
                                            stream_errors = 0;
                                        }
                                        _ => {
                                            stream_errors += 1;
                                            if stream_errors >= 5 {
                                                eprintln!("[OpenGridder native IO] 5 consecutive audio stream errors, resetting client socket");
                                                processor.client.disconnect();
                                                shared.status.store(0, Ordering::Release);
                                                stream_errors = 0;
                                            }
                                        }
                                    }
                                    for channel in &mut chunk.audio {
                                        channel.resize(max_frames, 0.0);
                                    }
                                }
                                chunk.midi_count = 0;
                                // A pool slot is always in exactly one of the queues or endpoint
                                // owners. Therefore a POOL-sized return queue has room here.
                                if outgoing.push(chunk).is_err() {
                                    break;
                                }
                            } else {
                                thread::sleep(Duration::from_micros(200));
                            }
                        }
                        processor.client.disconnect();
                    })
                    .expect("native worker creation failed"),
            );
        }
        this
    }

    /// Move every returned chunk into the wet ring and recycle it.
    fn absorb(
        cb: &mut Callback,
        channels: usize,
        selection: u32,
        transport_delay: usize,
        drive_lat: usize,
    ) {
        let size = cb.dry_tags.len();
        for _ in 0..POOL {
            let Some(chunk) = cb.receive.pop() else {
                break;
            };
            if chunk.valid && (chunk.selection == selection || selection == 0) {
                let chunk_transport = transport_part(chunk.latency) as u64;
                let effective_transport = if chunk_transport > 0 { chunk_transport } else { transport_delay as u64 };
                for n in 0..chunk.frames {
                    let target = chunk.start + n as u64 + effective_transport.saturating_sub(drive_lat as u64);
                    if target >= cb.clock && target < cb.clock + size as u64 {
                        let index = target as usize % size;
                        for ch in 0..channels {
                            cb.wet[ch][index] = chunk.audio[ch][n];
                        }
                        cb.wet_tags[index] = target;
                    }
                }
            }
            cb.free.push(chunk);
        }
    }

    unsafe fn process(
        &self,
        inputs: *const *const f32,
        outputs: *const *mut f32,
        frames: usize,
    ) -> i32 {
        if frames == 0 {
            return 0;
        }
        if frames > self.max_frames || inputs.is_null() || outputs.is_null() {
            return -1;
        }
        let input = std::slice::from_raw_parts(inputs, self.channels);
        let output = std::slice::from_raw_parts(outputs, self.channels);
        if input.iter().any(|p| p.is_null()) || output.iter().any(|p| p.is_null()) {
            return -1;
        }
        // SAFETY: the adapter guarantees exactly one callback at a time.
        let cb = &mut *self.audio.get();
        let size = cb.dry_tags.len();
        let selection = self.shared.selected.load(Ordering::Acquire);
        let requested = self.shared.requested_latency.load(Ordering::Acquire);
        // The profile is the latency budget. A block cannot come back before it has been
        // sent, so when the host block is larger than the budget the budget would be spent
        // on the block itself; keep one block of transit plus the whole profile as headroom.
        let base = self.shared.base_transport.load(Ordering::Acquire);
        if cb.seen_base != base {
            cb.seen_base = base;
            cb.seen_frames = 0;
        }
        cb.seen_frames = cb.seen_frames.max(frames);
        let wanted = if cb.seen_frames > base as usize {
            ((cb.seen_frames.next_power_of_two() as u32).saturating_add(base)).min(MAX_TRANSPORT_LATENCY as u32)
        } else {
            base
        };
        if wanted != transport_part(requested) {
            // At most one CAS attempt per callback: a racing worker update can
            // defer the request to the next callback, never make this callback wait.
            let _ = self.shared.requested_latency.compare_exchange(
                requested,
                latency_pair(wanted, plugin_part(requested)),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }

        // Local console DSP parameters (lock-free atomics)
        let in_norm = f32::from_bits(self.shared.input_gain.load(Ordering::Relaxed));
        let mix_norm = f32::from_bits(self.shared.wet_dry_mix.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let drive_norm = f32::from_bits(self.shared.drive.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let out_norm = f32::from_bits(self.shared.output_gain.load(Ordering::Relaxed));

        let in_lin = if (in_norm - 0.5).abs() < 1e-6 {
            1.0f32
        } else {
            10.0f32.powf((in_norm - 0.5) * 48.0 / 20.0)
        };
        let out_lin = if (out_norm - 0.5).abs() < 1e-6 {
            1.0f32
        } else {
            10.0f32.powf((out_norm - 0.5) * 48.0 / 20.0)
        };
        let in_target = in_lin;
        let out_target = out_lin;
        let drive_quality = self.shared.drive_quality.load(Ordering::Relaxed).min(2);
        let drive_char = self.shared.drive_char.load(Ordering::Relaxed).min(2);
        let drive_lat = crate::drive::quality_latency(drive_quality);

        // Per-sample one-pole smoothing (~10 ms) of every console parameter: no zipper noise.
        {
            let a = cb.smooth_a;
            let (mut si, mut sm, mut sd, mut so) = (cb.sm_in, cb.sm_mix, cb.sm_drive, cb.sm_out);
            for n in 0..frames {
                si += a * (in_target - si);
                sm += a * (mix_norm - sm);
                sd += a * (drive_norm - sd);
                so += a * (out_target - so);
                cb.g_in[n] = si;
                cb.g_mix[n] = sm;
                cb.g_drive[n] = sd;
                cb.g_out[n] = so;
            }
            cb.sm_in = si;
            cb.sm_mix = sm;
            cb.sm_drive = sd;
            cb.sm_out = so;
        }

        let active = self.shared.active_latency.load(Ordering::Acquire);
        let plugin_delay = plugin_part(active) as usize;
        let transport_delay = transport_part(active) as usize;
        if cb.selection != selection
            || cb.plugin_latency != plugin_delay
            || cb.transport_latency != transport_delay
            || cb.drive_latency != drive_lat
        {
            cb.drive_latency = drive_lat;
            cb.wet_tags.fill(EMPTY);
            cb.dry_tags.fill(EMPTY);
            cb.epoch_start = cb.clock;
            cb.wait_timeouts = 0;
            cb.selection = selection;
            cb.plugin_latency = plugin_delay;
            cb.transport_latency = transport_delay;
        }
        Self::absorb(cb, self.channels, selection, transport_delay, drive_lat);
        if let Some(mut chunk) = cb.free.pop() {
            chunk.start = cb.clock;
            chunk.frames = frames;
            chunk.selection = selection;
            chunk.latency = active;
            chunk.midi_count = cb.pending_midi_count;
            if cb.pending_midi_count > 0 {
                chunk.midi[..cb.pending_midi_count]
                    .copy_from_slice(&cb.pending_midi[..cb.pending_midi_count]);
                cb.pending_midi_count = 0;
            }
            for ch in 0..self.channels {
                for n in 0..frames {
                    chunk.audio[ch][n] = *input[ch].add(n) * cb.g_in[n];
                }
            }
            if let Err(chunk) = cb.send.push(chunk) {
                cb.free.push(chunk);
            }
        }
        // Offline bounce renders faster than real time: waiting for the server keeps the
        // wet signal aligned instead of falling back to delayed dry audio.
        let block_secs = frames as f64 / self.rate.max(8000.0);
        let now_instant = std::time::Instant::now();
        if let Some(prev) = cb.last_callback {
            let gap = now_instant.duration_since(prev).as_secs_f64();
            if gap < block_secs * 0.3 {
                cb.fast_callbacks = cb.fast_callbacks.saturating_add(1);
                cb.slow_callbacks = 0;
            } else if gap > block_secs * 0.8 {
                cb.slow_callbacks = cb.slow_callbacks.saturating_add(1);
                cb.fast_callbacks = 0;
            }
            if cb.fast_callbacks >= 4 {
                cb.auto_offline = true;
            } else if cb.slow_callbacks >= 8 {
                cb.auto_offline = false;
            }
        }
        cb.last_callback = Some(now_instant);
        let offline = self.shared.offline.load(Ordering::Acquire) || cb.auto_offline;
        if offline
            && !self.shared.bypass.load(Ordering::Acquire)
            && self.shared.status.load(Ordering::Acquire) == 3
            && cb.wait_timeouts < 3
        {
            let last = cb.clock + frames as u64 - 1;
            let lead = transport_delay.saturating_sub(drive_lat) as u64;
            if last >= cb.epoch_start + lead {
                let deadline = now_instant + std::time::Duration::from_millis(250);
                let index = last as usize % size;
                while cb.wet_tags[index] != last {
                    if std::time::Instant::now() >= deadline {
                        cb.wait_timeouts += 1;
                        break;
                    }
                    Self::absorb(cb, self.channels, selection, transport_delay, drive_lat);
                    if cb.wet_tags[index] != last {
                        std::thread::yield_now();
                    }
                }
            }
        }
        let bypass = self.shared.bypass.load(Ordering::Acquire);
        let mut missed = false;
        let mut peak_l = 0.0f32;
        let mut peak_r = 0.0f32;
        let mut peak_in_l = 0.0f32;
        let mut peak_in_r = 0.0f32;
        for n in 0..frames {
            let now = cb.clock + n as u64;
            let target = now + (transport_delay + plugin_delay).saturating_sub(drive_lat) as u64;
            let write = target as usize % size;
            let read = now as usize % size;
            for ch in 0..self.channels {
                let in_sample = *input[ch].add(n) * cb.g_in[n];
                cb.dry[ch][write] = in_sample;
                let ia = in_sample.abs();
                if ch == 0 {
                    if ia > peak_in_l { peak_in_l = ia; }
                } else if ia > peak_in_r {
                    peak_in_r = ia;
                }
            }
            if self.channels == 1 {
                peak_in_r = peak_in_l;
            }
            cb.dry_tags[write] = target;
            let wet = !bypass && cb.wet_tags[read] == now;
            let dry = cb.dry_tags[read] == now;
            let mix_n = cb.g_mix[n];
            let drive_n = cb.g_drive[n];
            let out_n = cb.g_out[n];
            let drive_active = drive_n > 1e-4;
            let shape = crate::drive::ShapeParams::new(1.0 + drive_n * 9.0, drive_char);
            for ch in 0..self.channels {
                let mut sample = if bypass {
                    if dry { cb.dry[ch][read] } else { 0.0 }
                } else if wet {
                    let w = cb.wet[ch][read];
                    if (mix_n - 1.0).abs() < 1e-6 || !dry {
                        w
                    } else {
                        let d = cb.dry[ch][read];
                        d * (1.0 - mix_n) + w * mix_n
                    }
                } else if dry {
                    cb.dry[ch][read]
                } else {
                    0.0
                };
                if !bypass {
                    sample = cb.drive.process(ch, sample, &shape, drive_quality, drive_char, drive_active);
                    sample *= out_n;
                } else {
                    sample = cb.drive.delay_only(ch, sample, drive_quality);
                }
                *output[ch].add(n) = sample;
                let a = sample.abs();
                if ch == 0 {
                    if a > peak_l { peak_l = a; }
                } else if a > peak_r {
                    peak_r = a;
                }
            }
            if self.channels == 1 {
                peak_r = peak_l;
            }
            missed |= !wet && !bypass;
            cb.wet_tags[read] = EMPTY;
            cb.dry_tags[read] = EMPTY;
        }
        self.shared.meter_l.store(peak_l.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
        self.shared.meter_r.store(peak_r.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
        self.shared.meter_in_l.store(peak_in_l.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
        self.shared.meter_in_r.store(peak_in_r.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
        cb.clock += frames as u64;
        if missed && self.shared.status.load(Ordering::Acquire) == 3 {
            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
        0
    }
}

impl Drop for NativePlugin {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Read only on a control thread. A file lets GUI-launched DAWs configure the
/// remote endpoint without inheriting shell environment variables.
#[derive(serde::Deserialize)]
#[serde(default)]
struct NativeConfiguration {
    server: String,
    transport_samples: u32,
}
impl Default for NativeConfiguration {
    fn default() -> Self {
        Self {
            server: "127.0.0.1:55056".into(),
            transport_samples: 256,
        }
    }
}
fn native_configuration() -> NativeConfiguration {
    let mut config = NativeConfiguration::default();
    let path = std::env::var_os("OPENGRIDDER_CLIENT_CONFIG")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| {
                std::path::PathBuf::from(home)
                    .join("Library/Application Support/OpenGridder/client.json")
            })
        });
    if let Some(path) = path {
        if path.metadata().map(|m| m.len() <= 65536).unwrap_or(false) {
            if let Ok(bytes) = std::fs::read(&path) {
                match serde_json::from_slice(&bytes) {
                    Ok(value) => config = value,
                    Err(error) => log::warn!("Invalid OpenGridder client configuration: {error}"),
                }
            }
        }
    }
    if let Ok(endpoint) = std::env::var("OPENGRIDDER_SERVER") {
        config.server = endpoint;
    }
    if let Ok(samples) = std::env::var("OPENGRIDDER_TRANSPORT_SAMPLES") {
        if let Ok(samples) = samples.parse() {
            config.transport_samples = samples;
        }
    }
    if !(32..=8192).contains(&config.transport_samples) {
        config.transport_samples = 256;
    }
    config
}
/// Preferred initial transport budget. Call on a control thread before creation.
#[unsafe(no_mangle)]
pub extern "C" fn og_native_default_latency() -> u32 {
    native_configuration().transport_samples
}

/// Create on a control thread. Supports mono/stereo FX and up to 8192 frames.
#[unsafe(no_mangle)]
pub extern "C" fn og_native_create(
    rate: f64,
    max_frames: u32,
    channels: u32,
    latency: u32,
) -> *mut NativePlugin {
    if !rate.is_finite()
        || rate < 8000.0
        || rate > 768000.0
        || !(1..=8192).contains(&max_frames)
        || !(1..=2).contains(&channels)
        || !(32..=8192).contains(&latency)
    {
        return std::ptr::null_mut();
    }
    let endpoint = native_configuration().server;
    let endpoint = endpoint
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().ok().map(|p| (host.to_owned(), p)))
        .unwrap_or_else(|| ("127.0.0.1".into(), 55056));
    catch_unwind(|| {
        Box::into_raw(Box::new(NativePlugin::new(
            rate,
            max_frames as usize,
            channels as usize,
            latency as usize,
            Some(endpoint),
        )))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// Handle must be live and not concurrently processed or destroyed. Buffer arrays
/// contain the configured number of valid planar channels, each at least frames long.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_process(
    handle: *mut NativePlugin,
    inputs: *const *const f32,
    outputs: *const *mut f32,
    frames: u32,
) -> i32 {
    let Some(plugin) = handle.as_ref() else {
        return -1;
    };
    catch_unwind(AssertUnwindSafe(|| {
        plugin.process(inputs, outputs, frames as usize)
    }))
    .unwrap_or(-2)
}

/// # Safety
/// Valid uniquely-owned handle; all callbacks/control users must have stopped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_destroy(handle: *mut NativePlugin) {
    if !handle.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(Box::from_raw(handle))));
    }
}

/// Tell the plugin the host is rendering offline (bounce), so it waits for the server.
///
/// # Safety
/// Live handle, not concurrently destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_set_offline(handle: *mut NativePlugin, offline: bool) {
    if let Some(p) = handle.as_ref() {
        p.shared.offline.store(offline, Ordering::Release);
    }
}

/// # Safety
/// Live handle, not concurrently destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_select(handle: *mut NativePlugin, index: u32) {
    if let Some(p) = handle.as_ref() {
        p.shared.selected.store(index.min(128), Ordering::Release);
    }
}
/// # Safety
/// Live handle, not concurrently destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_bypass(handle: *mut NativePlugin, bypass: bool) {
    if let Some(p) = handle.as_ref() {
        p.shared.bypass.store(bypass, Ordering::Release);
    }
}
/// # Safety
/// Live handle, not concurrently destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_latency(handle: *mut NativePlugin) -> u32 {
    handle
        .as_ref()
        .map(|p| total_latency(p.shared.requested_latency.load(Ordering::Acquire)))
        .unwrap_or(0)
}
/// Apply a latency only after the format's host has acknowledged it.
/// # Safety
/// Live handle, not concurrently destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_accept_latency(handle: *mut NativePlugin, total: u32) {
    if let Some(p) = handle.as_ref() {
        let requested = p.shared.requested_latency.load(Ordering::Acquire);
        if total == total_latency(requested) || total == transport_part(requested) {
            p.shared.active_latency.store(requested, Ordering::Release);
        }
    }
}
/// Dynamically request a new transport latency profile budget.
/// Host must acknowledge via og_native_accept_latency.
/// # Safety
/// Live handle, not concurrently destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_set_transport_latency(handle: *mut NativePlugin, transport: u32) {
    if let Some(p) = handle.as_ref() {
        p.shared.base_transport.store(transport.clamp(32, 8192), Ordering::Release);
        let mut current = p.shared.requested_latency.load(Ordering::Acquire);
        loop {
            let updated = latency_pair(
                transport.clamp(32, 8192),
                plugin_part(current),
            );
            match p.shared.requested_latency.compare_exchange_weak(
                current,
                updated,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let active_curr = p.shared.active_latency.load(Ordering::Acquire);
                    let active_updated = latency_pair(transport.clamp(32, 8192), plugin_part(active_curr));
                    p.shared.active_latency.store(active_updated, Ordering::Release);
                    break;
                }
                Err(actual) => current = actual,
            }
        }
    }
}
/// # Safety
/// Live handle, not concurrently destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_status(handle: *mut NativePlugin) -> i32 {
    handle
        .as_ref()
        .map(|p| p.shared.status.load(Ordering::Acquire))
        .unwrap_or(-1)
}
/// Copy a discovered plugin name. Control-thread only, never call from audio.
/// # Safety
/// Live handle and writable buffer of capacity bytes. No concurrent destruction.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_name(
    handle: *mut NativePlugin,
    index: u32,
    buffer: *mut c_char,
    capacity: u32,
) {
    if buffer.is_null() || capacity == 0 {
        return;
    }
    *buffer = 0;
    if let Some(p) = handle.as_ref() {
        let names = p.shared.names.lock().unwrap_or_else(|e| e.into_inner());
        let name = if index == 0 {
            "None"
        } else {
            names
                .get(index as usize - 1)
                .map(String::as_str)
                .unwrap_or("Unavailable")
        };
        let count = name.len().min(capacity as usize - 1);
        std::ptr::copy_nonoverlapping(name.as_ptr(), buffer.cast(), count);
        *buffer.add(count) = 0;
    }
}

/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_num_loaded_plugins(handle: *mut NativePlugin) -> u32 {
    let Some(p) = handle.as_ref() else { return 0 };
    let guard = p.shared.loaded_plugins.lock().unwrap_or_else(|e| e.into_inner());
    guard.len() as u32
}

/// # Safety
/// Buffer pointers must be valid if not null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_get_loaded_plugin(
    handle: *mut NativePlugin,
    idx: u32,
    name_buf: *mut c_char,
    name_cap: u32,
    bypassed: *mut bool,
    oversampling: *mut u32,
) -> bool {
    let Some(p) = handle.as_ref() else { return false };
    let guard = p.shared.loaded_plugins.lock().unwrap_or_else(|e| e.into_inner());
    let Some(lp) = guard.get(idx as usize) else { return false };
    if !name_buf.is_null() && name_cap > 0 {
        let count = lp.name.len().min(name_cap as usize - 1);
        std::ptr::copy_nonoverlapping(lp.name.as_ptr(), name_buf.cast(), count);
        *name_buf.add(count) = 0;
    }
    if !bypassed.is_null() {
        *bypassed = lp.bypassed;
    }
    if !oversampling.is_null() {
        *oversampling = lp.oversampling_factor as u32;
    }
    true
}

/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_num_available_plugins(handle: *mut NativePlugin) -> u32 {
    let Some(p) = handle.as_ref() else { return 0 };
    let guard = p.shared.available_plugins.lock().unwrap_or_else(|e| e.into_inner());
    guard.len() as u32
}

/// # Safety
/// Buffer pointers must be valid if not null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_get_available_plugin(
    handle: *mut NativePlugin,
    idx: u32,
    name_buf: *mut c_char,
    name_cap: u32,
    id_buf: *mut c_char,
    id_cap: u32,
    cat_buf: *mut c_char,
    cat_cap: u32,
) -> bool {
    let Some(p) = handle.as_ref() else { return false };
    let guard = p.shared.available_plugins.lock().unwrap_or_else(|e| e.into_inner());
    let Some(ap) = guard.get(idx as usize) else { return false };
    if !name_buf.is_null() && name_cap > 0 {
        let count = ap.name.len().min(name_cap as usize - 1);
        std::ptr::copy_nonoverlapping(ap.name.as_ptr(), name_buf.cast(), count);
        *name_buf.add(count) = 0;
    }
    if !id_buf.is_null() && id_cap > 0 {
        let count = ap.id.len().min(id_cap as usize - 1);
        std::ptr::copy_nonoverlapping(ap.id.as_ptr(), id_buf.cast(), count);
        *id_buf.add(count) = 0;
    }
    if !cat_buf.is_null() && cat_cap > 0 {
        let cat = if ap.category.is_empty() {
            if ap.id.starts_with("AU") { "AudioUnit" } else { "VST3" }
        } else {
            &ap.category
        };
        let count = cat.len().min(cat_cap as usize - 1);
        std::ptr::copy_nonoverlapping(cat.as_ptr(), cat_buf.cast(), count);
        *cat_buf.add(count) = 0;
    }
    true
}

/// # Safety
/// Buffer pointers must be valid if not null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_get_available_plugin_ext(
    handle: *mut NativePlugin,
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
    let Some(p) = handle.as_ref() else { return false };
    let guard = p.shared.available_plugins.lock().unwrap_or_else(|e| e.into_inner());
    let Some(ap) = guard.get(idx as usize) else { return false };
    if !name_buf.is_null() && name_cap > 0 {
        let count = ap.name.len().min(name_cap as usize - 1);
        std::ptr::copy_nonoverlapping(ap.name.as_ptr(), name_buf.cast(), count);
        *name_buf.add(count) = 0;
    }
    if !id_buf.is_null() && id_cap > 0 {
        let count = ap.id.len().min(id_cap as usize - 1);
        std::ptr::copy_nonoverlapping(ap.id.as_ptr(), id_buf.cast(), count);
        *id_buf.add(count) = 0;
    }
    if !type_buf.is_null() && type_cap > 0 {
        let ptype = if !ap.plugin_type.is_empty() {
            &ap.plugin_type
        } else if ap.id.starts_with("AudioUnit") || ap.id.starts_with("AU") {
            "AudioUnit"
        } else if ap.id.starts_with("VST3") {
            "VST3"
        } else if ap.id.starts_with("VST") {
            "VST"
        } else {
            "AudioUnit"
        };
        let count = ptype.len().min(type_cap as usize - 1);
        std::ptr::copy_nonoverlapping(ptype.as_ptr(), type_buf.cast(), count);
        *type_buf.add(count) = 0;
    }
    if !company_buf.is_null() && company_cap > 0 {
        let comp = if ap.manufacturer.is_empty() { "Unknown" } else { &ap.manufacturer };
        let count = comp.len().min(company_cap as usize - 1);
        std::ptr::copy_nonoverlapping(comp.as_ptr(), company_buf.cast(), count);
        *company_buf.add(count) = 0;
    }
    if !cat_buf.is_null() && cat_cap > 0 {
        let cat = if ap.category.is_empty() { "General" } else { &ap.category };
        let count = cat.len().min(cat_cap as usize - 1);
        std::ptr::copy_nonoverlapping(cat.as_ptr(), cat_buf.cast(), count);
        *cat_buf.add(count) = 0;
    }
    true
}

/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_load_plugin(handle: *mut NativePlugin, available_idx: u32) -> bool {
    og_native_load_plugin_at(handle, available_idx, 0, 0)
}

/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_load_plugin_at(handle: *mut NativePlugin, available_idx: u32, x: i32, y: i32) -> bool {
    let Some(p) = handle.as_ref() else { return false };
    let layout = if p.channels == 1 { "Mono".to_string() } else { "Stereo".to_string() };
    let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(NativeCommand::LoadPlugin { available_idx: available_idx as usize, layout, x, y });
    true
}

/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_unload_plugin(handle: *mut NativePlugin, loaded_idx: u32) -> bool {
    let Some(p) = handle.as_ref() else { return false };
    let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(NativeCommand::UnloadPlugin { loaded_idx: loaded_idx as usize });
    true
}

/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_set_plugin_oversampling(handle: *mut NativePlugin, loaded_idx: u32, factor: u32) -> bool {
    let Some(p) = handle.as_ref() else { return false };
    let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(NativeCommand::SetPluginOversampling { loaded_idx: loaded_idx as usize, factor: factor as i32 });
    true
}

/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_set_plugin_bypassed(handle: *mut NativePlugin, loaded_idx: u32, bypassed: bool) -> bool {
    let Some(p) = handle.as_ref() else { return false };
    let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(NativeCommand::SetPluginBypassed { loaded_idx: loaded_idx as usize, bypassed });
    true
}

/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_edit_plugin(handle: *mut NativePlugin, loaded_idx: u32, x: i32, y: i32) -> bool {
    let Some(p) = handle.as_ref() else { return false };
    let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(NativeCommand::EditPlugin { loaded_idx: loaded_idx as usize, x, y });
    true
}

/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_hide_plugin(handle: *mut NativePlugin) -> bool {
    let Some(p) = handle.as_ref() else { return false };
    let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(NativeCommand::HidePlugin);
    true
}

/// # Safety
/// Buffer pointers must be valid if not null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_get_screen_info(
    handle: *mut NativePlugin,
    width: *mut u32,
    height: *mut u32,
    scale: *mut f64,
    frame_id: *mut u64,
) -> bool {
    let Some(p) = handle.as_ref() else { return false; };
    let guard = p.shared.screen_frame.lock();
    if guard.width == 0 || guard.height == 0 || guard.rgba.is_empty() {
        return false;
    }
    if !width.is_null() { *width = guard.width; }
    if !height.is_null() { *height = guard.height; }
    if !scale.is_null() { *scale = guard.scale; }
    if !frame_id.is_null() { *frame_id = guard.frame_id; }
    true
}

/// # Safety
/// Dest pointer must be writable for at least dest_cap bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_copy_screen_frame(
    handle: *mut NativePlugin,
    dest: *mut u8,
    dest_cap: u32,
    frame_id: *mut u64,
) -> bool {
    if dest.is_null() || dest_cap == 0 { return false; }
    let Some(p) = handle.as_ref() else { return false; };
    let guard = p.shared.screen_frame.lock();
    if guard.rgba.is_empty() { return false; }
    let count = guard.rgba.len().min(dest_cap as usize);
    std::ptr::copy_nonoverlapping(guard.rgba.as_ptr(), dest, count);
    if !frame_id.is_null() { *frame_id = guard.frame_id; }
    true
}

/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_send_mouse(
    handle: *mut NativePlugin,
    ev_type: u8,
    x: f32,
    y: f32,
    shift: bool,
    ctrl: bool,
    alt: bool,
    delta_x: f32,
    delta_y: f32,
    is_smooth: bool,
) {
    if let Some(p) = handle.as_ref() {
        let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(NativeCommand::Mouse {
            ev_type,
            x,
            y,
            shift,
            ctrl,
            alt,
            delta_x,
            delta_y,
            is_smooth,
        });
    }
}

/// # Safety
/// Keys pointer must be valid for count u16 elements.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_send_key(
    handle: *mut NativePlugin,
    keys: *const u16,
    count: u32,
) {
    if keys.is_null() || count == 0 { return; }
    if let Some(p) = handle.as_ref() {
        let key_slice = std::slice::from_raw_parts(keys, count as usize);
        let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(NativeCommand::Key {
            key_codes: key_slice.to_vec(),
        });
    }
}

/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_is_editor_open(handle: *mut NativePlugin) -> bool {
    handle
        .as_ref()
        .map(|p| p.shared.is_editor_open.load(Ordering::Acquire))
        .unwrap_or(false)
}

/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_is_server_local_mode(handle: *mut NativePlugin) -> bool {
    handle
        .as_ref()
        .map(|p| p.shared.server_local_mode.load(Ordering::Acquire))
        .unwrap_or(false)
}

/// Set a normalized parameter (`0.0..=1.0`).
/// param_id: 0=Bypass, 1=Selection, 2=LatencyProfile, 3=InputGain, 4=WetDryMix, 5=Drive, 6=OutputGain, 7..22=AutomationSlots 0..15,
/// 23=DriveCharacter (0 Clean, 0.5 Tb, 1 Tr), 24=DriveQuality (0 ADAA, 0.5 2x, 1 4x). 23/24 are local GUI/state settings, not host-automated.
/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_set_param(handle: *mut NativePlugin, param_id: u32, norm_val: f32) {
    let Some(p) = handle.as_ref() else { return };
    if !norm_val.is_finite() { return; }
    let clamped = norm_val.clamp(0.0, 1.0);
    match param_id {
        0 => p.shared.bypass.store(clamped >= 0.5, Ordering::Release),
        1 => p.shared.selected.store((clamped * 128.0).round() as u32, Ordering::Release),
        2 => {
            let prof = ((clamped * 3.0).round() as u32).min(3);
            p.shared.profile.store(prof, Ordering::Release);
            let target = match prof {
                0 => 128,
                1 => 256,
                2 => 512,
                _ => 1024,
            };
            og_native_set_transport_latency(handle, target);
        }
        3 => p.shared.input_gain.store(clamped.to_bits(), Ordering::Release),
        4 => p.shared.wet_dry_mix.store(clamped.to_bits(), Ordering::Release),
        5 => p.shared.drive.store(clamped.to_bits(), Ordering::Release),
        6 => p.shared.output_gain.store(clamped.to_bits(), Ordering::Release),
        23 => p.shared.drive_char.store((clamped * 2.0).round() as u32, Ordering::Release),
        24 => p.shared.drive_quality.store((clamped * 2.0).round() as u32, Ordering::Release),
        7..=22 => {
            let slot = (param_id - 7) as usize;
            p.shared.automation_slots[slot].store(clamped.to_bits(), Ordering::Release);
            let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
            q.push_back(NativeCommand::SetPluginParam {
                loaded_idx: 0,
                param_idx: slot as i32,
                val: clamped,
            });
        }
        _ => {}
    }
}

/// Get a normalized parameter (`0.0..=1.0`).
/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_get_param(handle: *mut NativePlugin, param_id: u32) -> f32 {
    let Some(p) = handle.as_ref() else { return 0.0 };
    match param_id {
        0 => if p.shared.bypass.load(Ordering::Acquire) { 1.0 } else { 0.0 },
        1 => (p.shared.selected.load(Ordering::Acquire) as f32 / 128.0).clamp(0.0, 1.0),
        2 => (p.shared.profile.load(Ordering::Acquire) as f32 / 3.0).clamp(0.0, 1.0),
        3 => f32::from_bits(p.shared.input_gain.load(Ordering::Acquire)),
        4 => f32::from_bits(p.shared.wet_dry_mix.load(Ordering::Acquire)),
        5 => f32::from_bits(p.shared.drive.load(Ordering::Acquire)),
        6 => f32::from_bits(p.shared.output_gain.load(Ordering::Acquire)),
        23 => p.shared.drive_char.load(Ordering::Acquire) as f32 / 2.0,
        24 => p.shared.drive_quality.load(Ordering::Acquire) as f32 / 2.0,
        7..=22 => {
            let slot = (param_id - 7) as usize;
            f32::from_bits(p.shared.automation_slots[slot].load(Ordering::Acquire))
        }
        _ => 0.0,
    }
}

/// Read true stereo peak levels (`0.0..=1.0`) measured on the audio thread.
/// # Safety
/// Handle and output pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_get_meters(
    handle: *mut NativePlugin,
    left: *mut f32,
    right: *mut f32,
) {
    let Some(p) = handle.as_ref() else {
        if !left.is_null() { *left = 0.0; }
        if !right.is_null() { *right = 0.0; }
        return;
    };
    if !left.is_null() {
        *left = f32::from_bits(p.shared.meter_l.load(Ordering::Relaxed));
    }
    if !right.is_null() {
        *right = f32::from_bits(p.shared.meter_r.load(Ordering::Relaxed));
    }
}

/// Read true stereo input peak levels (post-Input Gain, pre-network/output) measured on the audio thread.
/// # Safety
/// Handle and output pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_get_input_meters(
    handle: *mut NativePlugin,
    left: *mut f32,
    right: *mut f32,
) {
    let Some(p) = handle.as_ref() else {
        if !left.is_null() { *left = 0.0; }
        if !right.is_null() { *right = 0.0; }
        return;
    };
    if !left.is_null() {
        *left = f32::from_bits(p.shared.meter_in_l.load(Ordering::Relaxed));
    }
    if !right.is_null() {
        *right = f32::from_bits(p.shared.meter_in_r.load(Ordering::Relaxed));
    }
}

/// Swap two loaded plugin slots in the chain.
/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_exchange_plugins(
    handle: *mut NativePlugin,
    idx_a: u32,
    idx_b: u32,
) -> bool {
    let Some(p) = handle.as_ref() else { return false };
    let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(NativeCommand::ExchangePlugins {
        idx_a: idx_a as usize,
        idx_b: idx_b as usize,
    });
    true
}

/// Return number of presets on a loaded plugin slot.
/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_num_presets(handle: *mut NativePlugin, loaded_idx: u32) -> u32 {
    let Some(p) = handle.as_ref() else { return 0 };
    let guard = p.shared.loaded_plugins.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get(loaded_idx as usize)
        .map(|lp| lp.presets.len() as u32)
        .unwrap_or(0)
}

/// Copy a preset name for a loaded plugin slot.
/// # Safety
/// Handle and buffer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_get_preset_name(
    handle: *mut NativePlugin,
    loaded_idx: u32,
    preset_idx: u32,
    buf: *mut c_char,
    cap: u32,
) -> bool {
    if buf.is_null() || cap == 0 { return false; }
    *buf = 0;
    let Some(p) = handle.as_ref() else { return false };
    let guard = p.shared.loaded_plugins.lock().unwrap_or_else(|e| e.into_inner());
    let Some(lp) = guard.get(loaded_idx as usize) else { return false };
    let Some(name) = lp.presets.get(preset_idx as usize) else { return false };
    let count = name.len().min(cap as usize - 1);
    std::ptr::copy_nonoverlapping(name.as_ptr(), buf.cast(), count);
    *buf.add(count) = 0;
    true
}

/// Select a preset on a loaded plugin slot.
/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_set_preset(
    handle: *mut NativePlugin,
    loaded_idx: u32,
    preset_idx: u32,
) -> bool {
    let Some(p) = handle.as_ref() else { return false };
    let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(NativeCommand::SetPreset {
        loaded_idx: loaded_idx as usize,
        preset_idx: preset_idx as usize,
    });
    true
}

/// Return number of parameters on a loaded plugin slot.
/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_num_plugin_params(
    handle: *mut NativePlugin,
    loaded_idx: u32,
) -> u32 {
    let Some(p) = handle.as_ref() else { return 0 };
    let guard = p.shared.loaded_plugins.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get(loaded_idx as usize)
        .map(|lp| lp.params.len() as u32)
        .unwrap_or(0)
}

/// Query parameter info for a loaded plugin slot.
/// # Safety
/// Handle and pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_get_plugin_param_info(
    handle: *mut NativePlugin,
    loaded_idx: u32,
    param_idx: u32,
    name_buf: *mut c_char,
    name_cap: u32,
    default_val: *mut f32,
    current_val: *mut f32,
) -> bool {
    let Some(p) = handle.as_ref() else { return false };
    let guard = p.shared.loaded_plugins.lock().unwrap_or_else(|e| e.into_inner());
    let Some(lp) = guard.get(loaded_idx as usize) else { return false };
    let Some(prm) = lp.params.get(param_idx as usize) else { return false };
    if !name_buf.is_null() && name_cap > 0 {
        let count = prm.name.len().min(name_cap as usize - 1);
        std::ptr::copy_nonoverlapping(prm.name.as_ptr(), name_buf.cast(), count);
        *name_buf.add(count) = 0;
    }
    if !default_val.is_null() { *default_val = prm.default_value; }
    if !current_val.is_null() { *current_val = prm.current_value; }
    true
}

/// Set a parameter value on a loaded plugin slot.
/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_set_plugin_param(
    handle: *mut NativePlugin,
    loaded_idx: u32,
    param_idx: u32,
    val: f32,
) -> bool {
    let Some(p) = handle.as_ref() else { return false };
    let real_idx = {
        let mut guard = p.shared.loaded_plugins.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(lp) = guard.get_mut(loaded_idx as usize) {
            if let Some(prm) = lp.params.get_mut(param_idx as usize) {
                prm.current_value = val;
                prm.idx
            } else {
                param_idx as i32
            }
        } else {
            return false;
        }
    };
    let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(NativeCommand::SetPluginParam {
        loaded_idx: loaded_idx as usize,
        param_idx: real_idx,
        val,
    });
    true
}

/// Store current chain state into slot A.
/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_store_a(handle: *mut NativePlugin) {
    if let Some(p) = handle.as_ref() {
        let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(NativeCommand::StoreA);
    }
}

/// Store current chain state into slot B.
/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_store_b(handle: *mut NativePlugin) {
    if let Some(p) = handle.as_ref() {
        let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(NativeCommand::StoreB);
    }
}

/// Toggle between A and B chain states.
/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_toggle_ab(handle: *mut NativePlugin) {
    if let Some(p) = handle.as_ref() {
        let cur = p.shared.active_is_b.load(Ordering::Acquire);
        p.shared.active_is_b.store(!cur, Ordering::Release);
        let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(NativeCommand::ToggleAB);
    }
}

/// Return true if slot B is currently active.
/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_is_b(handle: *mut NativePlugin) -> bool {
    handle
        .as_ref()
        .map(|p| p.shared.active_is_b.load(Ordering::Acquire))
        .unwrap_or(false)
}

/// Get active server address (`host:port`).
/// # Safety
/// Handle and buffer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_get_active_server(
    handle: *mut NativePlugin,
    buf: *mut c_char,
    cap: u32,
) {
    if buf.is_null() || cap == 0 { return; }
    *buf = 0;
    let Some(p) = handle.as_ref() else { return };
    let srv = p.shared.active_server.lock().unwrap_or_else(|e| e.into_inner());
    let count = srv.len().min(cap as usize - 1);
    std::ptr::copy_nonoverlapping(srv.as_ptr(), buf.cast(), count);
    *buf.add(count) = 0;
}

/// Connect to a new server (`host:port`).
/// # Safety
/// Handle and C-string must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_set_server(
    handle: *mut NativePlugin,
    host_port: *const c_char,
) -> bool {
    if host_port.is_null() { return false; }
    let Some(p) = handle.as_ref() else { return false };
    let Ok(s) = CStr::from_ptr(host_port).to_str() else { return false };
    let (host, port) = s
        .rsplit_once(':')
        .and_then(|(h, pt)| pt.parse::<u16>().ok().map(|port| (h.to_string(), port)))
        .unwrap_or_else(|| (s.to_string(), 55056));
    let formatted = format!("{}:{}", host, port);
    *p.shared.active_server.lock().unwrap_or_else(|e| e.into_inner()) = formatted.clone();
    {
        let mut list = p.shared.discovered_servers.lock().unwrap_or_else(|e| e.into_inner());
        if !list.contains(&formatted) {
            list.push(formatted);
        }
    }
    let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(NativeCommand::ReconnectServer { host, port });
    true
}

/// Return number of known/discovered servers.
/// # Safety
/// Handle must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_num_discovered_servers(handle: *mut NativePlugin) -> u32 {
    let Some(p) = handle.as_ref() else { return 0 };
    let list = p.shared.discovered_servers.lock().unwrap_or_else(|e| e.into_inner());
    list.len() as u32
}

/// Copy discovered server string (`host:port`).
/// # Safety
/// Handle and buffer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_get_discovered_server(
    handle: *mut NativePlugin,
    idx: u32,
    buf: *mut c_char,
    cap: u32,
) -> bool {
    if buf.is_null() || cap == 0 { return false; }
    *buf = 0;
    let Some(p) = handle.as_ref() else { return false };
    let list = p.shared.discovered_servers.lock().unwrap_or_else(|e| e.into_inner());
    let Some(srv) = list.get(idx as usize) else { return false };
    let count = srv.len().min(cap as usize - 1);
    std::ptr::copy_nonoverlapping(srv.as_ptr(), buf.cast(), count);
    *buf.add(count) = 0;
    true
}

/// Serialize full plugin state (State Version 7 JSON including loaded plugins, oversampling,
/// base64 plugin settings chunks, and local console parameters).
/// If `buf` is null or `cap == 0`, refreshes state and returns required byte length.
/// Otherwise writes up to `cap` bytes into `buf` and returns bytes written.
/// # Safety
/// Handle must be valid; if `buf` is non-null, it must be writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_get_state(
    handle: *mut NativePlugin,
    buf: *mut u8,
    cap: u32,
) -> u32 {
    let Some(p) = handle.as_ref() else { return 0 };
    let state_bytes = if p.worker.is_some() && !p.shared.stop.load(Ordering::Acquire) {
        let (tx, rx) = mpsc::sync_channel(1);
        {
            let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
            q.push_back(NativeCommand::FetchState { reply: tx });
        }
        rx.recv_timeout(Duration::from_millis(250)).unwrap_or_else(|_| {
            let mut dummy = PluginProcessor::new(PluginMode::Fx);
            p.shared.build_full_state(&mut dummy)
        })
    } else {
        let mut dummy = PluginProcessor::new(PluginMode::Fx);
        p.shared.build_full_state(&mut dummy)
    };
    if buf.is_null() || cap == 0 {
        return state_bytes.len() as u32;
    }
    let count = state_bytes.len().min(cap as usize);
    std::ptr::copy_nonoverlapping(state_bytes.as_ptr(), buf, count);
    count as u32
}

/// Restore full plugin state (State Version 7 JSON).
/// # Safety
/// Handle and `data` slice must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_set_state(
    handle: *mut NativePlugin,
    data: *const u8,
    len: u32,
) -> bool {
    if data.is_null() || len == 0 { return false; }
    let Some(p) = handle.as_ref() else { return false };
    let slice = std::slice::from_raw_parts(data, len as usize);
    let Ok(j) = serde_json::from_slice::<serde_json::Value>(slice) else { return false };
    if let Some(byp) = j.get("bypass").and_then(|v| v.as_bool()) {
        p.shared.bypass.store(byp, Ordering::Release);
    }
    if let Some(sel) = j.get("selectedPlugin").and_then(|v| v.as_u64()) {
        p.shared.selected.store(sel as u32, Ordering::Release);
    }
    if let Some(prof) = j.get("latencyProfile").and_then(|v| v.as_u64()) {
        og_native_set_param(handle, 2, (prof as f32) / 3.0);
    }
    if let Some(v) = j.get("inputGain").and_then(|v| v.as_f64()) {
        p.shared.input_gain.store((v as f32).clamp(0.0, 1.0).to_bits(), Ordering::Release);
    }
    if let Some(v) = j.get("wetDryMix").and_then(|v| v.as_f64()) {
        p.shared.wet_dry_mix.store((v as f32).clamp(0.0, 1.0).to_bits(), Ordering::Release);
    }
    if let Some(v) = j.get("drive").and_then(|v| v.as_f64()) {
        p.shared.drive.store((v as f32).clamp(0.0, 1.0).to_bits(), Ordering::Release);
    }
    p.shared.restore_drive_modes(&j);
if let Some(v) = j.get("outputGain").and_then(|v| v.as_f64()) {
        p.shared.output_gain.store((v as f32).clamp(0.0, 1.0).to_bits(), Ordering::Release);
    }
    *p.shared.cached_state.lock().unwrap_or_else(|e| e.into_inner()) = slice.to_vec();
    let mut q = p.shared.commands.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(NativeCommand::RestoreState {
        data: slice.to_vec(),
    });
    true
}

/// Queue a 3-byte MIDI event for the next `og_native_process` audio block without allocation.
/// # Safety
/// Must be called on the audio thread before `og_native_process`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_queue_midi(
    handle: *mut NativePlugin,
    sample_offset: i32,
    status: u8,
    data1: u8,
    data2: u8,
) {
    let Some(p) = handle.as_ref() else { return };
    let cb = &mut *p.audio.get();
    if cb.pending_midi_count < MAX_MIDI_EVENTS {
        cb.pending_midi[cb.pending_midi_count] = OgMidiEvent {
            sample_offset: sample_offset.max(0),
            len: 3,
            data: [status, data1, data2],
        };
        cb.pending_midi_count += 1;
    }
}

/// Process 64-bit float (`double`) planar audio buffers.
/// # Safety
/// Handle and planar channel buffers must be valid for `frames` samples.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_process_f64(
    handle: *mut NativePlugin,
    inputs: *const *const f64,
    outputs: *const *mut f64,
    frames: u32,
) -> i32 {
    let Some(plugin) = handle.as_ref() else { return -1 };
    let f = frames as usize;
    if f == 0 { return 0; }
    if f > plugin.max_frames || inputs.is_null() || outputs.is_null() { return -1; }
    let in_ch = std::slice::from_raw_parts(inputs, plugin.channels);
    let out_ch = std::slice::from_raw_parts(outputs, plugin.channels);
    if in_ch.iter().any(|p| p.is_null()) || out_ch.iter().any(|p| p.is_null()) {
        return -1;
    }
    let mut in_f32 = [[0.0f32; 8192]; 2];
    let mut out_f32 = [[0.0f32; 8192]; 2];
    for ch in 0..plugin.channels.min(2) {
        for n in 0..f {
            in_f32[ch][n] = *in_ch[ch].add(n) as f32;
        }
    }
    let in_ptrs = [in_f32[0].as_ptr(), in_f32[1].as_ptr()];
    let out_ptrs = [out_f32[0].as_mut_ptr(), out_f32[1].as_mut_ptr()];
    let rc = plugin.process(in_ptrs.as_ptr(), out_ptrs.as_ptr(), f);
    if rc == 0 {
        for ch in 0..plugin.channels.min(2) {
            for n in 0..f {
                *out_ch[ch].add(n) = out_f32[ch][n] as f64;
            }
        }
    }
    rc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    thread_local! { static HEAP_OPS: Cell<Option<usize>> = const { Cell::new(None) }; }
    struct CountHeap;
    fn heap_op() {
        let _ = HEAP_OPS.try_with(|count| {
            if let Some(n) = count.get() {
                count.set(Some(n + 1));
            }
        });
    }
    unsafe impl GlobalAlloc for CountHeap {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            heap_op();
            System.alloc(layout)
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            heap_op();
            System.dealloc(ptr, layout);
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            heap_op();
            System.realloc(ptr, layout, size)
        }
    }
    #[global_allocator]
    static ALLOCATOR: CountHeap = CountHeap;

    #[test]
    fn callback_does_not_allocate_or_free_when_pool_is_exhausted() {
        let plugin = NativePlugin::new(48000.0, 64, 2, 32, None);
        let input = [0.25f32; 64];
        let mut left = [0.0f32; 64];
        let mut right = [0.0f32; 64];
        let ins = [input.as_ptr(), input.as_ptr()];
        let outs = [left.as_mut_ptr(), right.as_mut_ptr()];
        HEAP_OPS.with(|n| n.set(Some(0)));
        for _ in 0..1000 {
            assert_eq!(
                unsafe { plugin.process(ins.as_ptr(), outs.as_ptr(), 64) },
                0
            );
        }
        let count = HEAP_OPS.with(|n| n.replace(None)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn late_wet_audio_is_never_played_at_a_new_time() {
        let mut plugin = NativePlugin::new(48000.0, 64, 1, 32, None);
        let (mut returned, receive) = spsc::channel(POOL);
        let cb = plugin.audio.get_mut();
        cb.receive = receive;
        cb.clock = 128;
        let mut late = cb.free.pop().unwrap();
        late.start = 0;
        late.frames = 16;
        late.valid = true;
        late.latency = latency_pair(32, 0);
        late.audio[0].fill(0.9);
        assert!(returned.push(late).is_ok());
        let input = [0.0f32; 64];
        let mut output = [0.0f32; 64];
        assert_eq!(
            unsafe {
                plugin.process(
                    [input.as_ptr()].as_ptr(),
                    [output.as_mut_ptr()].as_ptr(),
                    64,
                )
            },
            0
        );
        assert!(output.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn latency_changes_wait_for_host_acknowledgement() {
        let mut plugin = NativePlugin::new(48000.0, 64, 1, 32, None);
        plugin.shared.request_plugin_latency(18);
        let p = &mut plugin as *mut NativePlugin;
        assert_eq!(unsafe { og_native_latency(p) }, 50);
        assert_eq!(
            plugin_part(plugin.shared.active_latency.load(Ordering::Acquire)),
            0
        );
        unsafe {
            og_native_accept_latency(p, 49);
        }
        assert_eq!(
            plugin_part(plugin.shared.active_latency.load(Ordering::Acquire)),
            0
        );
        unsafe {
            og_native_accept_latency(p, 50);
        }
        assert_eq!(
            plugin_part(plugin.shared.active_latency.load(Ordering::Acquire)),
            18
        );
    }

    #[test]
    fn larger_host_blocks_request_a_coherent_larger_budget() {
        let mut plugin = NativePlugin::new(48000.0, 1024, 1, 256, None);
        plugin.shared.request_plugin_latency(18);
        let input = [0.25f32; 1024];
        let mut output = [0.0f32; 1024];
        assert_eq!(
            unsafe {
                plugin.process(
                    [input.as_ptr()].as_ptr(),
                    [output.as_mut_ptr()].as_ptr(),
                    1024,
                )
            },
            0
        );
        let handle = &mut plugin as *mut NativePlugin;
        assert_eq!(unsafe { og_native_latency(handle) }, 1298); // 1024-frame block + the 256 profile budget + 18 plugin
        assert_eq!(
            plugin.shared.active_latency.load(Ordering::Acquire),
            latency_pair(256, 0)
        );
        unsafe {
            og_native_accept_latency(handle, 1298);
        }
        assert_eq!(
            plugin.shared.active_latency.load(Ordering::Acquire),
            latency_pair(1280, 18)
        );
    }

    #[test]
    #[ignore = "benchmark"]
    fn callback_cost_with_drive() {
        let plugin = NativePlugin::new(48000.0, 1024, 2, 1024, None);
        let input = [0.25f32; 128];
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        for (label, drive, quality) in [("off", 0.0f32, 0u32), ("adaa", 1.0, 0), ("2x", 1.0, 1), ("4x", 1.0, 2)] {
            plugin.shared.drive.store(drive.to_bits(), Ordering::Release);
            plugin.shared.drive_quality.store(quality, Ordering::Release);
            let start = Instant::now();
            for _ in 0..2000 {
                let ins = [input.as_ptr(), input.as_ptr()];
                let outs = [l.as_mut_ptr(), r.as_mut_ptr()];
                unsafe { plugin.process(ins.as_ptr(), outs.as_ptr(), 128) };
            }
            let per_block = start.elapsed().as_secs_f64() / 2000.0;
            println!("drive {label}: {:.1} us per 128-frame block ({:.2}% of real time)", per_block * 1e6, per_block / (128.0 / 48000.0) * 100.0);
        }
    }

    #[test]
    fn offline_render_waits_for_server_instead_of_dry_fallback() {
        use og_common::server_plugin::ServerPlugin;
        use og_hosting::vst3::default_plugin_parameters;
        use og_server::{Server, ServerConfig};
        let server = Server::new(ServerConfig::default());
        let mut desc = ServerPlugin::new(
            "Native Test Gain",
            "OpenGridder",
            "OpenGridder:Gain",
            "OpenGridder:Gain",
            "OpenGridder",
            "Fx",
            false,
            vec!["Stereo".into()],
        );
        desc.parameters = default_plugin_parameters();
        desc.parameters[0].current_value = 1.0;
        desc.parameters[0].default_value = 1.0;
        server.add_known_plugin(desc);
        let port = server.start("127.0.0.1", 0).unwrap();
        let plugin = NativePlugin::new(48000.0, 1024, 2, 256, Some(("127.0.0.1".into(), port)));
        plugin.shared.selected.store(1, Ordering::Release);
        plugin.shared.offline.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(3);
        while plugin.shared.status.load(Ordering::Acquire) != 3 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(plugin.shared.status.load(Ordering::Acquire), 3);
        let input = [0.25f32; 128];
        let mut left = [0.0f32; 128];
        let mut right = [0.0f32; 128];
        let (mut processed_blocks, mut dry_blocks) = (0, 0);
        // Back-to-back callbacks with no pacing, as in a bounce.
        for block in 0..400 {
            let ins = [input.as_ptr(), input.as_ptr()];
            let outs = [left.as_mut_ptr(), right.as_mut_ptr()];
            assert_eq!(unsafe { plugin.process(ins.as_ptr(), outs.as_ptr(), 128) }, 0);
            if block >= 20 {
                processed_blocks += 1;
                if left.iter().any(|&v| (v - 0.5).abs() > 1e-5) {
                    dry_blocks += 1;
                }
            }
        }
        assert!(processed_blocks > 0);
        assert_eq!(dry_blocks, 0, "offline render must be fully processed, not dry or silent");
    }

    #[test]
    fn worker_processes_remote_audio_without_blocking_callback() {
        use og_common::server_plugin::ServerPlugin;
        use og_hosting::vst3::default_plugin_parameters;
        use og_server::{Server, ServerConfig};
        let server = Server::new(ServerConfig::default());
        let mut desc = ServerPlugin::new(
            "Native Test Gain",
            "OpenGridder",
            "OpenGridder:Gain",
            "OpenGridder:Gain",
            "OpenGridder",
            "Fx",
            false,
            vec!["Stereo".into()],
        );
        desc.parameters = default_plugin_parameters();
        desc.parameters[0].current_value = 1.0;
        desc.parameters[0].default_value = 1.0;
        server.add_known_plugin(desc);
        let port = server.start("127.0.0.1", 0).unwrap();
        let mut plugin = NativePlugin::new(48000.0, 1024, 2, 256, Some(("127.0.0.1".into(), port)));
        plugin.shared.selected.store(1, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(3);
        while plugin.shared.status.load(Ordering::Acquire) != 3 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(plugin.shared.status.load(Ordering::Acquire), 3);
        let input = [0.25f32; 32];
        let mut left = [0.0f32; 32];
        let mut right = [0.0f32; 32];
        let mut heard_remote = false;
        for _ in 0..100 {
            let ins = [input.as_ptr(), input.as_ptr()];
            let outs = [left.as_mut_ptr(), right.as_mut_ptr()];
            HEAP_OPS.with(|n| n.set(Some(0)));
            let result = unsafe { plugin.process(ins.as_ptr(), outs.as_ptr(), 32) };
            let heap_ops = HEAP_OPS.with(|n| n.replace(None)).unwrap();
            assert_eq!(result, 0);
            assert_eq!(
                heap_ops, 0,
                "live worker returns must not allocate/free on the callback"
            );
            heard_remote |= left.iter().any(|&v| (v - 0.5).abs() < 1e-5);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(plugin.shared.processed.load(Ordering::Acquire) > 0);
        assert!(
            heard_remote,
            "expected 2x gain from the server, not delayed dry fallback"
        );
        let input = [0.25f32; 1024];
        let mut left = [0.0f32; 1024];
        let mut right = [0.0f32; 1024];
        let mut heard_large_block = false;
        for _ in 0..30 {
            let ins = [input.as_ptr(), input.as_ptr()];
            let outs = [left.as_mut_ptr(), right.as_mut_ptr()];
            assert_eq!(
                unsafe { plugin.process(ins.as_ptr(), outs.as_ptr(), 1024) },
                0
            );
            let handle = &mut plugin as *mut NativePlugin;
            unsafe {
                og_native_accept_latency(handle, og_native_latency(handle));
            }
            heard_large_block |= left.iter().any(|&v| (v - 0.5).abs() < 1e-5);
            thread::sleep(Duration::from_millis(4));
        }
        assert!(
            heard_large_block,
            "large blocks must not be permanently late/dry"
        );
        drop(plugin);
        server.stop();
    }
    #[test]
    fn disconnected_callback_has_exact_delay_across_variable_blocks() {
        let plugin = NativePlugin::new(48000.0, 64, 2, 32, None);
        let mut timeline = Vec::new();
        let mut position = 0;
        for frames in [1, 7, 16, 3, 64, 32, 64] {
            let input: Vec<f32> = (position..position + frames)
                .map(|n| if n == 0 { 1.0 } else { 0.0 })
                .collect();
            let mut left = vec![0.0; frames];
            let mut right = vec![0.0; frames];
            let ins = [input.as_ptr(), input.as_ptr()];
            let outs = [left.as_mut_ptr(), right.as_mut_ptr()];
            assert_eq!(
                unsafe { plugin.process(ins.as_ptr(), outs.as_ptr(), frames) },
                0
            );
            assert_eq!(left, right);
            timeline.extend(left);
            position += frames;
        }
        assert_eq!(timeline[32], 1.0);
        assert_eq!(timeline.iter().filter(|&&v| v != 0.0).count(), 1);
    }
    #[test]
    fn invalid_configuration_is_rejected() {
        assert!(og_native_create(f64::NAN, 64, 2, 32).is_null());
        assert!(og_native_create(48000.0, 0, 2, 32).is_null());
        assert!(og_native_create(48000.0, 64, 3, 32).is_null());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_callback_receives_real_audio_unit_processing() {
        use og_common::server_plugin::ServerPlugin;
        use og_server::{Server, ServerConfig};
        let server = Server::new(ServerConfig::default());
        server.add_known_plugin(ServerPlugin::new(
            "AULowpass",
            "Apple",
            "AudioUnit:aufx:lpas:appl",
            "AULowpass",
            "AudioUnit",
            "Filter",
            false,
            vec!["Stereo".into()],
        ));
        let port = server.start("127.0.0.1", 0).unwrap();
        let plugin = NativePlugin::new(48000., 256, 2, 256, Some(("127.0.0.1".into(), port)));
        plugin.shared.selected.store(1, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(5);
        while plugin.shared.status.load(Ordering::Acquire) != 3 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(plugin.shared.status.load(Ordering::Acquire), 3);
        let mut wet_energy = 0.;
        let mut input = [0.0f32; 64];
        let mut left = [0.0f32; 64];
        let mut right = [0.0f32; 64];
        for block in 0..150 {
            for (index, value) in input.iter_mut().enumerate() {
                *value = (std::f64::consts::TAU * 16000. * (block * 64 + index) as f64 / 48000.)
                    .sin() as f32
                    * 0.25;
            }
            HEAP_OPS.with(|n| n.set(Some(0)));
            let result = unsafe {
                plugin.process(
                    [input.as_ptr(), input.as_ptr()].as_ptr(),
                    [left.as_mut_ptr(), right.as_mut_ptr()].as_ptr(),
                    64,
                )
            };
            assert_eq!(HEAP_OPS.with(|n| n.replace(None)), Some(0));
            assert_eq!(result, 0);
            assert_eq!(left, right);
            if block >= 100 {
                wet_energy += left.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
            }
            thread::sleep(Duration::from_millis(2));
        }
        let rms = (wet_energy / (50 * 64) as f64).sqrt();
        assert!(
            rms > 0.00001 && rms < 0.1,
            "Native callback must receive real lowpass output, not dry fallback (RMS={rms})"
        );
        assert!(plugin.shared.processed.load(Ordering::Acquire) > 100);
        drop(plugin);
        server.stop();
    }
}
