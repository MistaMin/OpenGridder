//! Rust ownership and processing layer for the external VST3 SDK ABI adapter.
use crate::{HostedPlugin, HostedPluginInstance};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use og_common::{
    message::{MidiEvent, PositionInfo},
    server_plugin::{PluginParameter, ServerPlugin},
};
use std::{
    ffi::{c_char, c_void, CString},
    path::Path,
    ptr,
};
#[repr(C)]
struct Class {
    cid: [u8; 16],
    name: [c_char; 64],
    vendor: [c_char; 64],
    version: [c_char; 64],
    category: [c_char; 128],
}
#[repr(C)]
struct Parameter {
    id: u32,
    steps: i32,
    flags: i32,
    default: f64,
    current: f64,
    name: [u16; 128],
    units: [u16; 128],
}
unsafe extern "C" {
    fn og_vst3_scan(
        path: *const c_char,
        classes: *mut Class,
        capacity: i32,
        error: *mut c_char,
    ) -> i32;
    fn og_vst3_create(path: *const c_char, cid: *const u8, error: *mut c_char) -> *mut c_void;
    fn og_vst3_destroy(handle: *mut c_void);
    fn og_vst3_prepare(
        handle: *mut c_void,
        rate: f64,
        frames: i32,
        channels: i32,
        error: *mut c_char,
    ) -> i32;
    fn og_vst3_stop(handle: *mut c_void);
    fn og_vst3_parameter_count(handle: *mut c_void) -> i32;
    fn og_vst3_parameter(handle: *mut c_void, index: i32, parameter: *mut Parameter) -> i32;
    fn og_vst3_get_parameter(handle: *mut c_void, index: i32) -> f64;
    fn og_vst3_set_parameter(handle: *mut c_void, index: i32, value: f64) -> i32;
    fn og_vst3_process(
        handle: *mut c_void,
        input: *const *mut f32,
        output: *const *mut f32,
        frames: i32,
        position: i64,
        ppq: f64,
        bpm: f64,
        numerator: i32,
        denominator: i32,
        transport: u32,
    ) -> i32;
    fn og_vst3_latency(handle: *mut c_void) -> u32;
    fn og_vst3_get_state(handle: *mut c_void, bytes: *mut *mut u8, length: *mut usize) -> i32;
    fn og_vst3_free_state(bytes: *mut u8);
    fn og_vst3_set_state(handle: *mut c_void, bytes: *const u8, length: usize) -> i32;
}
fn c_text(bytes: &[c_char]) -> String {
    let bytes: Vec<_> = bytes
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}
fn utf16(bytes: &[u16]) -> String {
    String::from_utf16_lossy(&bytes[..bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len())])
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

pub fn scan_bundle(path: &Path) -> Result<Vec<ServerPlugin>, String> {
    let module_path = std::fs::canonicalize(path)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .into_owned();
    let path = CString::new(module_path.as_str()).map_err(|e| e.to_string())?;
    let mut classes: Vec<Class> = (0..256).map(|_| unsafe { std::mem::zeroed() }).collect();
    let mut error = [0; 512];
    let count = unsafe {
        og_vst3_scan(
            path.as_ptr(),
            classes.as_mut_ptr(),
            classes.len() as i32,
            error.as_mut_ptr(),
        )
    };
    if count < 0 {
        return Err(c_text(&error));
    }
    let mut plugins = Vec::new();
    for class in classes.iter().take(count as usize) {
        let cid = hex(&class.cid);
        let category = c_text(&class.category);
        let instrument = category.split('|').any(|s| s == "Instrument");
        let mut desc = ServerPlugin::new(
            c_text(&class.name),
            c_text(&class.vendor),
            format!("VST3:{cid}"),
            "",
            "VST3",
            category,
            instrument,
            vec!["Stereo".into()],
        );
        desc.version = c_text(&class.version);
        desc.module_path = module_path.clone();
        desc.has_editor = false;
        desc.presets.clear();
        plugins.push(desc);
    }
    Ok(plugins)
}

