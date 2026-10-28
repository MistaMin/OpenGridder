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

/// Which half-band a stage runs. The first stage works at the host rate and must keep
/// the audible band flat up to about 20 kHz at 44.1 kHz; the second stage starts from a
/// signal that is already band-limited, so a much shorter filter is enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageKind {
    /// 109 taps, ~80 dB stop band, transition 20.0 -> 24.1 kHz at 44.1 kHz.
    Tight,
    /// 21 taps for the second 2x step of 4x.
    Loose,
}

impl StageKind {
    /// Half the filter order: the centre tap index of the symmetric FIR (always even).
    const fn centre(self) -> usize {
        match self {
            StageKind::Tight => 54,
            StageKind::Loose => 10,
        }
    }
}

/// Modified Bessel function of the first kind, order zero: sum_k ((x/2)^k / k!)^2.
fn bessel_i0(x: f64) -> f64 {
    let half = x / 2.0;
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..80 {
        term *= half / k as f64;
        sum += term * term;
    }
    sum
}

/// Kaiser-windowed half-band low-pass with cut-off at a quarter of the oversampled rate.
/// Returns the odd-offset taps `h[1], h[3], ... h[2C-1]` (the even taps are zero except the
/// centre 0.5), normalised so the odd taps sum to exactly 0.5 (unity DC gain).
fn halfband_odd_taps(kind: StageKind) -> Vec<f64> {
    let c = kind.centre();
    let beta = 7.86; // ~80 dB stop-band attenuation
    let n = 2 * c;
    let i0_beta = bessel_i0(beta);
    let mut odd = Vec::with_capacity(c);
    for r in 0..c {
        let k = 2 * r + 1; // tap index measured from the filter start
        let off = k as f64 - c as f64; // distance from the centre tap (odd)
        let x = off / 2.0;
        let sinc = (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x);
        let ratio = (2.0 * k as f64 / n as f64) - 1.0;
        let w = bessel_i0(beta * (1.0 - ratio * ratio).max(0.0).sqrt()) / i0_beta;
        odd.push(0.5 * sinc * w);
    }
    let sum: f64 = odd.iter().sum();
    for tap in &mut odd {
        *tap *= 0.5 / sum;
    }
    odd
}

#[derive(Debug, Clone)]
struct ChannelStageState {
    up_hist: Vec<f64>,
    down_even_hist: Vec<f64>,
    down_odd_hist: Vec<f64>,
}

impl ChannelStageState {
    fn new(c: usize) -> Self {
        Self {
            up_hist: vec![0.0; c],
            down_even_hist: vec![0.0; c / 2],
            down_odd_hist: vec![0.0; c],
        }
    }

    fn reset(&mut self) {
        self.up_hist.fill(0.0);
        self.down_even_hist.fill(0.0);
        self.down_odd_hist.fill(0.0);
    }
}

/// A single 2x upsampling + 2x downsampling polyphase half-band stage across `N` channels.
#[derive(Debug, Clone)]
pub struct HalfBandStage<T: Sample> {
    kind: StageKind,
    /// `odd[r] = h[2r + 1]`, the non-zero half-band taps.
    odd: Vec<f64>,
    states: Vec<ChannelStageState>,
    upsampled_buffers: Vec<Vec<T>>,
    scratch_a: Vec<f64>,
    scratch_b: Vec<f64>,
}

