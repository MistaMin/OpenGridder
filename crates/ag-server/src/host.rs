//! AudioUnit scan and load. The chain calls `scan` and `instantiate`.
//!
//! System AudioUnits are opened in-process. The generic editor is an RGBA
//! image of the parameter list, drawn here, not a screen capture.

use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use ag_protocol::PositionInfo;
use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub struct CatalogPlugin {
    pub name: String,
    pub company: String,
    pub id: String,
    pub format: String,
    pub is_instrument: bool,
    pub category: String,
}

/// A plugin instance whose editor `crate::ui::open_target` can show (raw handles).
#[derive(Clone, Copy, Debug)]
pub enum EditorTarget {
    /// An AudioUnit instance.
    Au(usize),
    /// A VST3 or CLAP instance from native/host.cpp.
    Native(usize),
}

pub struct EditorFrame {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

pub trait Hosted: Send {
    fn name(&self) -> String;
    fn has_editor(&self) -> bool;
    fn supports_double(&self) -> bool;
    fn latency(&self) -> i32;
    fn parameters(&self) -> Value;
    fn set_param(&mut self, index: i32, normalized: f32) -> bool;
    fn param(&self, index: i32) -> f32;
    fn set_oversampling(&mut self, factor: i32);
    fn oversampling_latency(&self) -> i32;
    fn process_f32(&mut self, audio: &mut [Vec<f32>], midi: &[[u8; 3]], frames: usize);
    fn process_f64(&mut self, audio: &mut [Vec<f64>], midi: &[[u8; 3]], frames: usize);
    fn show_editor(&mut self);
    /// What `crate::ui` needs to show this plugin's real editor, `None` without one.
    /// The window is opened without the chain lock, so audio keeps running meanwhile.
    fn editor_target(&self) -> Option<EditorTarget> {
        None
    }
    fn hide_editor(&mut self);
    fn editor_frame(&mut self) -> Option<EditorFrame>;
    fn mouse(&mut self, kind: u8, x: f32, y: f32, shift: bool, ctrl: bool, alt: bool, delta_x: f32, delta_y: f32);
    fn key(&mut self, codes: &[u16]);
    fn preset_names(&self) -> Vec<String> {
        Vec::new()
    }
    fn set_program(&mut self, _index: i32) -> bool {
        false
    }
    fn set_transport(&mut self, _position: &PositionInfo) {}
    /// Beat, tempo, playing, and timeline sample reported by the AudioUnit host callbacks.
    fn host_callback_transport(&self) -> Option<(f64, f64, bool, f64)> {
        None
    }
}

/// Mac virtual key codes, the same space AudioGridder forwards in `Key`.
/// Shift, control, and option do not move a parameter. Up and down step it.
pub(crate) fn apply_editor_key(current: f32, codes: &[u16]) -> Option<f32> {
    let mut value = current;
    let mut touched = false;
    for code in codes {
        match *code {
            0x38 | 0x3B | 0x3A => {}
            0x7E => {
                value = (value + 0.25).clamp(0.0, 1.0);
                touched = true;
            }
            0x7D => {
                value = (value - 0.25).clamp(0.0, 1.0);
                touched = true;
            }
            _ => {}
        }
    }
    touched.then_some(value)
}

/// `MouseEvType::WHEEL` is 10. `mouseevent_t::deltaY` is one notch at about 1.0,
/// and `deltaX` is used when `deltaY` is 0. The generic editor steps 0.25 per
/// notch, the same distance as the up arrow. This is not an OS scroll event.
pub(crate) fn apply_editor_wheel(current: f32, delta_x: f32, delta_y: f32) -> Option<f32> {
    let notches = if delta_y.is_finite() && delta_y.abs() > 1.0e-6 {
        delta_y
    } else if delta_x.is_finite() {
        delta_x
    } else {
        return None;
    };
    if notches.abs() <= 1.0e-6 {
        return None;
    }
    let next = (current + notches * 0.25).clamp(0.0, 1.0);
    ((next - current).abs() > 1.0e-6).then_some(next)
}

static CATALOG: Mutex<Option<Vec<CatalogPlugin>>> = Mutex::new(None);
static SCAN_GENERATION: AtomicU64 = AtomicU64::new(0);

/// How many times `scan` has rebuilt the catalog. A warm `scan` does not move it.
#[cfg(test)]
pub fn scan_generation() -> u64 {
    SCAN_GENERATION.load(Ordering::Acquire)
}

/// Drop the cached catalog. AudioGridder's Rescan payload `1` wipes the known
/// list and then restarts with a scan. Any other number restarts with a scan
/// and keeps the known list. There is no separate known-plugin file here, so
/// the wipe flag is the worker's stop reason: `restart-rescan-wipe` or
/// `restart-rescan`. The next `scan` rebuilds and advances `scan_generation`.
pub fn rescan(wipe: bool) -> &'static str {
    *CATALOG.lock().expect("catalog") = None;
    if wipe {
        crate::bundle_scan::wipe();
    }
    if wipe { "restart-rescan-wipe" } else { "restart-rescan" }
}

pub fn scan() -> Vec<CatalogPlugin> {
    let mut cache = CATALOG.lock().expect("catalog");
    if let Some(found) = cache.as_ref() {
        return found.clone();
    }
    // Hold the lock across the rebuild so overlapping scans share one generation bump.
    let found = scan_fresh();
    *cache = Some(found.clone());
    SCAN_GENERATION.fetch_add(1, Ordering::Release);
    found
}

/// Load the VST3/CLAP bundles that need loading in worker processes while the server is
/// already serving; then rebuild the catalog so clients see them (they re-request the
/// list when the plugin browser opens). One at a time.
pub fn scan_bundles_in_background() {
    static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| {
        let before = scan().len();
        crate::bundle_scan::scan(true);
        let fresh = scan_fresh();
        let total = fresh.len();
        *CATALOG.lock().expect("catalog") = Some(fresh);
        SCAN_GENERATION.fetch_add(1, Ordering::Release);
        eprintln!("ag-server: background plugin scan done, catalog {before} -> {total}");
        println!("SCAN_BACKGROUND_DONE:{}", serde_json::json!({"total": total}));
        let _ = std::io::Write::flush(&mut std::io::stdout());
        RUNNING.store(false, Ordering::Release);
    });
}