pub struct Vst3Plugin {
    handle: *mut c_void,
    desc: ServerPlugin,
    // All render storage is allocated during prepare, not during rendering.
    input: Vec<Vec<f32>>,
    output: Vec<Vec<f32>>,
    max_frames: usize,
    suspended: bool,
    failed: bool,
}
// The enclosing processor serializes command and audio calls; never shared directly.
unsafe impl Send for Vst3Plugin {}
impl Vst3Plugin {
    pub fn new(desc: &ServerPlugin) -> Result<Self, String> {
        let cid = desc
            .id
            .strip_prefix("VST3:")
            .ok_or("VST3 identifier must contain a factory class ID")?;
        if cid.len() != 32 || !cid.is_ascii() {
            return Err("Invalid VST3 class ID".into());
        }
        let mut bytes = [0u8; 16];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&cid[index * 2..index * 2 + 2], 16)
                .map_err(|e| e.to_string())?;
        }
        if desc.module_path.is_empty() {
            return Err("VST3 module path is absent; rescan the server catalog".into());
        }
        if desc.is_instrument {
            return Err("VST3 instrument/MIDI hosting is not implemented".into());
        }
        if desc.num_input_ch != desc.num_output_ch || !(1..=2).contains(&desc.num_output_ch) {
            return Err(
                "VST3 host supports mono/stereo effects with matching input/output channels".into(),
            );
        }
        let path = CString::new(desc.module_path.as_str()).map_err(|e| e.to_string())?;
        let mut error = [0; 512];
        let handle = unsafe { og_vst3_create(path.as_ptr(), bytes.as_ptr(), error.as_mut_ptr()) };
        if handle.is_null() {
            return Err(c_text(&error));
        }
        let mut plugin = Self {
            handle,
            desc: desc.clone(),
            input: Vec::new(),
            output: Vec::new(),
            max_frames: 0,
            suspended: false,
            failed: false,
        };
        plugin.desc.parameters.clear();
        let count = unsafe { og_vst3_parameter_count(handle) };
        if !(0..=65536).contains(&count) {
            return Err("VST3 returned invalid parameter count".into());
        }
        for index in 0..count {
            let mut param: Parameter = unsafe { std::mem::zeroed() };
            if unsafe { og_vst3_parameter(handle, index, &mut param) } != 0 {
                return Err("Could not read VST3 parameter metadata".into());
            }
            plugin.desc.parameters.push(PluginParameter {
                idx: index,
                name: utf16(&param.name),
                label: utf16(&param.units),
                default_value: param.default as f32,
                current_value: param.current as f32,
                num_steps: if param.steps > 0 {
                    param.steps.saturating_add(1)
                } else {
                    i32::MAX
                },
                is_boolean: param.steps == 1,
                is_discrete: param.steps > 0,
                ..Default::default()
            });
        }
        plugin.desc.has_editor = false;
        plugin.desc.presets.clear();
        Ok(plugin)
    }
    fn prepare(&mut self, rate: f64, frames: usize) -> Result<(), String> {
        if !rate.is_finite()
            || !(8000.0..=768000.0).contains(&rate)
            || frames == 0
            || frames > 32768
        {
            return Err("Unsupported VST3 sample rate/block size".into());
        }
        let mut error = [0; 512];
        if unsafe {
            og_vst3_prepare(
                self.handle,
                rate,
                frames as i32,
                self.desc.num_output_ch,
                error.as_mut_ptr(),
            )
        } != 0
        {
            self.max_frames = 0;
            return Err(c_text(&error));
        }
        self.input = vec![vec![0.0; frames]; self.desc.num_input_ch as usize];
        self.output = vec![vec![0.0; frames]; self.desc.num_output_ch as usize];
        self.max_frames = frames;
        self.failed = false;
        Ok(())
    }
    fn render(&mut self, frames: usize, position: &PositionInfo) -> bool {
        let mut input = [ptr::null_mut(); 2];
        let mut output = [ptr::null_mut(); 2];
        for (pointer, buffer) in input.iter_mut().zip(&mut self.input) {
            *pointer = buffer.as_mut_ptr();
        }
        for (pointer, buffer) in output.iter_mut().zip(&mut self.output) {
            *pointer = buffer.as_mut_ptr();
        }
        let transport = if position.is_playing { 1 << 1 } else { 0 }
            | if position.is_looping { 1 << 2 } else { 0 }
            | if position.is_recording { 1 << 3 } else { 0 };
        let result = unsafe {
            og_vst3_process(
                self.handle,
                input.as_ptr(),
                output.as_ptr(),
                frames as i32,
                position.time_in_samples,
                position.ppq_position,
                position.bpm,
                position.time_sig_numerator,
                position.time_sig_denominator,
                transport,
            )
        };
        if result != 0 && !self.failed {
            log::error!("{}: VST3 render failed", self.name());
        }
        self.failed = result != 0;
        result == 0
    }
}
impl Drop for Vst3Plugin {
    fn drop(&mut self) {
        unsafe {
            og_vst3_destroy(self.handle);
        }
    }
}
impl HostedPlugin for Vst3Plugin {
    fn descriptor(&self) -> &ServerPlugin {
        &self.desc
    }
    fn name(&self) -> &str {
        &self.desc.name
    }
    fn prepare_to_play(&mut self, rate: f64, frames: usize) {
        if let Err(error) = self.prepare(rate, frames) {
            log::error!("{}: {error}", self.name());
        }
    }
    fn try_prepare_to_play(&mut self, rate: f64, frames: usize) -> Result<(), String> {
        self.prepare(rate, frames)
    }
    fn release_resources(&mut self) {
        unsafe {
            og_vst3_stop(self.handle);
        }
        self.max_frames = 0;
    }
    fn is_suspended(&self) -> bool {
        self.suspended
    }
    fn suspend_processing(&mut self, suspended: bool) {
        self.suspended = suspended;
    }
    fn supports_double_precision(&self) -> bool {
        false
    }
    fn latency_samples(&self) -> i32 {
        unsafe { og_vst3_latency(self.handle) }.min(i32::MAX as u32) as i32
    }
    fn set_latency_samples(&mut self, _samples: i32) {}
    fn tail_seconds(&self) -> f64 {
        0.0
    }
    fn num_input_channels(&self) -> usize {
        self.desc.num_input_ch as usize
    }
    fn num_output_channels(&self) -> usize {
        self.desc.num_output_ch as usize
    }
    fn enable_all_buses(&mut self) {}
    fn check_buses_layout_supported(&self, inputs: usize, outputs: usize) -> bool {
        inputs == self.num_input_channels() && outputs == self.num_output_channels()
    }
    fn process_block_f32(
        &mut self,
        audio: &mut [Vec<f32>],
        _midi: &mut Vec<MidiEvent>,
        pos: &PositionInfo,
    ) {
        if self.suspended {
            return;
        }
        let frames = audio.first().map_or(0, Vec::len);
        if frames == 0 {
            return;
        }
        if frames > self.max_frames
            || audio.len() != self.input.len()
            || audio.iter().any(|ch| ch.len() != frames)
        {
            for ch in audio {
                ch.fill(0.0);
            }
            return;
        }
        for (source, target) in audio.iter().zip(&mut self.input) {
            target[..frames].copy_from_slice(source);
        }
        if self.render(frames, pos) {
            for (target, source) in audio.iter_mut().zip(&self.output) {
                target.copy_from_slice(&source[..frames]);
            }
        } else {
            for ch in audio {
                ch.fill(0.0);
            }
        }
    }
    fn process_block_f64(
        &mut self,
        audio: &mut [Vec<f64>],
        _midi: &mut Vec<MidiEvent>,
        pos: &PositionInfo,
    ) {
        if self.suspended {
            return;
        }
        let frames = audio.first().map_or(0, Vec::len);
        if frames == 0 {
            return;
        }
        if frames > self.max_frames
            || audio.len() != self.input.len()
            || audio.iter().any(|ch| ch.len() != frames)
        {
            for ch in audio {
                ch.fill(0.0);
            }
            return;
        }
        for (source, target) in audio.iter().zip(&mut self.input) {
            for (s, d) in source.iter().zip(target) {
                *d = *s as f32;
            }
        }
        if self.render(frames, pos) {
            for (target, source) in audio.iter_mut().zip(&self.output) {
                for (d, s) in target.iter_mut().zip(source) {
                    *d = *s as f64;
                }
            }
        } else {
            for ch in audio {
                ch.fill(0.0);
            }
        }
    }
    fn parameters(&self) -> &[PluginParameter] {
        &self.desc.parameters
    }
    fn get_parameter(&self, index: usize) -> f32 {
        if index >= self.desc.parameters.len() {
            return 0.0;
        }
        (unsafe { og_vst3_get_parameter(self.handle, index as i32) }) as f32
    }
    fn set_parameter(&mut self, index: usize, value: f32) {
        if index < self.desc.parameters.len()
            && value.is_finite()
            && unsafe { og_vst3_set_parameter(self.handle, index as i32, value as f64) } == 0
        {
            self.desc.parameters[index].current_value = value.clamp(0.0, 1.0);
        }
    }
    fn get_parameter_text(&self, index: usize) -> String {
        format!("{:.4}", self.get_parameter(index))
    }
    fn num_programs(&self) -> usize {
        0
    }
    fn get_program_name(&self, _index: usize) -> String {
        String::new()
    }
    fn current_program(&self) -> usize {
        0
    }
    fn set_current_program(&mut self, _index: usize) {}
    fn get_state_information(&self) -> Vec<u8> {
        let mut bytes = ptr::null_mut();
        let mut length = 0;
        let result = unsafe { og_vst3_get_state(self.handle, &mut bytes, &mut length) };
        let component = if result == 0 && !bytes.is_null() && length <= 32 * 1024 * 1024 {
            BASE64.encode(unsafe { std::slice::from_raw_parts(bytes, length) })
        } else {
            String::new()
        };
        unsafe {
            og_vst3_free_state(bytes);
        }
        if component.is_empty() {
            return Vec::new();
        }
        let parameters: Vec<_> = (0..self.desc.parameters.len())
            .map(|i| self.get_parameter(i))
            .collect();
        serde_json::to_vec(&serde_json::json!({"version":1,"plugin":self.desc.id,"component":component,"parameters":parameters})).unwrap_or_default()
    }
    fn set_state_information(&mut self, bytes: &[u8]) -> Result<(), String> {
        #[derive(serde::Deserialize)]
        struct State {
            version: u32,
            plugin: String,
            component: String,
            parameters: Vec<f32>,
        }
        if bytes.is_empty() || bytes.len() > 48 * 1024 * 1024 {
            return Err("Invalid VST3 state size".into());
        }
        let state: State =
            serde_json::from_slice(bytes).map_err(|e| format!("Invalid VST3 state: {e}"))?;
        if state.version != 1
            || state.plugin != self.desc.id
            || state.parameters.len() != self.desc.parameters.len()
            || state
                .parameters
                .iter()
                .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
        {
            return Err("VST3 state plugin/version/parameters do not match".into());
        }
        let component = BASE64.decode(state.component).map_err(|e| e.to_string())?;
        if unsafe { og_vst3_set_state(self.handle, component.as_ptr(), component.len()) } != 0 {
            return Err("VST3 plugin rejected component state".into());
        }
        for (index, value) in state.parameters.into_iter().enumerate() {
            self.set_parameter(index, value);
        }
        Ok(())
    }
    fn has_editor(&self) -> bool {
        false
    }
    fn editor_size(&self) -> (u32, u32) {
        (640, 360)
    }
    fn render_editor_rgba(&self) -> (u32, u32, Vec<u8>) {
        let mut desc = self.desc.clone();
        for (index, param) in desc.parameters.iter_mut().enumerate() {
            param.current_value = self.get_parameter(index);
        }
        HostedPluginInstance::new(desc).render_editor_rgba()
    }
}
