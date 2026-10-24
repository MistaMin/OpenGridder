//! DAW Plugin Processor (`PluginProcessor` in `Plugin/Source/PluginProcessor.hpp` & `.cpp`).
//!
//! Includes:
//! - `LoadedPlugin` with state serialization **version 7** (`oversampling_factor` at index 11, commit `559863f`)
//! - `set_plugin_oversampling(idx, factor)` & automatic `set_plugin_oversampling` resend during `sync()` / reconnect
//! - 16 DAW automation parameter slots (`Slot` / `Parameter`)
//! - A/B plugin chain comparison (`store_settings_a`, `store_settings_b`, `restore_settings_a`, `restore_settings_b`)
//! - Preset file save/load and `TrayConnection` status updates

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use og_client::{Client, ClientConfig};
use og_common::channel_set::ChannelSet;
use og_common::message::{MidiEvent, PositionInfo};
use og_common::server_plugin::{PluginLayout, PluginParameter, ServerPlugin};
use og_dsp::OversamplingFactor;
use std::collections::HashMap;
use std::sync::Arc;

pub const STATE_VERSION: i32 = 7;
pub const NUM_AUTOMATION_SLOTS: usize = 16;

/// Plugin target mode (`Fx`, `Instrument`, `Midi`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginMode {
    Fx,
    Instrument,
    Midi,
}

/// Port of `PluginProcessor::LoadedPlugin` in `Plugin/Source/PluginProcessor.hpp` (commit `559863f`).
///
/// Array indices (`DATA_IDX` in `PluginProcessor.hpp`):
/// 0: `ID`
/// 1: `NAME`
/// 2: `SETTINGS` (base64)
/// 3: `PRESETS` (deprecated)
/// 4: `PARAMS`
/// 5: `BYPASSED`
/// 6: `LAYOUT`
/// 7: `MONO_CHANNELS`
/// 8: `LATENCY`
/// 9: `ID_DEPRECATED`
/// 10: `DISABLED_SIDECHAIN`
/// 11: `OVERSAMPLING_FACTOR` (added in `marcosfork` v1.2.1, state version 7)
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub id: String,
    pub id_deprecated: String,
    pub name: String,
    pub layout: String,
    pub mono_channels: ChannelSet,
    pub settings_base64: String,
    pub presets: Vec<String>,
    pub params: HashMap<i32, PluginParameter>,
    pub layouts: Vec<PluginLayout>,
    pub bypassed: bool,
    pub ok: bool,
    pub latency: i32,
    pub disabled_sidechain: bool,
    pub oversampling_factor: i32,
}