/// Catalog name for `id` if this process has already scanned. Never starts a scan
/// (chain children have no catalog and must not rescan to name a plugin).
pub fn cached_name(id: &str) -> Option<String> {
    let cache = CATALOG.lock().ok()?;
    cache.as_ref()?.iter().find(|plugin| plugin.id == id).map(|plugin| plugin.name.clone()).filter(|name| !name.is_empty())
}

fn scan_fresh() -> Vec<CatalogPlugin> {
    let mut found = Vec::new();
    for (kind, subtype, maker, instrument, fallback) in PREFERRED {
        if let Some(plugin) = describe(fourcc(kind), fourcc(subtype), fourcc(maker), *instrument, fallback) {
            push_unique(&mut found, plugin);
        }
    }
    for plugin in crate::native::scan() {
        push_unique(&mut found, plugin);
    }
    // VST3 and CLAP bundles listed without loading code (cache, moduleinfo.json); the rest
    // come from the background worker scan (`scan_bundles_in_background`).
    for plugin in crate::bundle_scan::scan(false) {
        push_unique(&mut found, plugin);
    }
    // Every registered AudioUnit. Enumeration reads component metadata only (no instances).
    for kind in [b"aufx", b"aumf", b"aumu"] {
        collect_type(fourcc_bytes(kind), kind == b"aumu", &mut found);
    }
    found
}

/// Plugins are created on the main thread (AudioGridder does the same): JUCE-based
/// plugins bind their message handling and editor timers to the creating thread.
pub fn instantiate(id: &str, sample_rate: f32, max_block: usize, channels: usize) -> Option<Box<dyn Hosted>> {
    crate::ui::on_main(|| instantiate_here(id, sample_rate, max_block, channels))
}

fn instantiate_here(id: &str, sample_rate: f32, max_block: usize, channels: usize) -> Option<Box<dyn Hosted>> {
    if id.starts_with("lv2:") || id.starts_with("vst3:") || id.starts_with("clap:") {
        return crate::native::instantiate(id, sample_rate, max_block, channels);
    }
    let (kind, subtype, maker) = parse_id(id)?;
    let component = find_component(kind, subtype, maker)?;
    let mut unit: AudioUnit = ptr::null_mut();
    if unsafe { AudioComponentInstanceNew(component, &mut unit) } != 0 || unit.is_null() {
        return None;
    }
    let channels = channels.clamp(1, 2);
    let max_block = max_block.clamp(1, 8192);
    let sample_rate = if sample_rate.is_finite() { sample_rate.max(8_000.0) } else { 48_000.0 };
    let input = Box::new(InputView { ptrs: [ptr::null(); 2], frames: 0, channels: 0 });
    let input_ptr = &*input as *const InputView as *mut c_void;
    if !prepare_unit(unit, sample_rate, max_block, channels, input_ptr) {
        unsafe {
            AudioComponentInstanceDispose(unit);
        }
        return None;
    }
    let params = read_parameters(unit);
    let name = component_name(component).unwrap_or_else(|| id.to_string());
    let latency = read_latency_samples(unit, sample_rate);
    let presets = read_factory_presets(unit);
    let mut plugin = AuPlugin {
        unit,
        name,
        params,
        presets,
        input,
        scratch: vec![vec![0.0; max_block]; channels],
        sample_rate,
        max_block,
        channels,
        editor: false,
        oversampling: 1,
        latency,
        render_stamp: 0,
        playhead: Box::new(AuPlayhead::new(f64::from(sample_rate))),
    };
    install_host_callbacks(unit, plugin.playhead.as_mut());
    Some(Box::new(plugin))
}

pub fn catalog_entry(plugin: &CatalogPlugin) -> Value {
    json!({
        "name": plugin.name,
        "company": plugin.company,
        "id": plugin.id,
        "id2": plugin.id,
        "type": plugin.format,
        "category": plugin.category,
        "isInstrument": plugin.is_instrument,
        "layouts": ["Stereo"]
    })
}

const PREFERRED: &[(&str, &str, &str, bool, &str)] = &[
    ("aufx", "lpas", "appl", false, "AULowpass"),
    ("aufx", "dely", "appl", false, "AUDelay"),
    ("aufx", "lmtr", "appl", false, "AUPeakLimiter"),
    ("aumu", "dls ", "appl", true, "DLSMusicDevice"),
];

struct AuPlugin {
    unit: AudioUnit,
    name: String,
    params: Vec<AuParam>,
    presets: Vec<AuFactoryPreset>,
    input: Box<InputView>,
    scratch: Vec<Vec<f32>>,
    sample_rate: f32,
    max_block: usize,
    channels: usize,
    editor: bool,
    oversampling: i32,
    latency: i32,
    render_stamp: i64,
    /// Heap address stays put when this plugin moves. The AudioUnit callbacks read it.
    playhead: Box<AuPlayhead>,
}

struct AuFactoryPreset {
    number: i32,
    name: String,
}

struct AuPlayhead {
    bpm: f64,
    ppq: f64,
    sample_pos: f64,
    numerator: f32,
    denominator: u32,
    playing: u8,
    recording: u8,
    looping: u8,
    changed: u8,
    sample_rate: f64,
}

impl AuPlayhead {
    fn new(sample_rate: f64) -> Self {
        Self {
            bpm: 120.0,
            ppq: 0.0,
            sample_pos: 0.0,
            numerator: 4.0,
            denominator: 4,
            playing: 0,
            recording: 0,
            looping: 0,
            changed: 0,
            sample_rate,
        }
    }
}

struct AuParam {
    id: u32,
    name: String,
    min: f32,
    max: f32,
    default_norm: f32,
}

struct InputView {
    ptrs: [*const f32; 2],
    frames: usize,
    channels: usize,
}

unsafe impl Send for AuPlugin {}

impl Drop for AuPlugin {
    fn drop(&mut self) {
        if !self.unit.is_null() {
            // Disposed on the main thread, where it was created.
            let unit = self.unit as usize;
            crate::ui::on_main(move || unsafe {
                AudioUnitUninitialize(unit as AudioUnit);
                AudioComponentInstanceDispose(unit as AudioUnit);
            });
            self.unit = ptr::null_mut();
        }
    }
}

