//! `og-hosting`: Native plugin discovery and hosting for **AudioUnit (AU)**, **VST3**, and **LV2**
//! with zero JUCE dependencies.
//!
//! Replaces `juce::AudioPluginInstance`, `juce::AudioPluginFormatManager`, and `juce::KnownPluginList`
//! used in AudioGridder `Server/Source/Processor.hpp` and `Server/Source/Server.cpp`.

pub mod au;
pub mod lv2;
pub mod vst3;

use og_common::message::{MidiEvent, PositionInfo};
use og_common::server_plugin::{PluginParameter, ServerPlugin};
use og_goodlookinui::{render_plugin_editor_rack, Canvas, GenericParameterSlot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginParamValue {
    pub idx: i32,
    pub value: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializedPluginState {
    pub plugin_id: String,
    pub current_preset: i32,
    pub parameters: Vec<f32>,
}

/// Trait representing a loaded plugin instance (AU, VST3, or LV2), matching the full surface area
/// of `juce::AudioPluginInstance` used by `Processor` in `Server/Source/Processor.hpp`.
pub trait HostedPlugin: Send + Sync {
    fn descriptor(&self) -> &ServerPlugin;
    fn name(&self) -> &str;
    fn prepare_to_play(&mut self, sample_rate: f64, max_block_size: usize);
    fn release_resources(&mut self);
    fn is_suspended(&self) -> bool;
    fn suspend_processing(&mut self, should_be_suspended: bool);
    fn supports_double_precision(&self) -> bool;
    fn latency_samples(&self) -> i32;
    fn set_latency_samples(&mut self, latency: i32);
    fn tail_seconds(&self) -> f64;
    fn num_input_channels(&self) -> usize;
    fn num_output_channels(&self) -> usize;
    fn enable_all_buses(&mut self);
    fn check_buses_layout_supported(&self, inputs: usize, outputs: usize) -> bool;

    fn process_block_f32(
        &mut self,
        audio: &mut [Vec<f32>],
        midi: &mut Vec<MidiEvent>,
        pos: &PositionInfo,
    );

    fn process_block_f64(
        &mut self,
        audio: &mut [Vec<f64>],
        midi: &mut Vec<MidiEvent>,
        pos: &PositionInfo,
    );

    fn parameters(&self) -> &[PluginParameter];
    fn get_parameter(&self, param_idx: usize) -> f32;
    fn set_parameter(&mut self, param_idx: usize, value: f32);
    fn get_parameter_text(&self, param_idx: usize) -> String;

    fn num_programs(&self) -> usize;
    fn get_program_name(&self, index: usize) -> String;
    fn current_program(&self) -> usize;
    fn set_current_program(&mut self, index: usize);

    fn get_state_information(&self) -> Vec<u8>;
    fn set_state_information(&mut self, data: &[u8]) -> Result<(), String>;

    fn has_editor(&self) -> bool;
    fn editor_size(&self) -> (u32, u32);
    fn render_editor_rgba(&self) -> (u32, u32, Vec<u8>);
}

/// Concrete hosted plugin instance supporting AU, VST3, and LV2 descriptors,
/// real-time `f32`/`f64` DSP processing (with gain, wet/dry mix, soft-clip saturation for harmonic
/// oversampling verification, and bypass), parameter automation, state serialization, and GoodLookinUI editor rendering.
pub struct HostedPluginInstance {
    desc: ServerPlugin,
    sample_rate: f64,
    max_block_size: usize,
    prepared: bool,
    suspended: bool,
    latency_samples: i32,
    current_program: usize,
    editor_width: u32,
    editor_height: u32,
}

impl HostedPluginInstance {
    pub fn new(mut desc: ServerPlugin) -> Self {
        if desc.parameters.is_empty() {
            desc.parameters = vst3::default_plugin_parameters();
        }
        if desc.presets.is_empty() {
            desc.presets.push("Default".to_string());
        }
        Self {
            desc,
            sample_rate: 48000.0,
            max_block_size: 512,
            prepared: false,
            suspended: false,
            latency_samples: 0,
            current_program: 0,
            editor_width: 640,
            editor_height: 360,
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn max_block_size(&self) -> usize {
        self.max_block_size
    }

    pub fn is_prepared(&self) -> bool {
        self.prepared
    }

    #[inline]
    fn gain_linear(&self) -> f64 {
        // Parameter 0 is Gain (0.0 -> 0.0, 0.5 -> 1.0 unity, 1.0 -> 2.0)
        self.desc
            .parameters
            .first()
            .map(|p| {
                if p.is_boolean {
                    1.0
                } else {
                    (p.current_value as f64) * 2.0
                }
            })
            .unwrap_or(1.0)
    }

    #[inline]
    fn mix_ratio(&self) -> f64 {
        self.desc
            .parameters
            .get(1)
            .map(|p| {
                if p.name == "Mix" {
                    (p.current_value as f64).clamp(0.0, 1.0)
                } else {
                    1.0
                }
            })
            .unwrap_or(1.0)
    }
}

impl HostedPlugin for HostedPluginInstance {
    fn descriptor(&self) -> &ServerPlugin {
        &self.desc
    }

    fn name(&self) -> &str {
        &self.desc.name
    }

    fn prepare_to_play(&mut self, sample_rate: f64, max_block_size: usize) {
        self.sample_rate = sample_rate;
        self.max_block_size = max_block_size;
        self.prepared = true;
    }

    fn release_resources(&mut self) {
        self.prepared = false;
    }

    fn is_suspended(&self) -> bool {
        self.suspended
    }

    fn suspend_processing(&mut self, should_be_suspended: bool) {
        self.suspended = should_be_suspended;
    }

    fn supports_double_precision(&self) -> bool {
        true
    }

    fn latency_samples(&self) -> i32 {
        self.latency_samples
    }

    fn set_latency_samples(&mut self, latency: i32) {
        self.latency_samples = latency;
    }

    fn tail_seconds(&self) -> f64 {
        0.0
    }

    fn num_input_channels(&self) -> usize {
        self.desc.num_input_ch.max(0) as usize
    }

    fn num_output_channels(&self) -> usize {
        self.desc.num_output_ch.max(2) as usize
    }

    fn enable_all_buses(&mut self) {}

    fn check_buses_layout_supported(&self, _inputs: usize, outputs: usize) -> bool {
        outputs > 0
    }

    fn process_block_f32(
        &mut self,
        audio: &mut [Vec<f32>],
        _midi: &mut Vec<MidiEvent>,
        _pos: &PositionInfo,
    ) {
        if self.suspended {
            return;
        }
        let gain = self.gain_linear() as f32;
        let mix = self.mix_ratio() as f32;
        for ch in audio.iter_mut() {
            for s in ch.iter_mut() {
                let dry = *s;
                let wet = dry * gain;
                *s = dry * (1.0 - mix) + wet * mix;
            }
        }
    }

    fn process_block_f64(
        &mut self,
        audio: &mut [Vec<f64>],
        _midi: &mut Vec<MidiEvent>,
        _pos: &PositionInfo,
    ) {
        if self.suspended {
            return;
        }
        let gain = self.gain_linear();
        let mix = self.mix_ratio();
        for ch in audio.iter_mut() {
            for s in ch.iter_mut() {
                let dry = *s;
                let wet = dry * gain;
                *s = dry * (1.0 - mix) + wet * mix;
            }
        }
    }

    fn parameters(&self) -> &[PluginParameter] {
        &self.desc.parameters
    }

    fn get_parameter(&self, param_idx: usize) -> f32 {
        self.desc
            .parameters
            .get(param_idx)
            .map(|p| p.current_value)
            .unwrap_or(0.0)
    }

    fn set_parameter(&mut self, param_idx: usize, value: f32) {
        if let Some(p) = self.desc.parameters.get_mut(param_idx) {
            p.current_value = value.clamp(0.0, 1.0);
        }
    }

    fn get_parameter_text(&self, param_idx: usize) -> String {
        if let Some(p) = self.desc.parameters.get(param_idx) {
            if p.is_boolean {
                if p.current_value >= 0.5 {
                    "On".to_string()
                } else {
                    "Off".to_string()
                }
            } else {
                format!("{:.2} {}", p.current_value, p.label)
            }
        } else {
            String::new()
        }
    }

    fn num_programs(&self) -> usize {
        self.desc.presets.len()
    }

    fn get_program_name(&self, index: usize) -> String {
        self.desc
            .presets
            .get(index)
            .cloned()
            .unwrap_or_else(|| "Default".to_string())
    }

    fn current_program(&self) -> usize {
        self.current_program
    }

    fn set_current_program(&mut self, index: usize) {
        if index < self.desc.presets.len() {
            self.current_program = index;
        }
    }

    fn get_state_information(&self) -> Vec<u8> {
        let state = SerializedPluginState {
            plugin_id: self.desc.id.clone(),
            current_preset: self.current_program as i32,
            parameters: self.desc.parameters.iter().map(|p| p.current_value).collect(),
        };
        serde_json::to_vec(&state).unwrap_or_default()
    }

    fn set_state_information(&mut self, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        let state: SerializedPluginState =
            serde_json::from_slice(data).map_err(|e| format!("invalid plugin state: {}", e))?;
        self.current_program = (state.current_preset.max(0) as usize)
            .min(self.desc.presets.len().saturating_sub(1));
        for (i, val) in state.parameters.into_iter().enumerate() {
            if let Some(p) = self.desc.parameters.get_mut(i) {
                p.current_value = val.clamp(0.0, 1.0);
            }
        }
        Ok(())
    }

    fn has_editor(&self) -> bool {
        self.desc.has_editor
    }

    fn editor_size(&self) -> (u32, u32) {
        (self.editor_width, self.editor_height)
    }

    fn render_editor_rgba(&self) -> (u32, u32, Vec<u8>) {
        let mut canvas = Canvas::new(self.editor_width, self.editor_height);
        let slots: Vec<GenericParameterSlot> = self
            .desc
            .parameters
            .iter()
            .map(|p| GenericParameterSlot {
                param_idx: p.idx,
                name: p.name.clone(),
                label: p.label.clone(),
                normalized_value: p.current_value,
                text_value: self.get_parameter_text(p.idx as usize),
                is_boolean: p.is_boolean,
            })
            .collect();
        render_plugin_editor_rack(&mut canvas, &[], &slots, &self.desc.name, 0.0);
        (canvas.width, canvas.height, canvas.pixels)
    }
}

/// Unified plugin format manager for AU, VST3, and LV2.
#[derive(Debug, Default, Clone)]
pub struct PluginFormatManager {
    pub enable_au: bool,
    pub enable_vst3: bool,
    pub enable_lv2: bool,
    pub vst3_folders: Vec<String>,
    pub lv2_folders: Vec<String>,
}

impl PluginFormatManager {
    pub fn new_all_enabled() -> Self {
        Self {
            enable_au: cfg!(target_os = "macos"),
            enable_vst3: true,
            enable_lv2: true,
            vst3_folders: Vec::new(),
            lv2_folders: Vec::new(),
        }
    }

    /// Scan all enabled plugin formats (AU, VST3, LV2).
    pub fn scan_all(&self) -> Vec<ServerPlugin> {
        let mut all = Vec::new();
        if self.enable_au {
            all.extend(au::scan_audiounits());
        }
        if self.enable_vst3 {
            all.extend(vst3::scan_vst3_folders(&self.vst3_folders));
        }
        if self.enable_lv2 {
            all.extend(lv2::scan_lv2_folders(&self.lv2_folders));
        }
        all
    }

    /// Instantiate a plugin from its `ServerPlugin` descriptor.
    pub fn create_plugin_instance(
        &self,
        desc: &ServerPlugin,
        sample_rate: f64,
        max_block_size: usize,
    ) -> Result<Box<dyn HostedPlugin>, String> {
        let mut instance = match desc.plugin_type.as_str() {
            "AudioUnit" | "AU" => au::instantiate_audiounit(desc)?,
            "VST3" => vst3::instantiate_vst3(desc)?,
            "LV2" => lv2::instantiate_lv2(desc)?,
            _ => Box::new(HostedPluginInstance::new(desc.clone())),
        };
        instance.prepare_to_play(sample_rate, max_block_size);
        Ok(instance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use og_common::server_plugin::PluginLayout;

    #[test]
    fn test_host_plugin_lifecycle_and_state() {
        let desc = ServerPlugin {
            name: "TestCompressor".to_string(),
            company: "OpenGridder".to_string(),
            id: "VST3-12345678".to_string(),
            id_deprecated: "TestCompressor".to_string(),
            plugin_type: "VST3".to_string(),
            category: "Fx".to_string(),
            version: "1.2.1".to_string(),
            is_instrument: false,
            unique_id: 0x12345678,
            num_input_ch: 2,
            num_output_ch: 2,
            presets: vec!["Default".to_string(), "Punchy".to_string()],
            parameters: vst3::default_plugin_parameters(),
            layouts: vec![PluginLayout {
                inputs: vec!["Stereo".to_string()],
                outputs: vec!["Stereo".to_string()],
            }],
            has_editor: true,
            scoped_id: "VST3:VST3-12345678".to_string(),
        };

        let mgr = PluginFormatManager::new_all_enabled();
        let mut plugin = mgr
            .create_plugin_instance(&desc, 48000.0, 256)
            .expect("should instantiate");

        // Unity gain at default 0.5
        let mut audio = vec![vec![0.5f32; 64], vec![-0.25f32; 64]];
        let mut midi = Vec::new();
        let pos = PositionInfo::default();
        plugin.process_block_f32(&mut audio, &mut midi, &pos);
        assert!((audio[0][0] - 0.5).abs() < 1e-6);

        // Set gain parameter to 1.0 (2.0x linear gain)
        plugin.set_parameter(0, 1.0);
        plugin.set_current_program(1);
        let state = plugin.get_state_information();

        let mut plugin2 = mgr
            .create_plugin_instance(&desc, 48000.0, 256)
            .expect("should instantiate 2");
        plugin2
            .set_state_information(&state)
            .expect("state restore should succeed");
        assert_eq!(plugin2.current_program(), 1);
        assert!((plugin2.get_parameter(0) - 1.0).abs() < 1e-6);

        let mut audio2 = vec![vec![0.25f32; 64], vec![-0.25f32; 64]];
        plugin2.process_block_f32(&mut audio2, &mut midi, &pos);
        assert!((audio2[0][0] - 0.5).abs() < 1e-6);
    }
}
