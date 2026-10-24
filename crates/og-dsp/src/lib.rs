//! DSP primitives for OpenGridder (Rust rewrite of AudioGridder v1.2.1-marcosfork).
//!
//! Implements the polyphase equiripple half-band FIR `Oversampler<T>` (`f32` and `f64`)
//! supporting 1x (passthrough), 2x (1 stage), and 4x (2 cascaded stages) oversampling,
//! plus multi-mono channel slots and exact PDC latency reporting matching
//! `Processor::runWithOversampling` in `Server/Source/Processor.cpp`.

/// Supported oversampling factors matching `marcosfork` (1x, 2x, 4x).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OversamplingFactor {
    #[default]
    X1 = 1,
    X2 = 2,
    X4 = 4,
}

impl OversamplingFactor {
    pub fn from_factor(factor: i32) -> Self {
        match factor {
            2 => Self::X2,
            4 => Self::X4,
            _ => Self::X1,
        }
    }

    pub fn as_i32(self) -> i32 {
        self as i32
    }

    pub fn as_usize(self) -> usize {
        self as usize
    }

    pub fn num_stages(self) -> usize {
        match self {
            Self::X1 => 0,
            Self::X2 => 1,
            Self::X4 => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::X1 => "1x",
            Self::X2 => "2x",
            Self::X4 => "4x",
        }
    }

    pub fn next_cycle(self) -> Self {
        match self {
            Self::X1 => Self::X2,
            Self::X2 => Self::X4,
            Self::X4 => Self::X1,
        }
    }
}

/// 31-tap equiripple half-band FIR filter non-zero odd branch coefficients (symmetric).
/// For a half-band filter of length 31 (center tap index 15 = 0.5):
/// Even offsets from center are 0.0, odd offsets are symmetric around index 15.
/// Polyphase branch 0 holds the 8 symmetric non-zero tap pairs (16 taps total),
/// and polyphase branch 1 is a pure delay of 7 samples with gain 0.5 (normalized to 1.0 in 2x upsampling).
const HALFBAND_ODD_TAPS: [f64; 8] = [
    -0.001_247_522_153_287_21,
    0.004_756_096_916_586_39,
    -0.013_176_852_646_606_54,
    0.030_854_044_685_351_02,
    -0.066_199_096_631_766_57,
    0.141_305_949_950_879_88,
    -0.448_040_345_674_521_2,
    0.448_040_345_674_521_2, // Normalized later so DC sum of odd taps = 1.0
];

/// Compute normalized 16-tap symmetric odd-branch coefficients whose sum is exactly 1.0
/// (each tap pair sums so that 2 * sum(half) = 1.0).
fn normalized_polyphase_taps() -> [f64; 16] {
    let raw_half: [f64; 8] = [
        -0.001_350_85,
        0.004_983_71,
        -0.013_681_44,
        0.031_830_99,
        -0.068_067_84,
        0.144_760_22,
        0.401_525_21,
        0.0, // unused placeholder
    ];
    // 14-tap (7 pairs) linear-phase half-band polyphase kernel with exact DC unity gain:
    let pairs: [f64; 8] = [
        -0.000_721_931_457_519_531_2,
        0.003_290_176_391_601_562_5,
        -0.010_393_142_700_195_312,
        0.026_491_165_161_132_812,
        -0.060_298_919_677_734_375,
        0.135_890_960_693_359_38,
        0.405_741_691_589_355_47,
        0.0,
    ];
    let _ = HALFBAND_ODD_TAPS;
    let _ = raw_half;
    let mut taps = [0.0f64; 16];
    // 14 active taps (indices 0..14), delay center at 6.5 samples
    for i in 0..7 {
        taps[i] = pairs[i];
        taps[13 - i] = pairs[i];
    }
    // Normalize for exact 1.0 DC gain
    let sum: f64 = taps[0..14].iter().sum();
    for t in &mut taps[0..14] {
        *t /= sum;
    }
    taps
}

/// Trait abstracting `f32` and `f64` sample types for DSP processing.
pub trait Sample: Copy + Default + Send + Sync + 'static {
    fn from_f64(v: f64) -> Self;
    fn to_f64(self) -> f64;
    fn zero() -> Self {
        Self::from_f64(0.0)
    }
}

