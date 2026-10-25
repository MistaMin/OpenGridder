//! LV2 (`.lv2` bundle + `manifest.ttl` + `lv2_descriptor` C ABI) scanner and host.
//! Zero JUCE dependency.

use crate::vst3::default_plugin_parameters;
use crate::HostedPlugin;
use og_common::server_plugin::{PluginLayout, ServerPlugin};
use std::path::{Path, PathBuf};

/// LV2 C ABI descriptor (`lv2.h`).
#[repr(C)]
pub struct LV2Descriptor {
    pub uri: *const std::os::raw::c_char,
    pub instantiate: Option<
        unsafe extern "C" fn(
            descriptor: *const LV2Descriptor,
            sample_rate: f64,
            bundle_path: *const std::os::raw::c_char,
            features: *const *const std::ffi::c_void,
        ) -> *mut std::ffi::c_void,
    >,
    pub connect_port: Option<
        unsafe extern "C" fn(
            instance: *mut std::ffi::c_void,
            port: u32,
            data_location: *mut std::ffi::c_void,
        ),
    >,
    pub activate: Option<unsafe extern "C" fn(instance: *mut std::ffi::c_void)>,
    pub run: Option<unsafe extern "C" fn(instance: *mut std::ffi::c_void, sample_count: u32)>,
    pub deactivate: Option<unsafe extern "C" fn(instance: *mut std::ffi::c_void)>,
    pub cleanup: Option<unsafe extern "C" fn(instance: *mut std::ffi::c_void)>,
    pub extension_data:
        Option<unsafe extern "C" fn(uri: *const std::os::raw::c_char) -> *const std::ffi::c_void>,
}

pub fn default_lv2_folders() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/LV2"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/LV2"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        dirs.push(PathBuf::from("/usr/lib/lv2"));
        dirs.push(PathBuf::from("/usr/local/lib/lv2"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(home).join(".lv2"));
        }
    }
    dirs
}

pub fn scan_lv2_folders(extra_folders: &[String]) -> Vec<ServerPlugin> {
    let mut folders = default_lv2_folders();
    for f in extra_folders {
        folders.push(PathBuf::from(f));
    }

    let mut plugins = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("lv2") {
                if let Some(p) = inspect_lv2_bundle(&path) {
                    plugins.push(p);
                }
            }
        }
    }
    plugins
}

pub fn inspect_lv2_bundle(bundle_path: &Path) -> Option<ServerPlugin> {
    let manifest_path = bundle_path.join("manifest.ttl");
    let stem = bundle_path.file_stem()?.to_string_lossy().into_owned();
    let mut uri = format!("http://opengridder.org/lv2/{}", stem);
    if let Ok(ttl) = std::fs::read_to_string(&manifest_path) {
        for line in ttl.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('<') && trimmed.contains("://") {
                if let Some(end) = trimmed.find('>') {
                    uri = trimmed[1..end].to_string();
                    break;
                }
            }
        }
    }

    Some(ServerPlugin {
        module_path: bundle_path.to_string_lossy().into_owned(),
        name: stem.clone(),
        company: "LV2".to_string(),
        id: uri.clone(),
        id_deprecated: stem,
        plugin_type: "LV2".to_string(),
        category: "Effect".to_string(),
        version: "1.0".to_string(),
        is_instrument: false,
        unique_id: 0,
        num_input_ch: 2,
        num_output_ch: 2,
        presets: vec!["Default".to_string()],
        parameters: default_plugin_parameters(),
        layouts: vec![PluginLayout {
            inputs: vec!["Stereo".to_string()],
            outputs: vec!["Stereo".to_string()],
        }],
        has_editor: true,
        scoped_id: format!("LV2:{}", uri),
    })
}

pub struct Lv2HostedPlugin {
    desc: ServerPlugin,
    lib_handle: *mut std::ffi::c_void,
    descriptor: *const LV2Descriptor,
    instance: *mut std::ffi::c_void,
    bundle_path_cstr: std::ffi::CString,
    control_ports: [f32; 7],
    latency_port: f32,
    sample_rate: f64,
    suspended: bool,
}

unsafe impl Send for Lv2HostedPlugin {}