impl<T: Sample> HalfBandStage<T> {
    pub fn new(num_channels: usize, max_input_samples: usize, kind: StageKind) -> Self {
        let c = kind.centre();
        Self {
            kind,
            odd: halfband_odd_taps(kind),
            states: (0..num_channels).map(|_| ChannelStageState::new(c)).collect(),
            upsampled_buffers: vec![vec![T::zero(); max_input_samples * 2]; num_channels],
            scratch_a: vec![0.0; c + max_input_samples],
            scratch_b: vec![0.0; c + max_input_samples],
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
        let c = self.kind.centre();
        while self.states.len() < num_channels {
            self.states.push(ChannelStageState::new(c));
        }
        self.states.truncate(num_channels.max(1));
        if self.upsampled_buffers.len() != num_channels {
            self.upsampled_buffers
                .resize(num_channels, vec![T::zero(); max_input_samples * 2]);
        }
        for buf in &mut self.upsampled_buffers {
            if buf.len() < max_input_samples * 2 {
                buf.resize(max_input_samples * 2, T::zero());
            }
        }
        for s in [&mut self.scratch_a, &mut self.scratch_b] {
            if s.len() < c + max_input_samples {
                s.resize(c + max_input_samples, 0.0);
            }
        }
    }

    /// Upsample each channel by 2x into `self.upsampled_buffers`.
    pub fn process_up(&mut self, input: &[Vec<T>], num_samples: usize) {
        let c = self.kind.centre();
        let num_ch = input.len().min(self.states.len());
        if self.scratch_a.len() < c + num_samples {
            self.scratch_a.resize(c + num_samples, 0.0);
        }
        for ch in 0..num_ch {
            let state = &mut self.states[ch];
            let out = &mut self.upsampled_buffers[ch];
            let x = &mut self.scratch_a;
            x[..c].copy_from_slice(&state.up_hist);
            for i in 0..num_samples {
                x[c + i] = input[ch][i].to_f64();
            }
            for i in 0..num_samples {
                let pos = c + i;
                let mut odd = 0.0;
                for (r, tap) in self.odd.iter().enumerate() {
                    odd += 2.0 * tap * x[pos - r];
                }
                out[2 * i] = T::from_f64(x[pos - c / 2]);
                out[2 * i + 1] = T::from_f64(odd);
            }
            state.up_hist.copy_from_slice(&x[num_samples..num_samples + c]);
        }
    }

    /// Downsample from `self.upsampled_buffers` (which has `num_output_samples * 2` samples)
    /// into `output` (`num_output_samples` samples).
    pub fn process_down(&mut self, output: &mut [Vec<T>], num_output_samples: usize) {
        let c = self.kind.centre();
        let half = c / 2;
        let n = num_output_samples;
        let num_ch = output.len().min(self.states.len());
        if self.scratch_a.len() < c + n {
            self.scratch_a.resize(c + n, 0.0);
        }
        if self.scratch_b.len() < c + n {
            self.scratch_b.resize(c + n, 0.0);
        }
        for ch in 0..num_ch {
            let state = &mut self.states[ch];
            let inp = &self.upsampled_buffers[ch];
            let se = &mut self.scratch_a;
            let so = &mut self.scratch_b;
            se[..half].copy_from_slice(&state.down_even_hist);
            so[..c].copy_from_slice(&state.down_odd_hist);
            for i in 0..n {
                se[half + i] = inp[2 * i].to_f64();
                so[c + i] = inp[2 * i + 1].to_f64();
            }
            for i in 0..n {
                // z[i] = 0.5 * even[i - C/2] + sum_{r=1..C} h[2r - 1] * odd[i - r]
                let mut acc = 0.5 * se[i];
                for (idx, tap) in self.odd.iter().enumerate() {
                    acc += tap * so[c + i - (idx + 1)];
                }
                output[ch][i] = T::from_f64(acc);
            }
            state.down_even_hist.copy_from_slice(&se[n..n + half]);
            state.down_odd_hist.copy_from_slice(&so[n..n + c]);
        }
    }

    pub fn upsampled_buffers(&self) -> &[Vec<T>] {
        &self.upsampled_buffers
    }

    pub fn upsampled_buffers_mut(&mut self) -> &mut [Vec<T>] {
        &mut self.upsampled_buffers
    }

    /// Group delay of one 2x up + 2x down stage, in samples at the stage's own input rate.
    pub fn latency_samples(&self) -> f64 {
        self.kind.centre() as f64
    }
}

/// Multi-stage polyphase FIR oversampler.
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
            OversamplingFactor::X2 => (Some(HalfBandStage::new(num_channels, max_block, StageKind::Tight)), None),
            OversamplingFactor::X4 => (
                Some(HalfBandStage::new(num_channels, max_block, StageKind::Tight)),
                Some(HalfBandStage::new(num_channels, max_block * 2, StageKind::Loose)),
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
            OversamplingFactor::X2 => self.stage1.as_ref().map_or(0.0, HalfBandStage::latency_samples),
            OversamplingFactor::X4 => {
                // Stage 2 runs at twice the host rate, so its delay counts half in host samples.
                self.stage1.as_ref().map_or(0.0, HalfBandStage::latency_samples)
                    + self.stage2.as_ref().map_or(0.0, HalfBandStage::latency_samples) / 2.0
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
        assert_eq!(os2.latency_samples_i32(), 54);
        assert_eq!(os4.latency_samples_i32(), 59);

        let mut buf2 = vec![vec![1.0f64; 256], vec![-0.5f64; 256]];
        os2.process_with(&mut buf2, 256, |_ch, n| {
            assert_eq!(n, 512);
        });
        // After the filter has settled (a bit over the total latency), DC output must equal DC input within 1e-6
        assert!((buf2[0][240] - 1.0).abs() < 1e-6);
        assert!((buf2[1][240] + 0.5).abs() < 1e-6);

        let mut buf4 = vec![vec![1.0f64; 256], vec![-0.5f64; 256]];
        os4.process_with(&mut buf4, 256, |_ch, n| {
            assert_eq!(n, 1024);
        });
        assert!((buf4[0][240] - 1.0).abs() < 1e-6);
        assert!((buf4[1][240] + 0.5).abs() < 1e-6);
    }

    fn gain_db(factor: OversamplingFactor, freq: f64, rate: f64) -> f64 {
        let mut os = Oversampler::<f64>::new(1, factor, 4096);
        let total = 4096;
        let mut buf = vec![(0..total)
            .map(|n| (2.0 * std::f64::consts::PI * freq * n as f64 / rate).sin())
            .collect::<Vec<f64>>()];
        let (mut out, mut done) = (Vec::new(), 0);
        while done < total {
            let n = 256.min(total - done);
            let mut block = vec![buf[0][done..done + n].to_vec()];
            os.process_with(&mut block, n, |_c, _n| {});
            out.extend_from_slice(&block[0]);
            done += n;
        }
        buf.clear();
        let rms = |s: &[f64]| (s.iter().map(|v| v * v).sum::<f64>() / s.len() as f64).sqrt();
        let tail = &out[2048..];
        20.0 * (rms(tail) / (1.0 / 2f64.sqrt())).log10()
    }

    #[test]
    fn passband_is_flat_to_20khz_at_44_1k() {
        for factor in [OversamplingFactor::X2, OversamplingFactor::X4] {
            for freq in [100.0, 1_000.0, 10_000.0, 16_000.0, 18_000.0, 19_500.0] {
                let g = gain_db(factor, freq, 44_100.0);
                assert!(g.abs() < 0.1, "{factor:?} {freq} Hz: {g:.3} dB");
            }
        }
    }

    #[test]
    fn aliases_above_nyquist_are_rejected_by_the_up_down_pair() {
        // A tone only just inside Nyquist survives; the converter must not add level.
        let g = gain_db(OversamplingFactor::X2, 21_500.0, 44_100.0);
        assert!(g < 0.5, "unexpected boost {g:.3} dB");
    }
}
