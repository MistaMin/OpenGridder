//! Server channel map from AudioGridder `ChannelSet` and `ChannelMapper`.
//!
//! Output bits start at `PLUGIN_CHANNELS_MAX / 2` when the chain has an input.
//! The server mapping inverts the plugin direction. A forward copy reads the
//! packed index and writes the original channel. The reverse copy does the
//! opposite. This host has no extra plugin buses, so the proc width is
//! `max(channelsIn + channelsSC, channelsOut)`.

pub const PLUGIN_CHANNELS_MAX: usize = 64;

#[derive(Clone, Copy)]
pub struct ChannelMap {
    fwd: [i32; PLUGIN_CHANNELS_MAX],
    rev: [i32; PLUGIN_CHANNELS_MAX],
}

impl ChannelMap {
    /// `ChannelMapper::createServerMapping` after `setWithInput` and `setNumChannels`.
    pub fn server(bits: u64, channels_in: i32, channels_sc: i32, channels_out: i32) -> Self {
        let mut map = Self {
            fwd: [-1; PLUGIN_CHANNELS_MAX],
            rev: [-1; PLUGIN_CHANNELS_MAX],
        };
        let output_offset = if channels_in > 0 { PLUGIN_CHANNELS_MAX / 2 } else { 0 };
        let num_inputs = channels_in.saturating_add(channels_sc).clamp(0, PLUGIN_CHANNELS_MAX as i32);
        let num_outputs = channels_out.clamp(0, PLUGIN_CHANNELS_MAX as i32);
        let combined = num_inputs.max(num_outputs);
        if num_inputs != 0 {
            let mut ch_dst = 0i32;
            for ch_src in 0..combined {
                if !input_bit(bits, ch_src) {
                    continue;
                }
                if (ch_src as usize) < PLUGIN_CHANNELS_MAX {
                    map.rev[ch_src as usize] = ch_dst;
                }
                if output_bit(bits, ch_src, output_offset) && (ch_dst as usize) < PLUGIN_CHANNELS_MAX {
                    map.fwd[ch_dst as usize] = ch_src;
                }
                ch_dst += 1;
            }
        } else {
            let mut ch_dst = 0i32;
            for ch_src in 0..combined {
                if output_bit(bits, ch_src, output_offset) && (ch_src as usize) < PLUGIN_CHANNELS_MAX {
                    map.rev[ch_src as usize] = ch_dst;
                    ch_dst += 1;
                }
            }
        }
        map
    }

    pub fn forward(&self) -> &[i32; PLUGIN_CHANNELS_MAX] {
        &self.fwd
    }

    pub fn reverse(&self) -> &[i32; PLUGIN_CHANNELS_MAX] {
        &self.rev
    }
}

/// Popcount of the input bits in `0..channelsIn+channelsSC`.
pub fn active_input_count(bits: u64, channels_in: i32, channels_sc: i32) -> i32 {
    let num_inputs = channels_in.saturating_add(channels_sc).clamp(0, PLUGIN_CHANNELS_MAX as i32);
    (0..num_inputs).filter(|ch| input_bit(bits, *ch)).count() as i32
}

pub fn chain_width(channels_in: i32, channels_sc: i32, channels_out: i32) -> usize {
    let inputs = channels_in.saturating_add(channels_sc).max(0) as usize;
    let outputs = channels_out.max(0) as usize;
    inputs.max(outputs)
}

/// `ChannelMapper::mapInternal`. Unmapped destination channels are cleared.
pub fn apply_map<T: Copy + Default>(src: &[Vec<T>], dst: &mut [Vec<T>], table: &[i32; PLUGIN_CHANNELS_MAX]) {
    let mut mapped = [false; PLUGIN_CHANNELS_MAX];
    for (ch, channel) in src.iter().enumerate() {
        if ch >= PLUGIN_CHANNELS_MAX {
            break;
        }
        let ch_mapped = table[ch];
        if ch_mapped < 0 {
            continue;
        }
        let ch_dst = ch_mapped as usize;
        if ch_dst >= dst.len() || ch_dst >= PLUGIN_CHANNELS_MAX {
            continue;
        }
        if channel.len() != dst[ch_dst].len() {
            continue;
        }
        dst[ch_dst].copy_from_slice(channel);
        mapped[ch_dst] = true;
    }
    for (ch, channel) in dst.iter_mut().enumerate() {
        if ch >= PLUGIN_CHANNELS_MAX || !mapped[ch] {
            channel.fill(T::default());
        }
    }
}

fn input_bit(bits: u64, channel: i32) -> bool {
    (0..PLUGIN_CHANNELS_MAX as i32).contains(&channel) && ((bits >> channel) & 1) == 1
}

fn output_bit(bits: u64, channel: i32, output_offset: usize) -> bool {
    if channel < 0 {
        return false;
    }
    let index = channel as usize + output_offset;
    index < PLUGIN_CHANNELS_MAX && ((bits >> index) & 1) == 1
}