impl Lv2HostedPlugin {
    fn ensure_instantiated(&mut self, sample_rate: f64) {
        if !self.instance.is_null() && (self.sample_rate - sample_rate).abs() < 1.0 {
            return;
        }
        unsafe {
            if !self.instance.is_null() {
                if let Some(deactivate) = (*self.descriptor).deactivate {
                    deactivate(self.instance);
                }
                if let Some(cleanup) = (*self.descriptor).cleanup {
                    cleanup(self.instance);
                }
                self.instance = std::ptr::null_mut();
            }
            self.sample_rate = sample_rate;
            if let Some(inst_fn) = (*self.descriptor).instantiate {
                let null_feature: *const std::ffi::c_void = std::ptr::null();
                self.instance = inst_fn(
                    self.descriptor,
                    sample_rate,
                    self.bundle_path_cstr.as_ptr(),
                    &null_feature,
                );
                if !self.instance.is_null() {
                    if let Some(connect) = (*self.descriptor).connect_port {
                        for i in 0..7 {
                            connect(
                                self.instance,
                                (4 + i) as u32,
                                &mut self.control_ports[i] as *mut f32 as *mut std::ffi::c_void,
                            );
                        }
                        connect(
                            self.instance,
                            11,
                            &mut self.latency_port as *mut f32 as *mut std::ffi::c_void,
                        );
                    }
                    if let Some(activate) = (*self.descriptor).activate {
                        activate(self.instance);
                    }
                }
            }
        }
    }
}

impl Drop for Lv2HostedPlugin {
    fn drop(&mut self) {
        unsafe {
            if !self.instance.is_null() && !self.descriptor.is_null() {
                if let Some(deactivate) = (*self.descriptor).deactivate {
                    deactivate(self.instance);
                }
                if let Some(cleanup) = (*self.descriptor).cleanup {
                    cleanup(self.instance);
                }
            }
            if !self.lib_handle.is_null() {
                libc::dlclose(self.lib_handle);
            }
        }
    }
}

impl HostedPlugin for Lv2HostedPlugin {
    fn descriptor(&self) -> &ServerPlugin {
        &self.desc
    }
    fn name(&self) -> &str {
        &self.desc.name
    }
    fn prepare_to_play(&mut self, sample_rate: f64, _max_block_size: usize) {
        self.ensure_instantiated(sample_rate);
    }
    fn release_resources(&mut self) {}
    fn is_suspended(&self) -> bool {
        self.suspended
    }
    fn suspend_processing(&mut self, should_be_suspended: bool) {
        self.suspended = should_be_suspended;
    }
    fn supports_double_precision(&self) -> bool {
        false
    }
    fn latency_samples(&self) -> i32 {
        self.latency_port as i32
    }
    fn set_latency_samples(&mut self, latency: i32) {
        self.latency_port = latency as f32;
    }
    fn tail_seconds(&self) -> f64 {
        0.0
    }
    fn num_input_channels(&self) -> usize {
        2
    }
    fn num_output_channels(&self) -> usize {
        2
    }
    fn enable_all_buses(&mut self) {}
    fn check_buses_layout_supported(&self, inputs: usize, outputs: usize) -> bool {
        inputs <= 2 && outputs <= 2
    }
    fn process_block_f32(
        &mut self,
        audio: &mut [Vec<f32>],
        _midi: &mut Vec<og_common::message::MidiEvent>,
        _pos: &og_common::message::PositionInfo,
    ) {
        if self.suspended || self.instance.is_null() || self.descriptor.is_null() || audio.is_empty() {
            return;
        }
        let frames = audio[0].len();
        if frames == 0 {
            return;
        }
        let in_l = audio[0].clone();
        let in_r = if audio.len() > 1 {
            audio[1].clone()
        } else {
            in_l.clone()
        };
        unsafe {
            if let Some(connect) = (*self.descriptor).connect_port {
                connect(self.instance, 0, in_l.as_ptr() as *mut std::ffi::c_void);
                connect(self.instance, 1, in_r.as_ptr() as *mut std::ffi::c_void);
                connect(self.instance, 2, audio[0].as_mut_ptr() as *mut std::ffi::c_void);
                if audio.len() > 1 {
                    connect(self.instance, 3, audio[1].as_mut_ptr() as *mut std::ffi::c_void);
                } else {
                    connect(self.instance, 3, audio[0].as_mut_ptr() as *mut std::ffi::c_void);
                }
            }
            if let Some(run) = (*self.descriptor).run {
                run(self.instance, frames as u32);
            }
        }
    }
    fn process_block_f64(
        &mut self,
        audio: &mut [Vec<f64>],
        midi: &mut Vec<og_common::message::MidiEvent>,
        pos: &og_common::message::PositionInfo,
    ) {
        let mut f32_buf: Vec<Vec<f32>> = audio
            .iter()
            .map(|ch| ch.iter().map(|&s| s as f32).collect())
            .collect();
        self.process_block_f32(&mut f32_buf, midi, pos);
        for (dst, src) in audio.iter_mut().zip(f32_buf.iter()) {
            for (d, &s) in dst.iter_mut().zip(src.iter()) {
                *d = s as f64;
            }
        }
    }
    fn parameters(&self) -> &[og_common::server_plugin::PluginParameter] {
        &self.desc.parameters
    }
    fn get_parameter(&self, param_idx: usize) -> f32 {
        self.control_ports.get(param_idx).copied().unwrap_or(0.0)
    }
    fn set_parameter(&mut self, param_idx: usize, value: f32) {
        if let Some(slot) = self.control_ports.get_mut(param_idx) {
            *slot = value.clamp(0.0, 1.0);
        }
        if let Some(p) = self.desc.parameters.get_mut(param_idx) {
            p.default_value = value.clamp(0.0, 1.0);
        }
    }
    fn get_parameter_text(&self, param_idx: usize) -> String {
        format!("{:.2}", self.get_parameter(param_idx))
    }
    fn num_programs(&self) -> usize {
        1
    }
    fn get_program_name(&self, _index: usize) -> String {
        "Default".to_string()
    }
    fn current_program(&self) -> usize {
        0
    }
    fn set_current_program(&mut self, _index: usize) {}
    fn get_state_information(&self) -> Vec<u8> {
        serde_json::to_vec(&self.control_ports.to_vec()).unwrap_or_default()
    }
    fn set_state_information(&mut self, data: &[u8]) -> Result<(), String> {
        if let Ok(vals) = serde_json::from_slice::<Vec<f32>>(data) {
            for (i, v) in vals.into_iter().enumerate().take(self.control_ports.len()) {
                self.control_ports[i] = v;
            }
        }
        Ok(())
    }
    fn has_editor(&self) -> bool {
        true
    }
    fn editor_size(&self) -> (u32, u32) {
        (720, 420)
    }
    fn render_editor_rgba(&self) -> (u32, u32, Vec<u8>) {
        (720, 420, vec![30; 720 * 420 * 4])
    }
}

