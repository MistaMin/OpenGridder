//! Port of `Common/Source/ChannelSet.hpp`.

use crate::defaults::PLUGIN_CHANNELS_MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelSet {
    channels: u64,
    output_offset: usize,
    num_inputs: isize,
    num_outputs: isize,
}

impl Default for ChannelSet {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelSet {
    pub fn new() -> Self {
        Self {
            channels: 0,
            output_offset: 0,
            num_inputs: -1,
            num_outputs: -1,
        }
    }

    pub fn from_bits(bits: u64, with_input: bool) -> Self {
        Self {
            channels: bits,
            output_offset: if with_input { PLUGIN_CHANNELS_MAX / 2 } else { 0 },
            num_inputs: -1,
            num_outputs: -1,
        }
    }

    pub fn with_counts(channels: u64, num_inputs: usize, num_outputs: usize) -> Self {
        let clamped_in = num_inputs.min(PLUGIN_CHANNELS_MAX);
        let clamped_out = num_outputs.min(PLUGIN_CHANNELS_MAX);
        Self {
            channels,
            output_offset: if num_inputs > 0 { clamped_in } else { 0 },
            num_inputs: clamped_in as isize,
            num_outputs: clamped_out as isize,
        }
    }

    pub fn from_u64(channels: u64, num_inputs: usize, num_outputs: usize) -> Self {
        Self::with_counts(channels, num_inputs, num_outputs)
    }

    pub fn with_active(num_inputs: usize, num_outputs: usize, active: bool) -> Self {
        let mut cs = Self::with_counts(0, num_inputs, num_outputs);
        if active {
            cs.set_input_range_active(true);
            cs.set_output_range_active(true);
        }
        cs
    }

    pub fn to_u64(&self) -> u64 {
        self.channels
    }

    pub fn set_bits(&mut self, bits: u64) {
        self.channels = bits;
    }

    pub fn set_num_channels(&mut self, num_inputs: usize, num_outputs: usize) {
        self.num_inputs = num_inputs.min(PLUGIN_CHANNELS_MAX) as isize;
        self.num_outputs = num_outputs.min(PLUGIN_CHANNELS_MAX) as isize;
    }

    pub fn set_num_channels_with_offset(
        &mut self,
        num_inputs: usize,
        num_outputs: usize,
        output_offset: usize,
    ) {
        self.set_num_channels(num_inputs, num_outputs);
        self.output_offset = output_offset;
    }

    pub fn set_with_input(&mut self, with_input: bool) {
        self.output_offset = if with_input { PLUGIN_CHANNELS_MAX / 2 } else { 0 };
    }

    pub fn is_input(&self, ch: usize) -> bool {
        ch < self.output_offset
    }

    pub fn is_output(&self, ch: usize) -> bool {
        ch >= self.output_offset && ch < PLUGIN_CHANNELS_MAX
    }

    pub fn set_active(&mut self, ch: usize, input: bool, active: bool) {
        let idx = self.get_channel_index(ch, input);
        self.set_bit(idx, active);
    }

    pub fn set_input_active(&mut self, ch: usize, active: bool) {
        self.set_active(ch, true, active);
    }

    pub fn set_output_active(&mut self, ch: usize, active: bool) {
        self.set_active(ch, false, active);
    }

    pub fn is_active(&self, ch: usize, input: bool) -> bool {
        self.is_set(self.get_channel_index(ch, input))
    }

    pub fn is_input_active(&self, ch: usize) -> bool {
        self.is_active(ch, true)
    }

    pub fn is_output_active(&self, ch: usize) -> bool {
        self.is_active(ch, false)
    }

    pub fn set_range_active(&mut self, start: usize, end: usize, active: bool) {
        for ch in start..end.min(PLUGIN_CHANNELS_MAX) {
            self.set_bit(ch, active);
        }
    }

    pub fn is_range_active(&self, start: usize, end: usize) -> bool {
        for ch in start..end.min(PLUGIN_CHANNELS_MAX) {
            if !self.is_set(ch) {
                return false;
            }
        }
        true
    }

    pub fn set_input_range_active(&mut self, active: bool) {
        self.set_range_active(self.get_start(true), self.get_end(true), active);
    }

    pub fn set_output_range_active(&mut self, active: bool) {
        self.set_range_active(self.get_start(false), self.get_end(false), active);
    }

    pub fn is_input_range_active(&self) -> bool {
        self.is_range_active(self.get_start(true), self.get_end(true))
    }

    pub fn is_output_range_active(&self) -> bool {
        self.is_range_active(self.get_start(false), self.get_end(false))
    }

    pub fn get_num_channels(&self, input: bool) -> usize {
        let v = if input { self.num_inputs } else { self.num_outputs };
        v.max(0) as usize
    }

    pub fn get_num_channels_combined(&self) -> usize {
        self.get_num_channels(true).max(self.get_num_channels(false))
    }

    pub fn get_num_active_channels_range(&self, start: usize, end: usize) -> usize {
        let mut ret = 0;
        for ch in start..end.min(PLUGIN_CHANNELS_MAX) {
            if self.is_set(ch) {
                ret += 1;
            }
        }
        ret
    }

    pub fn get_num_active_channels(&self, input: bool) -> usize {
        self.get_num_active_channels_range(self.get_start(input), self.get_end(input))
    }

    pub fn get_num_active_channels_combined(&self) -> usize {
        self.get_num_active_channels(true)
            .max(self.get_num_active_channels(false))
    }

    pub fn get_active_channels(&self, input: bool) -> Vec<usize> {
        let mut ret = Vec::new();
        for ch in self.get_start(input)..self.get_end(input) {
            if self.is_set(ch) {
                ret.push(if input { ch } else { ch - self.output_offset });
            }
        }
        ret
    }

    fn get_channel_index(&self, ch: usize, input: bool) -> usize {
        ch + if input { 0 } else { self.output_offset }
    }

    fn set_bit(&mut self, ch: usize, active: bool) {
        if ch < PLUGIN_CHANNELS_MAX {
            if active {
                self.channels |= 1u64 << ch;
            } else {
                self.channels &= !(1u64 << ch);
            }
        }
    }

    fn get_start(&self, input: bool) -> usize {
        if input {
            0
        } else {
            self.output_offset
        }
    }

    fn get_end(&self, input: bool) -> usize {
        if input {
            if self.num_inputs > -1 {
                self.num_inputs as usize
            } else {
                self.output_offset
            }
        } else if self.num_outputs > -1 {
            (self.num_outputs as usize) + self.output_offset
        } else {
            PLUGIN_CHANNELS_MAX
        }
    }

    fn is_set(&self, ch: usize) -> bool {
        if ch < PLUGIN_CHANNELS_MAX {
            (self.channels & (1u64 << ch)) != 0
        } else {
            false
        }
    }
}

impl std::fmt::Display for ChannelSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inputs: Vec<String> = self
            .get_active_channels(true)
            .into_iter()
            .map(|c| c.to_string())
            .collect();
        let outputs: Vec<String> = self
            .get_active_channels(false)
            .into_iter()
            .map(|c| c.to_string())
            .collect();
        if !inputs.is_empty() {
            write!(f, "inputs: {} ", inputs.join(","))?;
        }
        if !outputs.is_empty() {
            write!(f, "outputs: {}", outputs.join(","))?;
        }
        Ok(())
    }
}
