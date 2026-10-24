//! Port of `Common/Source/ChannelMapper.hpp`.

use std::collections::{HashMap, HashSet};

use crate::channel_set::ChannelSet;

#[derive(Debug, Clone, Default)]
pub struct ChannelMapper {
    fwd_map: HashMap<usize, usize>,
    rev_map: HashMap<usize, usize>,
}

impl ChannelMapper {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_mapping(active_channels: &ChannelSet, plugin_mode: bool) -> Self {
        let mut mapper = Self::new();
        mapper.create_mapping_internal(active_channels, plugin_mode);
        mapper
    }

    pub fn create_plugin_mapping(&mut self, active_channels: &ChannelSet) {
        self.create_mapping_internal(active_channels, true);
    }

    pub fn create_server_mapping(&mut self, active_channels: &ChannelSet) {
        self.create_mapping_internal(active_channels, false);
    }

    pub fn reset(&mut self) {
        self.fwd_map.clear();
        self.rev_map.clear();
    }

    pub fn map<T: Copy + Default>(&self, src: &[Vec<T>], dst: &mut [Vec<T>]) {
        self.map_internal(src, dst, false);
    }

    pub fn map_reverse<T: Copy + Default>(&self, src: &[Vec<T>], dst: &mut [Vec<T>]) {
        self.map_internal(src, dst, true);
    }

    pub fn get_mapped_channel(&self, ch: usize) -> Option<usize> {
        self.fwd_map.get(&ch).copied()
    }

    pub fn get_mapped_channel_reverse(&self, ch: usize) -> Option<usize> {
        self.rev_map.get(&ch).copied()
    }

    fn create_mapping_internal(&mut self, active_channels: &ChannelSet, plugin_mode: bool) {
        self.reset();
        let mut ch_dst = 0usize;
        if active_channels.get_num_channels(true) > 0 {
            // FX mode
            for ch_src in 0..active_channels.get_num_channels_combined() {
                if active_channels.is_input_active(ch_src) {
                    if plugin_mode {
                        self.fwd_map.insert(ch_src, ch_dst);
                        if active_channels.is_output_active(ch_src) {
                            self.rev_map.insert(ch_dst, ch_src);
                        }
                    } else {
                        self.rev_map.insert(ch_src, ch_dst);
                        if active_channels.is_output_active(ch_src) {
                            self.fwd_map.insert(ch_dst, ch_src);
                        }
                    }
                    ch_dst += 1;
                }
            }
        } else {
            // Instrument mode
            for ch_src in 0..active_channels.get_num_channels_combined() {
                if active_channels.is_output_active(ch_src) {
                    if plugin_mode {
                        self.rev_map.insert(ch_dst, ch_src);
                    } else {
                        self.rev_map.insert(ch_src, ch_dst);
                    }
                    ch_dst += 1;
                }
            }
        }
    }

    fn map_internal<T: Copy + Default>(&self, src: &[Vec<T>], dst: &mut [Vec<T>], reverse: bool) {
        let mut mapped = HashSet::new();
        for (ch, src_chan) in src.iter().enumerate() {
            let ch_mapped = if reverse {
                self.get_mapped_channel_reverse(ch)
            } else {
                self.get_mapped_channel(ch)
            };
            if let Some(dst_idx) = ch_mapped {
                if dst_idx < dst.len() && src_chan.len() == dst[dst_idx].len() {
                    dst[dst_idx].copy_from_slice(src_chan);
                    mapped.insert(dst_idx);
                }
            }
        }
        for (ch, dst_chan) in dst.iter_mut().enumerate() {
            if !mapped.contains(&ch) {
                dst_chan.fill(T::default());
            }
        }
    }
}