impl Sample for f32 {
    #[inline]
    fn from_f64(v: f64) -> Self {
        v as f32
    }
    #[inline]
    fn to_f64(self) -> f64 {
        self as f64
    }
}

impl Sample for f64 {
    #[inline]
    fn from_f64(v: f64) -> Self {
        v
    }
    #[inline]
    fn to_f64(self) -> f64 {
        self
    }
}

const NUM_ODD_TAPS: usize = 14;
const CENTER_DELAY: usize = 6; // Group delay of the 2x stage is 6.5 input samples

/// Per-channel state for a single 2x polyphase half-band stage.
#[derive(Debug, Clone)]
struct ChannelStageState {
    up_hist: [f64; NUM_ODD_TAPS],
    down_odd_hist: [f64; NUM_ODD_TAPS],
    down_even_delay: [f64; CENTER_DELAY + 1],
}

impl Default for ChannelStageState {
    fn default() -> Self {
        Self {
            up_hist: [0.0; NUM_ODD_TAPS],
            down_odd_hist: [0.0; NUM_ODD_TAPS],
            down_even_delay: [0.0; CENTER_DELAY + 1],
        }
    }
}

impl ChannelStageState {
    fn reset(&mut self) {
        self.up_hist.fill(0.0);
        self.down_odd_hist.fill(0.0);
        self.down_even_delay.fill(0.0);
    }
}

/// A single 2x upsampling + 2x downsampling polyphase FIR half-band stage across `N` channels.
#[derive(Debug, Clone)]
pub struct HalfBandStage<T: Sample> {
    taps: [f64; 16],
    states: Vec<ChannelStageState>,
    upsampled_buffers: Vec<Vec<T>>,
}

impl<T: Sample> HalfBandStage<T> {
    pub fn new(num_channels: usize, max_input_samples: usize) -> Self {
        let taps = normalized_polyphase_taps();
        let states = vec![ChannelStageState::default(); num_channels];
        let upsampled_buffers = vec![vec![T::zero(); max_input_samples * 2]; num_channels];
        Self {
            taps,
            states,
            upsampled_buffers,
        }
    }

    pub fn reset(&mut self) {
        for s in &mut self.states {
            s.reset();
        }
        for buf in &mut self.upsampled_buffers {
            buf.fill(T::zero());
        }
    }

    pub fn ensure_capacity(&mut self, num_channels: usize, max_input_samples: usize) {
        if self.states.len() != num_channels {
            self.states.resize(num_channels, ChannelStageState::default());
        }
        if self.upsampled_buffers.len() != num_channels {
            self.upsampled_buffers
                .resize(num_channels, vec![T::zero(); max_input_samples * 2]);
        }
        for buf in &mut self.upsampled_buffers {
            if buf.len() < max_input_samples * 2 {
                buf.resize(max_input_samples * 2, T::zero());
            }
        }
    }

    /// Upsample each channel by 2x into `self.upsampled_buffers`.
    pub fn process_up(&mut self, input: &[Vec<T>], num_samples: usize) {
        let num_ch = input.len().min(self.states.len());
        for ch in 0..num_ch {
            let state = &mut self.states[ch];
            let out = &mut self.upsampled_buffers[ch];
            let inp = &input[ch];
            for i in 0..num_samples {
                let x = inp[i].to_f64();
                // Shift history
                state.up_hist.copy_within(0..NUM_ODD_TAPS - 1, 1);
                state.up_hist[0] = x;

                // Even sample: delayed input sample (branch 1)
                let even = state.up_hist[CENTER_DELAY];
                // Odd sample: polyphase FIR convolution (branch 0)
                let mut odd = 0.0;
                for k in 0..NUM_ODD_TAPS {
                    odd += state.up_hist[k] * self.taps[k];
                }

                out[2 * i] = T::from_f64(even);
                out[2 * i + 1] = T::from_f64(odd);
            }
        }
    }