pub fn instantiate_lv2(desc: &ServerPlugin) -> Result<Box<dyn HostedPlugin>, String> {
    let bundle_path = Path::new(&desc.module_path);
    let mut candidate_dylibs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(bundle_path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                if ext == "dylib" || ext == "so" {
                    candidate_dylibs.push(p);
                }
            }
        }
    }
    let dylib_path = candidate_dylibs
        .first()
        .ok_or_else(|| format!("No LV2 binary found in {:?}", bundle_path))?;
    let c_path = std::ffi::CString::new(dylib_path.to_string_lossy().as_bytes())
        .map_err(|e| e.to_string())?;
    let bundle_cstr = std::ffi::CString::new(bundle_path.to_string_lossy().as_bytes())
        .map_err(|e| e.to_string())?;

    unsafe {
        let handle = libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            return Err(format!("Failed to dlopen LV2 binary {:?}", dylib_path));
        }
        let sym_name = std::ffi::CString::new("lv2_descriptor").unwrap();
        let sym = libc::dlsym(handle, sym_name.as_ptr());
        if sym.is_null() {
            libc::dlclose(handle);
            return Err("Missing lv2_descriptor symbol".to_string());
        }
        let desc_fn: unsafe extern "C" fn(u32) -> *const LV2Descriptor =
            std::mem::transmute(sym);
        let lv2_desc = desc_fn(0);
        if lv2_desc.is_null() {
            libc::dlclose(handle);
            return Err("lv2_descriptor(0) returned null".to_string());
        }
        let mut hosted = Lv2HostedPlugin {
            desc: desc.clone(),
            lib_handle: handle,
            descriptor: lv2_desc,
            instance: std::ptr::null_mut(),
            bundle_path_cstr: bundle_cstr,
            control_ports: [0.0, 0.0, 0.0, 0.5, 1.0, 0.0, 0.5],
            latency_port: 1024.0,
            sample_rate: 48000.0,
            suspended: false,
        };
        hosted.ensure_instantiated(48000.0);
        Ok(Box::new(hosted))
    }
}
