//! Server-side plugin wrapper (`Processor` in `Server/Source/Processor.hpp` & `Processor.cpp`).
//!
//! Includes:
//! - Per-plugin `1x`/`2x`/`4x` oversampling (`m_oversamplingFactor`, `ProcessorOversamplers`)
//! - Live `set_oversampling_factor` that suspends, releases, resets oversamplers, and re-prepares
//!   each hosted plugin at `sample_rate * factor` and `block_size * factor` (commit `559863f`)
//! - Latency-compensated bypass ring buffers (`AudioRingBuffer<f32>` and `AudioRingBuffer<f64>`)
//! - Multi-mono per-channel plugin execution

use og_common::audio_ring_buffer::AudioRingBuffer;
use og_common::channel_set::ChannelSet;
use og_common::message::{MidiEvent, PositionInfo};
use og_common::server_plugin::{PluginLayout, PluginParameter, ServerPlugin};
use og_dsp::{OversamplingFactor, ProcessorOversamplers};
use og_hosting::{HostedPlugin, PluginFormatManager};

pub struct Processor {
    id: String,
    layout: String,
    mono_channels: ChannelSet,
    is_mono: bool,
    plugins: Vec<Box<dyn HostedPlugin>>,
    bypass_buffers_f32: Vec<AudioRingBuffer<f32>>,
    bypass_buffers_f64: Vec<AudioRingBuffer<f64>>,
    oversamplers: ProcessorOversamplers,
    prepared: bool,
    suspended: bool,
    sample_rate: f64,
    max_block_size: usize,
    extra_in_channels: usize,
    extra_out_channels: usize,
    last_known_latency: i32,
}