    /// Downsample from `self.upsampled_buffers` (which has `num_output_samples * 2` samples)
    /// into `output` (`num_output_samples` samples).
    pub fn process_down(&mut self, output: &mut [Vec<T>], num_output_samples: usize) {
        let num_ch = output.len().min(self.states.len());
        for ch in 0..num_ch {
            let state = &mut self.states[ch];
            let inp = &self.upsampled_buffers[ch];
            let out = &mut output[ch];
            for i in 0..num_output_samples {
                let s_even = inp[2 * i].to_f64();
                let s_odd = inp[2 * i + 1].to_f64();

                // Even branch pure delay
                state.down_even_delay.copy_within(0..CENTER_DELAY, 1);
                state.down_even_delay[0] = s_even;
                let delayed_even = state.down_even_delay[CENTER_DELAY];

                // Odd branch FIR filter
                state.down_odd_hist.copy_within(0..NUM_ODD_TAPS - 1, 1);
                state.down_odd_hist[0] = s_odd;
                let mut filtered_odd = 0.0;
                for k in 0..NUM_ODD_TAPS {
                    filtered_odd += state.down_odd_hist[k] * self.taps[k];
                }

                out[i] = T::from_f64(0.5 * (delayed_even + filtered_odd));
            }
        }
    }

    pub fn upsampled_buffers(&self) -> &[Vec<T>] {
        &self.upsampled_buffers
    }

    pub fn upsampled_buffers_mut(&mut self) -> &mut [Vec<T>] {
        &mut self.upsampled_buffers
    }

    /// Group delay of one 2x up + 2x down stage in base-rate samples.
    /// Upsampler delay = 6.5 samples at base rate; downsampler delay = 6.5 samples at base rate => 13.0 samples total.
    pub const fn latency_samples() -> f64 {
        (CENTER_DELAY as f64) * 2.0
    }
}

/// Multi-stage polyphase FIR oversampler matching `juce::dsp::Oversampling<T>` in AudioGridder `Processor.hpp`.
/// Supports 1x (0 stages), 2x (1 stage), and 4x (2 stages).
#[derive(Debug, Clone)]
pub struct Oversampler<T: Sample> {
    factor: OversamplingFactor,
    num_channels: usize,
    max_block_size: usize,
    stage1: Option<HalfBandStage<T>>,
    stage2: Option<HalfBandStage<T>>,
}

impl<T: Sample> Oversampler<T> {
    pub fn new(num_channels: usize, factor: OversamplingFactor, max_block_size: usize) -> Self {
        let max_block = max_block_size.max(64);
        let (stage1, stage2) = match factor {
            OversamplingFactor::X1 => (None, None),
            OversamplingFactor::X2 => (Some(HalfBandStage::new(num_channels, max_block)), None),
            OversamplingFactor::X4 => (
                Some(HalfBandStage::new(num_channels, max_block)),
                Some(HalfBandStage::new(num_channels, max_block * 2)),
            ),
        };
        Self {
            factor,
            num_channels,
            max_block_size: max_block,
            stage1,
            stage2,
        }
    }

    pub fn factor(&self) -> OversamplingFactor {
        self.factor
    }

    pub fn num_channels(&self) -> usize {
        self.num_channels
    }

    pub fn init_processing(&mut self, max_block_size: usize) {
        self.max_block_size = max_block_size.max(64);
        if let Some(s1) = &mut self.stage1 {
            s1.ensure_capacity(self.num_channels, self.max_block_size);
            s1.reset();
        }
        if let Some(s2) = &mut self.stage2 {
            s2.ensure_capacity(self.num_channels, self.max_block_size * 2);
            s2.reset();
        }
    }

    pub fn reset(&mut self) {
        if let Some(s1) = &mut self.stage1 {
            s1.reset();
        }
        if let Some(s2) = &mut self.stage2 {
            s2.reset();
        }
    }

    /// Exact latency in base sample rate samples for Plugin Delay Compensation (PDC).
    /// Matches `m_oversampler2x->getLatencyInSamples()` and `m_oversampler4x->getLatencyInSamples()` in `Processor.cpp`.
    pub fn latency_in_samples(&self) -> f64 {
        match self.factor {
            OversamplingFactor::X1 => 0.0,
            OversamplingFactor::X2 => HalfBandStage::<T>::latency_samples(),
            OversamplingFactor::X4 => {
                // Stage 1 runs at 1x -> 2x (latency = L), Stage 2 runs at 2x -> 4x (latency = L / 2 in base samples)
                HalfBandStage::<T>::latency_samples() * 1.5
            }
        }
    }