impl Hosted for AuPlugin {
    fn editor_target(&self) -> Option<EditorTarget> {
        Some(EditorTarget::Au(self.unit as usize))
    }

    fn name(&self) -> String {
        self.name.clone()
    }

    fn has_editor(&self) -> bool {
        true
    }

    fn supports_double(&self) -> bool {
        false
    }

    fn latency(&self) -> i32 {
        self.latency
    }

    fn parameters(&self) -> Value {
        let list: Vec<Value> = self.params.iter().enumerate().map(|(index, param)| {
            json!({
                "idx": index,
                "name": param.name,
                "defaultValue": param.default_norm,
                "currentValue": self.param(index as i32),
                "category": 0,
                "label": "",
                "numSteps": 0x7fff_ffff_u32,
                "isBoolean": false,
                "isDiscrete": false,
                "isMeta": false,
                "isOrientInv": false,
                "minValue": param.min,
                "maxValue": param.max,
                "allValues": []
            })
        }).collect();
        Value::Array(list)
    }

    fn set_param(&mut self, index: i32, normalized: f32) -> bool {
        let Some(param) = self.params.get(index as usize) else {
            return false;
        };
        let raw = denorm(param, normalized);
        unsafe { AudioUnitSetParameter(self.unit, param.id, SCOPE_GLOBAL, 0, raw, 0) == 0 }
    }

    fn param(&self, index: i32) -> f32 {
        let Some(param) = self.params.get(index as usize) else {
            return 0.0;
        };
        let mut raw = 0.0f32;
        let status = unsafe { AudioUnitGetParameter(self.unit, param.id, SCOPE_GLOBAL, 0, &mut raw) };
        if status != 0 {
            return param.default_norm;
        }
        norm(param, raw)
    }

    fn set_oversampling(&mut self, factor: i32) {
        self.oversampling = match factor {
            2 => 2,
            4 => 4,
            _ => 1,
        };
    }

    fn oversampling_latency(&self) -> i32 {
        let _ = self.oversampling;
        0
    }

    fn process_f32(&mut self, audio: &mut [Vec<f32>], midi: &[[u8; 3]], frames: usize) {
        if self.unit.is_null() || frames == 0 || audio.is_empty() {
            return;
        }
        let frames = frames.min(self.max_block).min(audio.iter().map(|channel| channel.len()).min().unwrap_or(0));
        if frames == 0 {
            return;
        }
        let channels = audio.len().min(self.channels).min(2);
        if self.scratch.len() < channels || self.scratch.iter().any(|channel| channel.len() < frames) {
            self.scratch = vec![vec![0.0; frames.max(self.max_block)]; channels];
        }
        for channel in 0..channels {
            self.scratch[channel][..frames].copy_from_slice(&audio[channel][..frames]);
        }
        for event in midi {
            let status = u32::from(event[0]);
            let data1 = u32::from(event[1]);
            let data2 = u32::from(event[2]);
            unsafe {
                MusicDeviceMIDIEvent(self.unit, status, data1, data2, 0);
            }
        }
        self.input.frames = frames;
        self.input.channels = channels;
        for channel in 0..2 {
            self.input.ptrs[channel] = if channel < channels { self.scratch[channel].as_ptr() } else { ptr::null() };
        }
        let mut buffers = AudioBufferList {
            mNumberBuffers: channels as u32,
            mBuffers: [AudioBuffer { mNumberChannels: 1, mDataByteSize: 0, mData: ptr::null_mut() }; 2],
        };
        for channel in 0..channels {
            buffers.mBuffers[channel].mDataByteSize = (frames * 4) as u32;
            buffers.mBuffers[channel].mData = audio[channel].as_mut_ptr() as *mut c_void;
        }
        let mut flags = 0u32;
        let mut time = AudioTimeStamp::default();
        time.mSampleTime = self.render_stamp as f64;
        time.mFlags = 1;
        time.mRateScalar = 1.0;
        let status = unsafe { AudioUnitRender(self.unit, &mut flags, &time, 0, frames as u32, &mut buffers) };
        if status != 0 {
            for channel in 0..channels {
                audio[channel][..frames].copy_from_slice(&self.scratch[channel][..frames]);
            }
        }
        self.render_stamp += frames as i64;
    }

    fn process_f64(&mut self, audio: &mut [Vec<f64>], midi: &[[u8; 3]], frames: usize) {
        let mut temporary: Vec<Vec<f32>> = audio.iter().map(|channel| channel.iter().take(frames).map(|sample| *sample as f32).collect()).collect();
        self.process_f32(&mut temporary, midi, frames);
        for (dst, src) in audio.iter_mut().zip(&temporary) {
            for (out, value) in dst.iter_mut().zip(src) {
                *out = f64::from(*value);
            }
        }
    }

    fn show_editor(&mut self) {
        self.editor = true;
    }

    fn hide_editor(&mut self) {
        self.editor = false;
    }