impl Processor {
    pub fn new(
        format_mgr: &PluginFormatManager,
        desc: &ServerPlugin,
        layout: &str,
        mono_channels: u64,
        sample_rate: f64,
        max_block_size: usize,
    ) -> Result<Self, String> {
        let is_mono = layout == "Multi-Mono";
        let num_ch = desc.num_output_ch.max(2) as usize;
        let num_slots = if is_mono { num_ch } else { 1 };
        let ch_per_slot = if is_mono { 1 } else { num_ch };

        let mut plugins = Vec::with_capacity(num_slots);
        for _ in 0..num_slots {
            let mut slot_desc = desc.clone();
            if is_mono {
                slot_desc.num_input_ch = 1;
                slot_desc.num_output_ch = 1;
            }
            let instance =
                format_mgr.create_plugin_instance(&slot_desc, sample_rate, max_block_size)?;
            plugins.push(instance);
        }

        let mut proc = Self {
            id: desc.id.clone(),
            layout: layout.to_string(),
            mono_channels: ChannelSet::from_u64(mono_channels, num_ch.min(16), 0),
            is_mono,
            plugins,
            bypass_buffers_f32: vec![AudioRingBuffer::new(); num_slots],
            bypass_buffers_f64: vec![AudioRingBuffer::new(); num_slots],
            oversamplers: ProcessorOversamplers::new(num_slots, ch_per_slot, max_block_size),
            prepared: false,
            suspended: false,
            sample_rate,
            max_block_size,
            extra_in_channels: 0,
            extra_out_channels: 0,
            last_known_latency: 0,
        };
        proc.try_prepare_to_play(sample_rate, max_block_size)?;
        Ok(proc)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn layout(&self) -> &str {
        &self.layout
    }

    pub fn name(&self) -> &str {
        self.plugins.first().map(|p| p.name()).unwrap_or(&self.id)
    }

    pub fn descriptor(&self) -> Option<&ServerPlugin> {
        self.plugins.first().map(|p| p.descriptor())
    }

    pub fn prepare_to_play(&mut self, sample_rate: f64, max_block_size: usize) {
        if let Err(error) = self.try_prepare_to_play(sample_rate, max_block_size) {
            log::error!("{}: {error}", self.id);
            self.prepared = false;
            self.suspend_processing(true);
        }
    }

    fn try_prepare_to_play(
        &mut self,
        sample_rate: f64,
        max_block_size: usize,
    ) -> Result<(), String> {
        self.sample_rate = sample_rate;
        self.max_block_size = max_block_size.max(64);
        let factor = self.oversamplers.factor.as_usize();
        let ch_per_slot = if self.is_mono {
            1
        } else {
            self.num_output_channels()
        };
        self.oversamplers =
            ProcessorOversamplers::new(self.plugins.len(), ch_per_slot, self.max_block_size);
        self.oversamplers.factor = OversamplingFactor::from_factor(factor as i32);

        for plugin in &mut self.plugins {
            plugin
                .try_prepare_to_play(sample_rate * (factor as f64), self.max_block_size * factor)?;
        }
        self.prepared = true;
        self.update_bypass_buffers();
        Ok(())
    }

    pub fn release_resources(&mut self) {
        for plugin in &mut self.plugins {
            plugin.release_resources();
        }
        self.oversamplers.reset();
        self.prepared = false;
    }

    /// 1:1 port of `Processor::setOversamplingFactor(int factor)` from `Server/Source/Processor.cpp` (commit `559863f`).
    pub fn set_oversampling_factor(&mut self, mut factor: i32) {
        if factor != 1 && factor != 2 && factor != 4 {
            factor = 1;
        }
        let new_factor = OversamplingFactor::from_factor(factor);
        if new_factor == self.oversamplers.factor {
            return;
        }
        self.oversamplers.factor = new_factor;

        if self.prepared && self.sample_rate > 0.0 && self.max_block_size > 0 {
            let f_usize = new_factor.as_usize();
            for plugin in &mut self.plugins {
                plugin.suspend_processing(true);
                plugin.release_resources();
            }
            self.oversamplers.reset();
            for plugin in &mut self.plugins {
                if let Err(error) = plugin.try_prepare_to_play(
                    self.sample_rate * (f_usize as f64),
                    self.max_block_size * f_usize,
                ) {
                    log::error!("{} oversampling setup failed: {error}", self.id);
                    self.prepared = false;
                    self.suspended = true;
                    break;
                }
                if !self.suspended {
                    plugin.suspend_processing(false);
                }
            }
            self.update_bypass_buffers();
        }
    }

    pub fn oversampling_factor(&self) -> i32 {
        self.oversamplers.factor.as_i32()
    }

    /// Total latency of this processor = plugin latency + oversampler FIR filter latency.
    /// Matches `Processor::getLatencySamples()` in `Server/Source/Processor.cpp`.
    pub fn latency_samples(&self) -> i32 {
        let base_lat = self
            .plugins
            .first()
            .map(|p| p.latency_samples())
            .unwrap_or(0);
        // Plugin latency is measured at its processing rate, which grows with oversampling.
        let factor = self.oversamplers.factor.as_usize() as i32;
        base_lat.max(0).saturating_add(factor - 1) / factor + self.oversamplers.latency_samples()
    }

    pub fn set_plugin_latency_samples(&mut self, lat: i32) {
        for p in &mut self.plugins {
            p.set_latency_samples(lat);
        }
        self.update_bypass_buffers();
    }

    pub fn update_bypass_buffers(&mut self) {
        let total_lat = self.latency_samples();
        if total_lat != self.last_known_latency {
            let ch = if self.is_mono {
                1
            } else {
                self.num_output_channels()
            };
            let cap = (total_lat.max(0) as usize) + self.max_block_size * 2;
            for b in &mut self.bypass_buffers_f32 {
                b.resize(ch, cap.max(1), true);
            }
            for b in &mut self.bypass_buffers_f64 {
                b.resize(ch, cap.max(1), true);
            }
            self.last_known_latency = total_lat;
        }
    }

    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    pub fn suspend_processing(&mut self, should_be_suspended: bool) {
        self.suspended = should_be_suspended;
        for plugin in &mut self.plugins {
            plugin.suspend_processing(should_be_suspended);
        }
    }

    pub fn num_input_channels(&self) -> usize {
        self.plugins
            .first()
            .map(|p| p.num_input_channels())
            .unwrap_or(2)
    }

    pub fn num_output_channels(&self) -> usize {
        self.plugins
            .first()
            .map(|p| p.num_output_channels())
            .unwrap_or(2)
    }

    pub fn set_extra_channels(&mut self, in_ch: usize, out_ch: usize) {
        self.extra_in_channels = in_ch;
        self.extra_out_channels = out_ch;
    }

    pub fn process_block_f32(
        &mut self,
        buffer: &mut [Vec<f32>],
        midi: &mut Vec<MidiEvent>,
        pos: &PositionInfo,
    ) {
        if !self.prepared {
            for channel in buffer {
                channel.fill(0.0);
            }
            return;
        }
        let num_samples = buffer.first().map(|c| c.len()).unwrap_or(0);
        self.update_bypass_buffers();
        if self.suspended {
            if self.latency_samples() > 0 {
                if let Some(ring) = self.bypass_buffers_f32.first_mut() {
                    ring.process(buffer, num_samples);
                }
            }
            return;
        }

        if self.is_mono {
            for (ch_idx, ch_vec) in buffer.iter_mut().enumerate() {
                if ch_idx >= self.plugins.len() {
                    break;
                }
                if self.mono_channels.is_input_active(ch_idx) {
                    let mut single_ch = vec![ch_vec.clone()];
                    let plugin = &mut self.plugins[ch_idx];
                    self.oversamplers.process_slot_f32(
                        ch_idx,
                        &mut single_ch,
                        num_samples,
                        |os_buf, _os_n| {
                            plugin.process_block_f32(os_buf, midi, pos);
                        },
                    );
                    ch_vec.copy_from_slice(&single_ch[0][..num_samples]);
                } else if self.latency_samples() > 0 {
                    let mut single_ch = vec![ch_vec.clone()];
                    self.bypass_buffers_f32[ch_idx].process(&mut single_ch, num_samples);
                    ch_vec.copy_from_slice(&single_ch[0][..num_samples]);
                }
            }
        } else if let Some(plugin) = self.plugins.first_mut() {
            self.oversamplers
                .process_slot_f32(0, buffer, num_samples, |os_buf, _os_n| {
                    plugin.process_block_f32(os_buf, midi, pos);
                });
        }
    }

    pub fn process_block_f64(
        &mut self,
        buffer: &mut [Vec<f64>],
        midi: &mut Vec<MidiEvent>,
        pos: &PositionInfo,
    ) {
        if !self.prepared {
            for channel in buffer {
                channel.fill(0.0);
            }
            return;
        }
        let num_samples = buffer.first().map(|c| c.len()).unwrap_or(0);
        self.update_bypass_buffers();
        if self.suspended {
            if self.latency_samples() > 0 {
                if let Some(ring) = self.bypass_buffers_f64.first_mut() {
                    ring.process(buffer, num_samples);
                }
            }
            return;
        }

        if self.is_mono {
            for (ch_idx, ch_vec) in buffer.iter_mut().enumerate() {
                if ch_idx >= self.plugins.len() {
                    break;
                }
                if self.mono_channels.is_input_active(ch_idx) {
                    let mut single_ch = vec![ch_vec.clone()];
                    let plugin = &mut self.plugins[ch_idx];
                    self.oversamplers.process_slot_f64(
                        ch_idx,
                        &mut single_ch,
                        num_samples,
                        |os_buf, _os_n| {
                            plugin.process_block_f64(os_buf, midi, pos);
                        },
                    );
                    ch_vec.copy_from_slice(&single_ch[0][..num_samples]);
                } else if self.latency_samples() > 0 {
                    let mut single_ch = vec![ch_vec.clone()];
                    self.bypass_buffers_f64[ch_idx].process(&mut single_ch, num_samples);
                    ch_vec.copy_from_slice(&single_ch[0][..num_samples]);
                }
            }
        } else if let Some(plugin) = self.plugins.first_mut() {
            self.oversamplers
                .process_slot_f64(0, buffer, num_samples, |os_buf, _os_n| {
                    plugin.process_block_f64(os_buf, midi, pos);
                });
        }
    }

    pub fn parameters(&self) -> Vec<PluginParameter> {
        self.plugins
            .first()
            .map(|p| p.parameters().to_vec())
            .unwrap_or_default()
    }

    pub fn layouts(&self) -> Vec<PluginLayout> {
        self.plugins
            .first()
            .map(|p| p.descriptor().layouts.clone())
            .unwrap_or_default()
    }

    pub fn presets(&self) -> Vec<String> {
        self.plugins
            .first()
            .map(|p| p.descriptor().presets.clone())
            .unwrap_or_default()
    }

    pub fn get_parameter(&self, param_idx: usize) -> f32 {
        self.plugins
            .first()
            .map(|p| p.get_parameter(param_idx))
            .unwrap_or(0.0)
    }

    pub fn set_parameter(&mut self, param_idx: usize, value: f32) {
        for p in &mut self.plugins {
            p.set_parameter(param_idx, value);
        }
    }

    pub fn set_current_program(&mut self, index: usize) {
        for p in &mut self.plugins {
            p.set_current_program(index);
        }
    }

    pub fn get_state_information(&self) -> Vec<u8> {
        self.plugins
            .first()
            .map(|p| p.get_state_information())
            .unwrap_or_default()
    }

    pub fn set_state_information(&mut self, data: &[u8]) -> Result<(), String> {
        for p in &mut self.plugins {
            p.set_state_information(data)?;
        }
        Ok(())
    }

    pub fn render_editor_rgba(&self) -> Option<(u32, u32, Vec<u8>)> {
        self.plugins.first().map(|p| p.render_editor_rgba())
    }
}