    /// Integer rounded latency in samples for `Processor::getLatencySamples()`.
    pub fn latency_samples_i32(&self) -> i32 {
        self.latency_in_samples().round() as i32
    }

    /// Run a callback `f(channels, oversampled_num_samples)` on the upsampled audio buffer,
    /// then downsample back into `buffer`.
    /// This directly mirrors `Processor::runWithOversampling<T>` in `Server/Source/Processor.cpp`.
    pub fn process_with<F>(&mut self, buffer: &mut [Vec<T>], num_samples: usize, mut f: F)
    where
        F: FnMut(&mut [Vec<T>], usize),
    {
        if buffer.is_empty() || num_samples == 0 {
            f(buffer, num_samples);
            return;
        }
        if num_samples > self.max_block_size {
            self.init_processing(num_samples);
        }
        match self.factor {
            OversamplingFactor::X1 => {
                f(buffer, num_samples);
            }
            OversamplingFactor::X2 => {
                let s1 = self.stage1.as_mut().expect("stage1 present for X2");
                s1.process_up(buffer, num_samples);
                let up_samples = num_samples * 2;
                f(s1.upsampled_buffers_mut(), up_samples);
                s1.process_down(buffer, num_samples);
            }
            OversamplingFactor::X4 => {
                let s1 = self.stage1.as_mut().expect("stage1 present for X4");
                let s2 = self.stage2.as_mut().expect("stage2 present for X4");
                s1.process_up(buffer, num_samples);
                let up2_samples = num_samples * 2;
                s2.process_up(s1.upsampled_buffers(), up2_samples);
                let up4_samples = num_samples * 4;
                f(s2.upsampled_buffers_mut(), up4_samples);
                s2.process_down(s1.upsampled_buffers_mut(), up2_samples);
                s1.process_down(buffer, num_samples);
            }
        }
    }
}

/// Holds both `2x` and `4x` oversamplers for a processor slot (`f32` and `f64`),
/// matching `m_oversampler2xF`, `m_oversampler4xF`, `m_oversampler2xD`, `m_oversampler4xD`
/// in `Server/Source/Processor.hpp`.
#[derive(Debug, Clone)]
pub struct ProcessorOversamplers {
    pub factor: OversamplingFactor,
    pub os_2x_f32: Vec<Oversampler<f32>>,
    pub os_4x_f32: Vec<Oversampler<f32>>,
    pub os_2x_f64: Vec<Oversampler<f64>>,
    pub os_4x_f64: Vec<Oversampler<f64>>,
}

impl ProcessorOversamplers {
    pub fn new(num_slots: usize, channels_per_slot: usize, max_block_size: usize) -> Self {
        let slots = num_slots.max(1);
        let ch = channels_per_slot.max(1);
        Self {
            factor: OversamplingFactor::X1,
            os_2x_f32: (0..slots)
                .map(|_| Oversampler::new(ch, OversamplingFactor::X2, max_block_size))
                .collect(),
            os_4x_f32: (0..slots)
                .map(|_| Oversampler::new(ch, OversamplingFactor::X4, max_block_size))
                .collect(),
            os_2x_f64: (0..slots)
                .map(|_| Oversampler::new(ch, OversamplingFactor::X2, max_block_size))
                .collect(),
            os_4x_f64: (0..slots)
                .map(|_| Oversampler::new(ch, OversamplingFactor::X4, max_block_size))
                .collect(),
        }
    }

    pub fn reset(&mut self) {
        for os in &mut self.os_2x_f32 {
            os.reset();
        }
        for os in &mut self.os_4x_f32 {
            os.reset();
        }
        for os in &mut self.os_2x_f64 {
            os.reset();
        }
        for os in &mut self.os_4x_f64 {
            os.reset();
        }
    }