    fn editor_frame(&mut self) -> Option<EditorFrame> {
        if !self.editor {
            return None;
        }
        let width = 64;
        let height = 16;
        let mut pixels = vec![0u8; width * height * 4];
        for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            let row = index / width;
            let level = if self.params.is_empty() {
                180
            } else {
                let row_param = row.min(self.params.len() - 1);
                crate::screen::gray_level(norm_current(self, &self.params[row_param]))
            };
            pixel[0] = level;
            pixel[1] = level;
            pixel[2] = level;
            pixel[3] = 255;
        }
        Some(EditorFrame { width: width as i32, height: height as i32, pixels })
    }

    fn mouse(&mut self, kind: u8, _x: f32, _y: f32, _shift: bool, _ctrl: bool, _alt: bool, delta_x: f32, delta_y: f32) {
        if self.params.is_empty() {
            return;
        }
        // MouseEvType::WHEEL. A scroll is not a click, so it does not toggle.
        if kind == 10 {
            if let Some(next) = apply_editor_wheel(self.param(0), delta_x, delta_y) {
                self.set_param(0, next);
            }
            return;
        }
        if kind != 1 {
            return;
        }
        let current = self.param(0);
        let next = if current < 0.5 { 1.0 } else { 0.0 };
        self.set_param(0, next);
    }

    fn key(&mut self, codes: &[u16]) {
        if self.params.is_empty() {
            return;
        }
        if let Some(next) = apply_editor_key(self.param(0), codes) {
            self.set_param(0, next);
        }
    }

    fn preset_names(&self) -> Vec<String> {
        self.presets.iter().map(|preset| preset.name.clone()).collect()
    }

    fn set_program(&mut self, index: i32) -> bool {
        let Some(preset) = self.presets.get(index as usize) else {
            return false;
        };
        let value = AuPreset { preset_number: preset.number, preset_name: ptr::null() };
        unsafe {
            AudioUnitSetProperty(
                self.unit,
                PROP_PRESENT_PRESET,
                SCOPE_GLOBAL,
                0,
                &value as *const AuPreset as *const c_void,
                std::mem::size_of::<AuPreset>() as u32,
            ) == 0
        }
    }

    fn set_transport(&mut self, position: &PositionInfo) {
        let play = self.playhead.as_mut();
        let playing = u8::from(position.playing);
        let recording = u8::from(position.recording);
        let looping = u8::from(position.looping);
        let sample = position.time_in_samples as f64;
        let bpm = if position.bpm.is_finite() { position.bpm } else { 120.0 };
        let ppq = if position.ppq_position.is_finite() { position.ppq_position } else { 0.0 };
        let sample = if sample.is_finite() { sample } else { 0.0 };
        let changed = play.playing != playing
            || play.recording != recording
            || play.looping != looping
            || (play.sample_pos - sample).abs() > 1.0
            || (play.ppq - ppq).abs() > 1.0e-9
            || (play.bpm - bpm).abs() > 1.0e-9;
        play.bpm = bpm;
        play.ppq = ppq;
        play.sample_pos = sample;
        play.numerator = position.time_sig_numerator.max(1) as f32;
        play.denominator = position.time_sig_denominator.max(1) as u32;
        play.playing = playing;
        play.recording = recording;
        play.looping = looping;
        play.changed = u8::from(changed);
        play.sample_rate = f64::from(self.sample_rate);
    }

    fn host_callback_transport(&self) -> Option<(f64, f64, bool, f64)> {
        let mut info = HostCallbackInfo {
            host_user_data: ptr::null_mut(),
            beat_and_tempo: None,
            musical_time: None,
            transport: None,
            transport2: None,
        };
        let mut size = std::mem::size_of::<HostCallbackInfo>() as u32;
        let status = unsafe {
            AudioUnitGetProperty(
                self.unit,
                PROP_HOST_CALLBACKS,
                SCOPE_GLOBAL,
                0,
                &mut info as *mut HostCallbackInfo as *mut c_void,
                &mut size,
            )
        };
        if status != 0 {
            return None;
        }
        let beat_and_tempo = info.beat_and_tempo?;
        let transport2 = info.transport2?;
        let mut beat = 0.0f64;
        let mut tempo = 0.0f64;
        let mut playing = 0u8;
        let mut recording = 0u8;
        let mut changed = 0u8;
        let mut sample = 0.0f64;
        let mut cycling = 0u8;
        let mut cycle_start = 0.0f64;
        let mut cycle_end = 0.0f64;
        let beat_status = unsafe { beat_and_tempo(info.host_user_data, &mut beat, &mut tempo) };
        let transport_status = unsafe {
            transport2(
                info.host_user_data,
                &mut playing,
                &mut recording,
                &mut changed,
                &mut sample,
                &mut cycling,
                &mut cycle_start,
                &mut cycle_end,
            )
        };
        if beat_status != 0 || transport_status != 0 {
            return None;
        }
        Some((beat, tempo, playing != 0, sample))
    }
}

fn norm_current(plugin: &AuPlugin, param: &AuParam) -> f32 {
    plugin.params.iter().position(|item| item.id == param.id).map(|index| plugin.param(index as i32)).unwrap_or(param.default_norm)
}

fn prepare_unit(unit: AudioUnit, sample_rate: f32, max_block: usize, channels: usize, input_ptr: *mut c_void) -> bool {
    let mut max_frames = max_block as u32;
    if unsafe { AudioUnitSetProperty(unit, PROP_MAX_FRAMES, SCOPE_GLOBAL, 0, &max_frames as *const u32 as *const c_void, 4) } != 0 {
        return false;
    }
    let mut rate = f64::from(sample_rate);
    unsafe {
        AudioUnitSetProperty(unit, PROP_SAMPLE_RATE, SCOPE_GLOBAL, 0, &rate as *const f64 as *const c_void, 8);
        let _ = rate;
    }
    let format = pcm_format(f64::from(sample_rate), channels as u32);
    if unsafe { AudioUnitSetProperty(unit, PROP_STREAM_FORMAT, SCOPE_OUTPUT, 0, &format as *const _ as *const c_void, std::mem::size_of::<AudioStreamBasicDescription>() as u32) } != 0 {
        return false;
    }
    let mut input_buses = 0u32;
    let mut size = 4u32;
    unsafe {
        AudioUnitGetProperty(unit, PROP_ELEMENT_COUNT, SCOPE_INPUT, 0, &mut input_buses as *mut u32 as *mut c_void, &mut size);
    }
    if input_buses > 0 {
        if unsafe { AudioUnitSetProperty(unit, PROP_STREAM_FORMAT, SCOPE_INPUT, 0, &format as *const _ as *const c_void, std::mem::size_of::<AudioStreamBasicDescription>() as u32) } != 0 {
            return false;
        }
        let callback = RenderCallback { inputProc: pull_input, inputProcRefCon: input_ptr };
        if unsafe { AudioUnitSetProperty(unit, PROP_RENDER_CALLBACK, SCOPE_INPUT, 0, &callback as *const _ as *const c_void, std::mem::size_of::<RenderCallback>() as u32) } != 0 {
            return false;
        }
    }
    unsafe { AudioUnitInitialize(unit) == 0 }
}