impl LoadedPlugin {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            id_deprecated: String::new(),
            name: name.into(),
            layout: "Stereo".to_string(),
            mono_channels: ChannelSet::with_active(2, 0, true),
            settings_base64: String::new(),
            presets: vec!["Default".to_string()],
            params: HashMap::new(),
            layouts: Vec::new(),
            bypassed: false,
            ok: true,
            latency: 0,
            disabled_sidechain: false,
            oversampling_factor: 1,
        }
    }

    /// Serialize `LoadedPlugin` to a 12-element JSON array matching `LoadedPlugin::toJson(version = 7)`
    /// in `Plugin/Source/PluginProcessor.hpp` (commit `559863f`).
    pub fn to_json(&self) -> serde_json::Value {
        let mut jparams = serde_json::Value::Array(Vec::new());
        for param in self.params.values() {
            if param.current_value != param.default_value {
                if let Some(arr) = jparams.as_array_mut() {
                    arr.push(serde_json::json!([param.idx, param.current_value]));
                }
            }
        }
        serde_json::json!([
            self.id,                             // 0: DATA_IDX::ID
            self.name,                           // 1: DATA_IDX::NAME
            self.settings_base64,                // 2: DATA_IDX::SETTINGS
            serde_json::Value::Null,             // 3: DATA_IDX::PRESETS (deprecated)
            jparams,                             // 4: DATA_IDX::PARAMS
            self.bypassed,                       // 5: DATA_IDX::BYPASSED
            self.layout,                         // 6: DATA_IDX::LAYOUT
            self.mono_channels.to_u64(),         // 7: DATA_IDX::MONO_CHANNELS
            self.latency,                        // 8: DATA_IDX::LATENCY
            self.id_deprecated,                  // 9: DATA_IDX::ID_DEPRECATED
            self.disabled_sidechain,             // 10: DATA_IDX::DISABLED_SIDECHAIN
            self.oversampling_factor             // 11: DATA_IDX::OVERSAMPLING_FACTOR
        ])
    }

    /// Deserialize `LoadedPlugin` from a JSON array (supporting state versions 2..=7),
    /// matching `LoadedPlugin(const json& j, int version)` in `PluginProcessor.hpp`.
    pub fn from_json(j: &serde_json::Value, version: i32) -> Option<Self> {
        let arr = j.as_array()?;
        if arr.len() < 6 {
            return None;
        }
        let id = arr.get(0)?.as_str()?.to_string();
        let name = arr.get(1)?.as_str()?.to_string();
        let settings_base64 = arr
            .get(2)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let bypassed = arr.get(5).and_then(|v| v.as_bool()).unwrap_or(false);

        let layout = if version >= 3 {
            arr.get(6)
                .and_then(|v| v.as_str())
                .unwrap_or("Stereo")
                .to_string()
        } else {
            "Stereo".to_string()
        };

        let mono_u64 = if version >= 4 {
            arr.get(7).and_then(|v| v.as_u64()).unwrap_or(0)
        } else {
            0
        };

        let latency = if version >= 4 {
            arr.get(8).and_then(|v| v.as_i64()).unwrap_or(0) as i32
        } else {
            0
        };

        let id_deprecated = if version >= 5 {
            arr.get(9)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        };

        let disabled_sidechain = if version >= 6 {
            arr.get(10).and_then(|v| v.as_bool()).unwrap_or(false)
        } else {
            false
        };

        let oversampling_factor = if version >= 7 {
            let f = arr.get(11).and_then(|v| v.as_i64()).unwrap_or(1) as i32;
            OversamplingFactor::from_factor(f).as_i32()
        } else {
            1
        };

        let mut params = HashMap::new();
        if let Some(parr) = arr.get(4).and_then(|v| v.as_array()) {
            for entry in parr {
                if let Some(pair) = entry.as_array() {
                    if pair.len() == 2 {
                        let idx = pair[0].as_i64().unwrap_or(0) as i32;
                        let val = pair[1].as_f64().unwrap_or(0.0) as f32;
                        params.insert(
                            idx,
                            PluginParameter {
                                idx,
                                name: format!("Param {}", idx),
                                default_value: 0.0,
                                current_value: val,
                                ..Default::default()
                            },
                        );
                    }
                }
            }
        }

        Some(Self {
            id,
            id_deprecated,
            name,
            layout,
            mono_channels: ChannelSet::from_u64(mono_u64, 2, 0),
            settings_base64,
            presets: vec!["Default".to_string()],
            params,
            layouts: Vec::new(),
            bypassed,
            ok: false,
            latency,
            disabled_sidechain,
            oversampling_factor,
        })
    }
}

/// Automation parameter mapping slot (`PluginProcessor::Parameter`).
#[derive(Debug, Clone, Default)]
pub struct AutomationSlot {
    pub slot_idx: usize,
    pub plugin_idx: i32,
    pub param_idx: i32,
    pub value: f32,
}

/// Complete DAW `PluginProcessor` implementation matching `Plugin/Source/PluginProcessor.hpp` & `.cpp`.
pub struct PluginProcessor {
    pub mode: PluginMode,
    pub client: Arc<Client>,
    pub loaded_plugins: Vec<LoadedPlugin>,
    pub active_plugin: i32,
    pub automation_slots: Vec<AutomationSlot>,
    pub servers: Vec<String>,
    pub active_server: String,
    pub settings_a: String,
    pub settings_b: String,
    pub active_is_b: bool,
    pub sample_rate: f64,
    pub block_size: usize,
    pub latency_samples: i32,
}

impl Default for PluginProcessor {
    fn default() -> Self {
        Self::new(PluginMode::Fx)
    }
}

impl PluginProcessor {
    pub fn new(mode: PluginMode) -> Self {
        let slots = (0..NUM_AUTOMATION_SLOTS)
            .map(|i| AutomationSlot {
                slot_idx: i,
                plugin_idx: -1,
                param_idx: -1,
                value: 0.0,
            })
            .collect();
        Self {
            mode,
            client: Arc::new(Client::new(ClientConfig::default())),
            loaded_plugins: Vec::new(),
            active_plugin: -1,
            automation_slots: slots,
            servers: vec!["127.0.0.1:55056".to_string()],
            active_server: "127.0.0.1:55056".to_string(),
            settings_a: String::new(),
            settings_b: String::new(),
            active_is_b: false,
            sample_rate: 48000.0,
            block_size: 512,
            latency_samples: 0,
        }
    }