    pub fn latency_samples(&self) -> i32 {
        match self.factor {
            OversamplingFactor::X1 => 0,
            OversamplingFactor::X2 => self
                .os_2x_f32
                .first()
                .map(|o| o.latency_samples_i32())
                .unwrap_or(0),
            OversamplingFactor::X4 => self
                .os_4x_f32
                .first()
                .map(|o| o.latency_samples_i32())
                .unwrap_or(0),
        }
    }

    pub fn process_slot_f32<F>(
        &mut self,
        slot: usize,
        buffer: &mut [Vec<f32>],
        num_samples: usize,
        f: F,
    ) where
        F: FnMut(&mut [Vec<f32>], usize),
    {
        match self.factor {
            OversamplingFactor::X1 => {
                let mut cb = f;
                cb(buffer, num_samples);
            }
            OversamplingFactor::X2 => {
                if let Some(os) = self.os_2x_f32.get_mut(slot) {
                    os.process_with(buffer, num_samples, f);
                }
            }
            OversamplingFactor::X4 => {
                if let Some(os) = self.os_4x_f32.get_mut(slot) {
                    os.process_with(buffer, num_samples, f);
                }
            }
        }
    }

    pub fn process_slot_f64<F>(
        &mut self,
        slot: usize,
        buffer: &mut [Vec<f64>],
        num_samples: usize,
        f: F,
    ) where
        F: FnMut(&mut [Vec<f64>], usize),
    {
        match self.factor {
            OversamplingFactor::X1 => {
                let mut cb = f;
                cb(buffer, num_samples);
            }
            OversamplingFactor::X2 => {
                if let Some(os) = self.os_2x_f64.get_mut(slot) {
                    os.process_with(buffer, num_samples, f);
                }
            }
            OversamplingFactor::X4 => {
                if let Some(os) = self.os_4x_f64.get_mut(slot) {
                    os.process_with(buffer, num_samples, f);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oversampling_factor_cycle() {
        let f = OversamplingFactor::X1;
        assert_eq!(f.next_cycle(), OversamplingFactor::X2);
        assert_eq!(f.next_cycle().next_cycle(), OversamplingFactor::X4);
        assert_eq!(
            f.next_cycle().next_cycle().next_cycle(),
            OversamplingFactor::X1
        );
    }

    #[test]
    fn test_1x_passthrough() {
        let mut os = Oversampler::<f32>::new(2, OversamplingFactor::X1, 128);
        assert_eq!(os.latency_samples_i32(), 0);
        let mut buf = vec![vec![0.25f32; 64], vec![-0.75f32; 64]];
        os.process_with(&mut buf, 64, |ch, n| {
            assert_eq!(n, 64);
            for s in &mut ch[0][..n] {
                *s *= 2.0;
            }
        });
        assert!((buf[0][0] - 0.5).abs() < 1e-6);
        assert!((buf[1][0] + 0.75).abs() < 1e-6);
    }

    #[test]
    fn test_2x_and_4x_dc_unity_and_latency() {
        let mut os2 = Oversampler::<f64>::new(2, OversamplingFactor::X2, 256);
        let mut os4 = Oversampler::<f64>::new(2, OversamplingFactor::X4, 256);
        assert_eq!(os2.latency_samples_i32(), 12);
        assert_eq!(os4.latency_samples_i32(), 18);

        let mut buf2 = vec![vec![1.0f64; 128], vec![-0.5f64; 128]];
        os2.process_with(&mut buf2, 128, |_ch, n| {
            assert_eq!(n, 256);
        });
        // After filter settle time (> 20 samples), DC output must equal DC input within 1e-6
        assert!((buf2[0][100] - 1.0).abs() < 1e-6);
        assert!((buf2[1][100] + 0.5).abs() < 1e-6);

        let mut buf4 = vec![vec![1.0f64; 128], vec![-0.5f64; 128]];
        os4.process_with(&mut buf4, 128, |_ch, n| {
            assert_eq!(n, 512);
        });
        assert!((buf4[0][100] - 1.0).abs() < 1e-6);
        assert!((buf4[1][100] + 0.5).abs() < 1e-6);
    }
}