fn read_parameters(unit: AudioUnit) -> Vec<AuParam> {
    let mut size = 0u32;
    if unsafe { AudioUnitGetPropertyInfo(unit, PROP_PARAM_LIST, SCOPE_GLOBAL, 0, &mut size, ptr::null_mut()) } != 0 || size < 4 {
        return Vec::new();
    }
    let count = (size as usize) / 4;
    let mut ids = vec![0u32; count];
    if unsafe { AudioUnitGetProperty(unit, PROP_PARAM_LIST, SCOPE_GLOBAL, 0, ids.as_mut_ptr() as *mut c_void, &mut size) } != 0 {
        return Vec::new();
    }
    let mut writable = Vec::new();
    let mut others = Vec::new();
    for id in ids {
        let Some(param) = read_one_parameter(unit, id) else {
            continue;
        };
        let is_writable = param.writable;
        if is_writable {
            writable.push(param.param);
        } else {
            others.push(param.param);
        }
    }
    writable.extend(others);
    writable
}

struct ReadParam {
    param: AuParam,
    writable: bool,
}

fn read_one_parameter(unit: AudioUnit, id: u32) -> Option<ReadParam> {
    let mut info = AudioUnitParameterInfo::default();
    let mut size = std::mem::size_of::<AudioUnitParameterInfo>() as u32;
    let id_bytes = id;
    if unsafe { AudioUnitGetProperty(unit, PROP_PARAM_INFO, SCOPE_GLOBAL, id_bytes, &mut info as *mut _ as *mut c_void, &mut size) } != 0 {
        return None;
    }
    let name = parameter_name(&info);
    let default_norm = norm_range(info.min_value, info.max_value, info.default_value);
    Some(ReadParam {
        writable: info.flags & PARAM_WRITABLE != 0,
        param: AuParam { id, name, min: info.min_value, max: info.max_value, default_norm },
    })
}

fn parameter_name(info: &AudioUnitParameterInfo) -> String {
    if info.flags & PARAM_HAS_CFNAME != 0 && !info.cf_name.is_null() {
        if let Some(name) = cf_to_string(info.cf_name) {
            if !name.is_empty() {
                return name;
            }
        }
    }
    let end = info.name.iter().position(|byte| *byte == 0).unwrap_or(info.name.len());
    String::from_utf8_lossy(&info.name[..end]).trim().to_string()
}

fn read_latency_samples(unit: AudioUnit, sample_rate: f32) -> i32 {
    let mut seconds = 0.0f64;
    let mut size = 8u32;
    if unsafe { AudioUnitGetProperty(unit, PROP_LATENCY, SCOPE_GLOBAL, 0, &mut seconds as *mut f64 as *mut c_void, &mut size) } != 0 {
        return 0;
    }
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds * f64::from(sample_rate)).round().clamp(0.0, 8192.0) as i32
}

fn describe(kind: u32, subtype: u32, maker: u32, instrument: bool, fallback: &str) -> Option<CatalogPlugin> {
    let component = find_component(kind, subtype, maker)?;
    let name = component_name(component).unwrap_or_else(|| fallback.to_string());
    Some(plugin_record(name, kind, subtype, maker, instrument))
}

fn collect_type(kind: u32, instrument: bool, found: &mut Vec<CatalogPlugin>) {
    let mut description = AudioComponentDescription {
        componentType: kind,
        componentSubType: 0,
        componentManufacturer: 0,
        componentFlags: 0,
        componentFlagsMask: 0,
    };
    let mut current: AudioComponent = ptr::null_mut();
    loop {
        current = unsafe { AudioComponentFindNext(current, &description) };
        if current.is_null() {
            break;
        }
        let mut actual = AudioComponentDescription::default();
        if unsafe { AudioComponentGetDescription(current, &mut actual) } != 0 {
            continue;
        }
        let name = component_name(current).unwrap_or_else(|| fourcc_string(actual.componentSubType));
        push_unique(found, plugin_record(name, actual.componentType, actual.componentSubType, actual.componentManufacturer, instrument));
    }
}

fn plugin_record(name: String, kind: u32, subtype: u32, maker: u32, instrument: bool) -> CatalogPlugin {
    // AudioUnit names are "Company: Plugin" by convention; the maker code is only a fallback.
    let (company, name) = match name.split_once(": ") {
        Some((company, plugin)) if !company.is_empty() && !plugin.is_empty() => (company.to_string(), plugin.to_string()),
        _ => (fourcc_string(maker), name),
    };
    CatalogPlugin {
        name,
        company,
        id: format!("au:{}:{}:{}", fourcc_string(kind), fourcc_string(subtype), fourcc_string(maker)),
        format: "AudioUnit".to_string(),
        is_instrument: instrument,
        category: if instrument { "Instrument".to_string() } else { "Effect".to_string() },
    }
}

fn push_unique(found: &mut Vec<CatalogPlugin>, plugin: CatalogPlugin) {
    if found.iter().any(|existing| existing.id == plugin.id) {
        return;
    }
    found.push(plugin);
}

fn find_component(kind: u32, subtype: u32, maker: u32) -> Option<AudioComponent> {
    let description = AudioComponentDescription {
        componentType: kind,
        componentSubType: subtype,
        componentManufacturer: maker,
        componentFlags: 0,
        componentFlagsMask: 0,
    };
    let component = unsafe { AudioComponentFindNext(ptr::null_mut(), &description) };
    if component.is_null() { None } else { Some(component) }
}

fn component_name(component: AudioComponent) -> Option<String> {
    let mut text: *const c_void = ptr::null();
    if unsafe { AudioComponentCopyName(component, &mut text) } != 0 || text.is_null() {
        return None;
    }
    let name = cf_to_string(text);
    unsafe { CFRelease(text) };
    name
}

fn cf_to_string(text: *const c_void) -> Option<String> {
    let mut buffer = [0u8; 256];
    let ok = unsafe { CFStringGetCString(text, buffer.as_mut_ptr() as *mut c_char, buffer.len() as isize, CF_UTF8) };
    if ok == 0 {
        return None;
    }
    let end = buffer.iter().position(|byte| *byte == 0).unwrap_or(buffer.len());
    Some(String::from_utf8_lossy(&buffer[..end]).to_string())
}

fn parse_id(id: &str) -> Option<(u32, u32, u32)> {
    let mut parts = id.splitn(4, ':');
    if parts.next()? != "au" {
        return None;
    }
    let kind = parts.next()?;
    let subtype = parts.next()?;
    let maker = parts.next()?;
    Some((fourcc(kind), fourcc(subtype), fourcc(maker)))
}