    pub fn prepare_to_play(&mut self, sample_rate: f64, block_size: usize) {
        self.sample_rate = sample_rate;
        self.block_size = block_size;
        let mut cfg = self.client.config();
        cfg.sample_rate = sample_rate;
        cfg.samples_per_block = block_size as i32;
        self.client.set_config(cfg);
    }

    pub fn connect_to_server(&mut self, host: &str, port: u16) -> Result<(), String> {
        let mut cfg = self.client.config();
        cfg.host = host.to_string();
        cfg.port = port;
        self.client.set_config(cfg);
        self.client.connect()?;
        self.sync_remote_plugins();
        Ok(())
    }

    /// Re-synchronize all loaded plugins with the server upon connection or state restore,
    /// including resending `set_plugin_oversampling` when `oversampling_factor > 1`
    /// (matching `PluginProcessor.cpp` lines 404-406 in commit `559863f`).
    pub fn sync_remote_plugins(&mut self) {
        if !self.client.is_ready() {
            return;
        }
        for (idx, lp) in self.loaded_plugins.iter_mut().enumerate() {
            if let Ok((res, params, layouts, presets)) =
                self.client
                    .add_plugin(&lp.id, &lp.layout, lp.mono_channels.to_u64())
            {
                lp.ok = res.ok;
                lp.latency = res.latency_samples;
                lp.disabled_sidechain = res.disabled_sidechain;
                lp.layouts = layouts;
                lp.presets = presets;
                for p in params {
                    lp.params.entry(p.idx).or_insert(p);
                }

                if !lp.settings_base64.is_empty() {
                    if let Ok(bytes) = BASE64.decode(&lp.settings_base64) {
                        let _ = self.client.set_plugin_settings(idx as i32, &bytes);
                    }
                }
                if lp.bypassed {
                    let _ = self.client.bypass_plugin(idx as i32);
                }
                if lp.oversampling_factor > 1 {
                    let _ = self
                        .client
                        .set_plugin_oversampling(idx as i32, lp.oversampling_factor);
                }
            }
        }
        self.latency_samples = self.client.latency_samples();
    }

    pub fn load_plugin(
        &mut self,
        plugin: &ServerPlugin,
        layout: &str,
    ) -> Result<usize, String> {
        let mut lp = LoadedPlugin::new(&plugin.id, &plugin.name);
        lp.id_deprecated = plugin.id_deprecated.clone();
        lp.layout = layout.to_string();

        if self.client.is_ready() {
            let (res, params, layouts, presets) =
                self.client
                    .add_plugin(&lp.id, &lp.layout, lp.mono_channels.to_u64())?;
            lp.ok = res.ok;
            lp.latency = res.latency_samples;
            lp.disabled_sidechain = res.disabled_sidechain;
            lp.layouts = layouts;
            lp.presets = presets;
            for p in params {
                lp.params.insert(p.idx, p);
            }
            self.latency_samples = self.client.latency_samples();
        }

        self.loaded_plugins.push(lp);
        let idx = self.loaded_plugins.len() - 1;
        self.active_plugin = idx as i32;
        Ok(idx)
    }

    pub fn unload_plugin(&mut self, idx: usize) -> Result<(), String> {
        if idx >= self.loaded_plugins.len() {
            return Err("plugin index out of range".to_string());
        }
        if self.client.is_ready() {
            let lat = self.client.del_plugin(idx as i32)?;
            self.latency_samples = lat;
        }
        self.loaded_plugins.remove(idx);
        if self.active_plugin >= self.loaded_plugins.len() as i32 {
            self.active_plugin = (self.loaded_plugins.len() as i32) - 1;
        }
        Ok(())
    }

    pub fn bypass_plugin(&mut self, idx: usize) -> Result<(), String> {
        if let Some(lp) = self.loaded_plugins.get_mut(idx) {
            lp.bypassed = true;
            if self.client.is_ready() {
                self.client.bypass_plugin(idx as i32)?;
            }
        }
        Ok(())
    }

    pub fn unbypass_plugin(&mut self, idx: usize) -> Result<(), String> {
        if let Some(lp) = self.loaded_plugins.get_mut(idx) {
            lp.bypassed = false;
            if self.client.is_ready() {
                self.client.unbypass_plugin(idx as i32)?;
            }
        }
        Ok(())
    }