fn fourcc(text: &str) -> u32 {
    fourcc_bytes(text.as_bytes())
}

fn fourcc_bytes(bytes: &[u8]) -> u32 {
    let mut raw = [b' '; 4];
    for (index, byte) in bytes.iter().take(4).enumerate() {
        raw[index] = *byte;
    }
    u32::from_be_bytes(raw)
}

fn fourcc_string(value: u32) -> String {
    String::from_utf8_lossy(&value.to_be_bytes()).to_string()
}

fn pcm_format(sample_rate: f64, channels: u32) -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        mSampleRate: sample_rate,
        mFormatID: fourcc("lpcm"),
        mFormatFlags: FLAG_FLOAT | FLAG_PACKED | FLAG_NONINTERLEAVED,
        mBytesPerPacket: 4,
        mFramesPerPacket: 1,
        mBytesPerFrame: 4,
        mChannelsPerFrame: channels,
        mBitsPerChannel: 32,
        mReserved: 0,
    }
}

fn norm(param: &AuParam, raw: f32) -> f32 {
    norm_range(param.min, param.max, raw)
}

fn norm_range(min: f32, max: f32, raw: f32) -> f32 {
    if !min.is_finite() || !max.is_finite() || max <= min {
        return 0.0;
    }
    ((raw - min) / (max - min)).clamp(0.0, 1.0)
}

fn denorm(param: &AuParam, normalized: f32) -> f32 {
    let span = param.max - param.min;
    param.min + span * normalized.clamp(0.0, 1.0)
}

unsafe extern "C" fn pull_input(
    refcon: *mut c_void,
    _flags: *mut u32,
    _time: *const AudioTimeStamp,
    _bus: u32,
    frames: u32,
    data: *mut AudioBufferList,
) -> i32 {
    if refcon.is_null() || data.is_null() {
        return -1;
    }
    let view = &*refcon.cast::<InputView>();
    let list = &mut *data;
    let channels = (list.mNumberBuffers as usize).min(view.channels).min(2);
    let frames = (frames as usize).min(view.frames);
    for channel in 0..channels {
        let buffer = &mut list.mBuffers[channel];
        if buffer.mData.is_null() || view.ptrs[channel].is_null() {
            continue;
        }
        let dst = std::slice::from_raw_parts_mut(buffer.mData as *mut f32, frames);
        let src = std::slice::from_raw_parts(view.ptrs[channel], frames);
        dst.copy_from_slice(src);
    }
    0
}

fn read_factory_presets(unit: AudioUnit) -> Vec<AuFactoryPreset> {
    let mut array: *const c_void = ptr::null();
    let mut size = std::mem::size_of::<*const c_void>() as u32;
    let status = unsafe {
        AudioUnitGetProperty(
            unit,
            PROP_FACTORY_PRESETS,
            SCOPE_GLOBAL,
            0,
            &mut array as *mut *const c_void as *mut c_void,
            &mut size,
        )
    };
    if status != 0 || array.is_null() {
        return Vec::new();
    }
    let count = unsafe { CFArrayGetCount(array) }.max(0);
    let mut presets = Vec::new();
    for index in 0..count {
        if presets.len() >= 256 {
            break;
        }
        let item = unsafe { CFArrayGetValueAtIndex(array, index) };
        if item.is_null() {
            continue;
        }
        let preset = unsafe { &*item.cast::<AuPreset>() };
        let name = if preset.preset_name.is_null() {
            String::new()
        } else {
            cf_to_string(preset.preset_name).unwrap_or_default()
        };
        let name = name.replace('|', "/");
        if name.is_empty() {
            continue;
        }
        presets.push(AuFactoryPreset { number: preset.preset_number, name });
    }
    unsafe { CFRelease(array) };
    presets
}

fn install_host_callbacks(unit: AudioUnit, playhead: *mut AuPlayhead) {
    let info = HostCallbackInfo {
        host_user_data: playhead.cast(),
        beat_and_tempo: Some(host_beat_and_tempo),
        musical_time: Some(host_musical_time),
        transport: Some(host_transport),
        transport2: Some(host_transport2),
    };
    unsafe {
        AudioUnitSetProperty(
            unit,
            PROP_HOST_CALLBACKS,
            SCOPE_GLOBAL,
            0,
            &info as *const HostCallbackInfo as *const c_void,
            std::mem::size_of::<HostCallbackInfo>() as u32,
        );
    }
}

unsafe extern "C" fn host_beat_and_tempo(user: *mut c_void, beat: *mut f64, tempo: *mut f64) -> i32 {
    if user.is_null() {
        return -1;
    }
    let play = &*user.cast::<AuPlayhead>();
    if !beat.is_null() {
        *beat = play.ppq;
    }
    if !tempo.is_null() {
        *tempo = play.bpm;
    }
    0
}

unsafe extern "C" fn host_musical_time(user: *mut c_void, delta: *mut u32, numerator: *mut f32, denominator: *mut u32, downbeat: *mut f64) -> i32 {
    if user.is_null() {
        return -1;
    }
    let play = &*user.cast::<AuPlayhead>();
    if !delta.is_null() {
        *delta = samples_until_next_beat(play);
    }
    if !numerator.is_null() {
        *numerator = play.numerator;
    }
    if !denominator.is_null() {
        *denominator = play.denominator;
    }
    if !downbeat.is_null() {
        *downbeat = measure_downbeat(play);
    }
    0
}

unsafe extern "C" fn host_transport(
    user: *mut c_void,
    playing: *mut u8,
    changed: *mut u8,
    sample: *mut f64,
    cycling: *mut u8,
    cycle_start: *mut f64,
    cycle_end: *mut f64,
) -> i32 {
    if user.is_null() {
        return -1;
    }
    let play = &*user.cast::<AuPlayhead>();
    if !playing.is_null() {
        *playing = play.playing;
    }
    if !changed.is_null() {
        *changed = play.changed;
    }
    if !sample.is_null() {
        *sample = play.sample_pos;
    }
    if !cycling.is_null() {
        *cycling = play.looping;
    }
    if !cycle_start.is_null() {
        *cycle_start = 0.0;
    }
    if !cycle_end.is_null() {
        *cycle_end = 0.0;
    }
    0
}

unsafe extern "C" fn host_transport2(
    user: *mut c_void,
    playing: *mut u8,
    recording: *mut u8,
    changed: *mut u8,
    sample: *mut f64,
    cycling: *mut u8,
    cycle_start: *mut f64,
    cycle_end: *mut f64,
) -> i32 {
    if user.is_null() {
        return -1;
    }
    let play = &*user.cast::<AuPlayhead>();
    if !playing.is_null() {
        *playing = play.playing;
    }
    if !recording.is_null() {
        *recording = play.recording;
    }
    if !changed.is_null() {
        *changed = play.changed;
    }
    if !sample.is_null() {
        *sample = play.sample_pos;
    }
    if !cycling.is_null() {
        *cycling = play.looping;
    }
    if !cycle_start.is_null() {
        *cycle_start = 0.0;
    }
    if !cycle_end.is_null() {
        *cycle_end = 0.0;
    }
    0
}

fn samples_until_next_beat(play: &AuPlayhead) -> u32 {
    let denominator = play.denominator.max(1) as f64;
    let beat_quarters = 4.0 / denominator;
    if !(play.bpm.is_finite() && play.bpm > 0.0 && play.sample_rate.is_finite() && play.sample_rate > 0.0) {
        return 0;
    }
    let beat = play.ppq / beat_quarters;
    let fraction = beat - beat.floor();
    if fraction < 1.0e-8 {
        return 0;
    }
    let samples_per_beat = play.sample_rate * 60.0 / play.bpm * beat_quarters;
    let samples = (1.0 - fraction) * samples_per_beat;
    if !samples.is_finite() || samples <= 0.0 {
        return 0;
    }
    samples.round().min(u32::MAX as f64) as u32
}

fn measure_downbeat(play: &AuPlayhead) -> f64 {
    let denominator = play.denominator.max(1) as f64;
    let numerator = f64::from(play.numerator.max(1.0));
    let beat = play.ppq / (4.0 / denominator);
    (beat / numerator).floor() * numerator
}

const SCOPE_GLOBAL: u32 = 0;
const SCOPE_INPUT: u32 = 1;
const SCOPE_OUTPUT: u32 = 2;
const PROP_SAMPLE_RATE: u32 = 2;
const PROP_PARAM_LIST: u32 = 3;
const PROP_PARAM_INFO: u32 = 4;
const PROP_STREAM_FORMAT: u32 = 8;
const PROP_ELEMENT_COUNT: u32 = 11;
const PROP_LATENCY: u32 = 12;
const PROP_MAX_FRAMES: u32 = 14;
const PROP_RENDER_CALLBACK: u32 = 23;
const PROP_FACTORY_PRESETS: u32 = 24;
const PROP_HOST_CALLBACKS: u32 = 27;
const PROP_PRESENT_PRESET: u32 = 36;
const PARAM_HAS_CFNAME: u32 = 1 << 27;
const PARAM_WRITABLE: u32 = 1 << 31;
const FLAG_FLOAT: u32 = 1;
const FLAG_PACKED: u32 = 1 << 3;
const FLAG_NONINTERLEAVED: u32 = 1 << 5;
const CF_UTF8: u32 = 0x0800_0100;

type AudioComponent = *mut c_void;
type AudioUnit = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioComponentDescription {
    componentType: u32,
    componentSubType: u32,
    componentManufacturer: u32,
    componentFlags: u32,
    componentFlagsMask: u32,
}

impl Default for AudioComponentDescription {
    fn default() -> Self {
        Self { componentType: 0, componentSubType: 0, componentManufacturer: 0, componentFlags: 0, componentFlagsMask: 0 }
    }
}

#[repr(C)]
struct AudioStreamBasicDescription {
    mSampleRate: f64,
    mFormatID: u32,
    mFormatFlags: u32,
    mBytesPerPacket: u32,
    mFramesPerPacket: u32,
    mBytesPerFrame: u32,
    mChannelsPerFrame: u32,
    mBitsPerChannel: u32,
    mReserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioBuffer {
    mNumberChannels: u32,
    mDataByteSize: u32,
    mData: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    mNumberBuffers: u32,
    mBuffers: [AudioBuffer; 2],
}

#[repr(C)]
struct AudioTimeStamp {
    mSampleTime: f64,
    mHostTime: u64,
    mRateScalar: f64,
    mWordClockTime: u64,
    mSmpte: [u8; 24],
    mFlags: u32,
    mReserved: u32,
}

impl Default for AudioTimeStamp {
    fn default() -> Self {
        Self {
            mSampleTime: 0.0,
            mHostTime: 0,
            mRateScalar: 1.0,
            mWordClockTime: 0,
            mSmpte: [0; 24],
            mFlags: 0,
            mReserved: 0,
        }
    }
}

#[repr(C)]
struct RenderCallback {
    inputProc: unsafe extern "C" fn(*mut c_void, *mut u32, *const AudioTimeStamp, u32, u32, *mut AudioBufferList) -> i32,
    inputProcRefCon: *mut c_void,
}

#[repr(C)]
struct AudioUnitParameterInfo {
    name: [u8; 52],
    unit_name: *const c_void,
    clump_id: u32,
    cf_name: *const c_void,
    unit: u32,
    min_value: f32,
    max_value: f32,
    default_value: f32,
    flags: u32,
}

impl Default for AudioUnitParameterInfo {
    fn default() -> Self {
        Self {
            name: [0; 52],
            unit_name: ptr::null(),
            clump_id: 0,
            cf_name: ptr::null(),
            unit: 0,
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.0,
            flags: 0,
        }
    }
}

#[repr(C)]
struct AuPreset {
    preset_number: i32,
    preset_name: *const c_void,
}

#[repr(C)]
struct HostCallbackInfo {
    host_user_data: *mut c_void,
    beat_and_tempo: Option<unsafe extern "C" fn(*mut c_void, *mut f64, *mut f64) -> i32>,
    musical_time: Option<unsafe extern "C" fn(*mut c_void, *mut u32, *mut f32, *mut u32, *mut f64) -> i32>,
    transport: Option<unsafe extern "C" fn(*mut c_void, *mut u8, *mut u8, *mut f64, *mut u8, *mut f64, *mut f64) -> i32>,
    transport2: Option<unsafe extern "C" fn(*mut c_void, *mut u8, *mut u8, *mut u8, *mut f64, *mut u8, *mut f64, *mut f64) -> i32>,
}

#[link(name = "AudioToolbox", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn AudioComponentFindNext(component: AudioComponent, description: *const AudioComponentDescription) -> AudioComponent;
    fn AudioComponentInstanceNew(component: AudioComponent, instance: *mut AudioUnit) -> i32;
    fn AudioComponentInstanceDispose(instance: AudioUnit) -> i32;
    fn AudioComponentGetDescription(component: AudioComponent, description: *mut AudioComponentDescription) -> i32;
    fn AudioComponentCopyName(component: AudioComponent, name: *mut *const c_void) -> i32;
    fn AudioUnitInitialize(unit: AudioUnit) -> i32;
    fn AudioUnitUninitialize(unit: AudioUnit) -> i32;
    fn AudioUnitSetProperty(unit: AudioUnit, property: u32, scope: u32, element: u32, data: *const c_void, size: u32) -> i32;
    fn AudioUnitGetProperty(unit: AudioUnit, property: u32, scope: u32, element: u32, data: *mut c_void, size: *mut u32) -> i32;
    fn AudioUnitGetPropertyInfo(unit: AudioUnit, property: u32, scope: u32, element: u32, size: *mut u32, writable: *mut u8) -> i32;
    fn AudioUnitSetParameter(unit: AudioUnit, parameter: u32, scope: u32, element: u32, value: f32, offset: u32) -> i32;
    fn AudioUnitGetParameter(unit: AudioUnit, parameter: u32, scope: u32, element: u32, value: *mut f32) -> i32;
    fn AudioUnitRender(unit: AudioUnit, flags: *mut u32, time: *const AudioTimeStamp, bus: u32, frames: u32, data: *mut AudioBufferList) -> i32;
    fn MusicDeviceMIDIEvent(unit: AudioUnit, status: u32, data1: u32, data2: u32, offset: u32) -> i32;
    fn CFStringGetCString(text: *const c_void, buffer: *mut c_char, length: isize, encoding: u32) -> u8;
    fn CFRelease(value: *const c_void);
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effect_id() -> String {
        let list = scan();
        list.iter()
            .find(|plugin| !plugin.is_instrument && (plugin.id.contains("lpas") || plugin.id.contains("dely") || plugin.id.contains("lmtr")))
            .map(|plugin| plugin.id.clone())
            .unwrap_or_else(|| panic!("scan has no Apple lowpass, delay, or limiter: {list:?}"))
    }