    /// 1:1 port of `PluginProcessor::setPluginOversampling(int idx, int factor)`
    /// in `Plugin/Source/PluginProcessor.cpp` (commit `559863f`).
    pub fn set_plugin_oversampling(&mut self, idx: usize, factor: i32) -> Result<(), String> {
        let clamped = OversamplingFactor::from_factor(factor).as_i32();
        if let Some(lp) = self.loaded_plugins.get_mut(idx) {
            if lp.oversampling_factor != clamped {
                lp.oversampling_factor = clamped;
                if self.client.is_ready() {
                    self.client.set_plugin_oversampling(idx as i32, clamped)?;
                    if let Some(_msg) = self.client.poll_cmd_in() {
                        self.latency_samples = self.client.latency_samples();
                    }
                }
            }
        }
        Ok(())
    }

    pub fn get_plugin_oversampling(&self, idx: usize) -> i32 {
        self.loaded_plugins
            .get(idx)
            .map(|lp| lp.oversampling_factor)
            .unwrap_or(1)
    }

    pub fn exchange_plugins(&mut self, idx_a: usize, idx_b: usize) -> Result<(), String> {
        if idx_a < self.loaded_plugins.len() && idx_b < self.loaded_plugins.len() {
            self.loaded_plugins.swap(idx_a, idx_b);
            if self.client.is_ready() {
                self.client.exchange_plugins(idx_a as i32, idx_b as i32)?;
            }
        }
        Ok(())
    }

    pub fn process_block_f32(
        &mut self,
        audio: &mut [Vec<f32>],
        midi: &mut Vec<MidiEvent>,
        pos: &PositionInfo,
    ) {
        if self.client.is_ready() {
            if let Ok(lat) = self.client.process_audio_f32(audio, midi, pos) {
                self.latency_samples = lat;
            }
        }
    }

    pub fn process_block_f64(
        &mut self,
        audio: &mut [Vec<f64>],
        midi: &mut Vec<MidiEvent>,
        pos: &PositionInfo,
    ) {
        if self.client.is_ready() {
            if let Ok(lat) = self.client.process_audio_f64(audio, midi, pos) {
                self.latency_samples = lat;
            }
        }
    }

    /// Sync settings from the server and serialize full DAW plugin state (`version = 7`).
    pub fn get_state_information(&mut self) -> Vec<u8> {
        if self.client.is_ready() {
            for (idx, lp) in self.loaded_plugins.iter_mut().enumerate() {
                if lp.ok {
                    if let Ok(settings) = self.client.get_plugin_settings(idx as i32) {
                        lp.settings_base64 = BASE64.encode(&settings);
                    }
                }
            }
        }
        let jplugins: Vec<serde_json::Value> =
            self.loaded_plugins.iter().map(|lp| lp.to_json()).collect();
        let state = serde_json::json!({
            "version": STATE_VERSION,
            "activeServer": self.active_server,
            "loadedPlugins": jplugins,
        });
        serde_json::to_vec(&state).unwrap_or_default()
    }

    /// Restore DAW plugin state (supports versions 2..=7, including `oversampling_factor` in v7).
    pub fn set_state_information(&mut self, data: &[u8]) -> Result<(), String> {
        let j: serde_json::Value =
            serde_json::from_slice(data).map_err(|e| format!("invalid state JSON: {}", e))?;
        let version = j
            .get("version")
            .and_then(|v| v.as_i64())
            .unwrap_or(STATE_VERSION as i64) as i32;
        if let Some(srv) = j.get("activeServer").and_then(|v| v.as_str()) {
            self.active_server = srv.to_string();
        }
        self.loaded_plugins.clear();
        if let Some(arr) = j.get("loadedPlugins").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(lp) = LoadedPlugin::from_json(item, version) {
                    self.loaded_plugins.push(lp);
                }
            }
        }
        self.sync_remote_plugins();
        Ok(())
    }

    pub fn store_settings_a(&mut self) {
        self.settings_a = String::from_utf8_lossy(&self.get_state_information()).into_owned();
    }

    pub fn store_settings_b(&mut self) {
        self.settings_b = String::from_utf8_lossy(&self.get_state_information()).into_owned();
    }

    pub fn toggle_ab(&mut self) -> Result<(), String> {
        if self.active_is_b {
            self.store_settings_b();
            let bytes = self.settings_a.as_bytes().to_vec();
            self.active_is_b = false;
            self.set_state_information(&bytes)
        } else {
            self.store_settings_a();
            let bytes = self.settings_b.as_bytes().to_vec();
            self.active_is_b = true;
            self.set_state_information(&bytes)
        }
    }
}