    #[test]
    fn scan_includes_an_apple_effect() {
        let _ = effect_id();
    }

    #[test]
    fn apple_effect_is_not_a_copy_of_the_input() {
        let id = effect_id();
        let mut plugin = instantiate(&id, 48_000.0, 256, 2).unwrap_or_else(|| panic!("instantiate {id}"));
        let signal: Vec<f32> = (0..256).map(|index| (index as f32 * 0.17).sin() * 0.8).collect();
        let input = vec![signal.clone(), signal.clone()];
        let mut heard = false;
        let count = plugin.parameters().as_array().map(|items| items.len()).unwrap_or(0).min(8);
        assert!(count > 0, "{id} published no parameters");
        for index in 0..count as i32 {
            plugin.set_param(index, 0.0);
            let mut low = input.clone();
            plugin.process_f32(&mut low, &[], 256);
            plugin.set_param(index, 1.0);
            let mut high = input.clone();
            plugin.process_f32(&mut high, &[], 256);
            let low_diff = max_diff(&low[0], &signal);
            let high_diff = max_diff(&high[0], &signal);
            let between = max_diff(&low[0], &high[0]);
            if low_diff > 1.0e-3 || high_diff > 1.0e-3 || between > 1.0e-3 {
                heard = true;
                break;
            }
        }
        assert!(heard, "{id} output copied the input for every parameter");
    }

    #[test]
    fn dls_note_on_is_not_silent() {
        let list = scan();
        let Some(instrument) = list.iter().find(|plugin| plugin.is_instrument && plugin.name.to_ascii_lowercase().contains("dls"))
            .or_else(|| list.iter().find(|plugin| plugin.is_instrument))
        else {
            panic!("scan has no instrument: {list:?}");
        };
        let mut plugin = instantiate(&instrument.id, 48_000.0, 256, 2).unwrap_or_else(|| panic!("instantiate {}", instrument.id));
        let silence = vec![vec![0.0f32; 256], vec![0.0; 256]];
        let note = [[0x90u8, 60, 100]];
        let mut peak = 0.0f32;
        for block in 0..4 {
            let mut audio = silence.clone();
            let midi: &[[u8; 3]] = if block == 0 { &note } else { &[] };
            plugin.process_f32(&mut audio, midi, 256);
            peak = peak.max(audio.iter().flatten().copied().map(f32::abs).fold(0.0, f32::max));
        }
        assert!(peak > 1.0e-4, "MIDI note-on on {} stayed silent (peak {peak})", instrument.id);
    }

    #[test]
    fn apple_effect_host_callbacks_report_the_block_position() {
        let id = effect_id();
        let mut plugin = instantiate(&id, 48_000.0, 64, 2).unwrap_or_else(|| panic!("instantiate {id}"));
        let mut position = PositionInfo::default();
        position.bpm = 173.0;
        position.ppq_position = 3.25;
        position.time_in_samples = 999;
        position.playing = true;
        plugin.set_transport(&position);
        let mut audio = vec![vec![0.0f32; 64], vec![0.0; 64]];
        plugin.process_f32(&mut audio, &[], 64);
        let (beat, tempo, playing, sample) = plugin.host_callback_transport().unwrap_or_else(|| panic!("{id} did not expose host callbacks"));
        eprintln!("host callbacks beat {beat} tempo {tempo} playing {playing} sample {sample}");
        assert!((beat - 3.25).abs() < 1.0e-9, "beat {beat}");
        assert!((tempo - 173.0).abs() < 1.0e-9, "tempo {tempo}");
        assert!(playing, "transport was not playing");
        assert!((sample - 999.0).abs() < 1.0e-6, "timeline sample {sample}");
    }

    fn max_diff(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(left, right)| (left - right).abs()).fold(0.0, f32::max)
    }
}
