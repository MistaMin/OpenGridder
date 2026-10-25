//! Console Drive / Saturation stage (realtime-safe: no allocation after `new`).
//!
//! * Waveshaper: `tanh` soft clipper, optionally biased (asymmetric) for even harmonics.
//! * Anti-aliasing: first-order ADAA (zero latency), or 2x / 4x linear-phase FIR oversampling.
//! * Character: Clean (symmetric), `Tb` (tube-style bias + DC blocker),
//!   `Tr` (transformer-style bias, low-frequency-weighted saturation + DC blocker).
//!
//! Oversampled modes have a fixed integer-sample group delay (`quality_latency`). The
//! caller compensates it by moving the dry/wet ring alignment earlier by the same amount
//! so the *reported* plugin latency never changes when the quality is switched.

pub const QUALITY_ADAA: u32 = 0;
pub const QUALITY_2X: u32 = 1;
pub const QUALITY_4X: u32 = 2;

pub const CHAR_CLEAN: u32 = 0;
pub const CHAR_TUBE: u32 = 1;
pub const CHAR_TRANSFORMER: u32 = 2;

/// Linear-phase FIR length for each 2x stage. `4k+1` gives an integer base-rate delay.
const FIR_LEN: usize = 41;
const DELAY_LEN: usize = 64;

/// Extra latency in base-rate samples introduced by the stage for a given quality.
pub fn quality_latency(quality: u32) -> usize {
    match quality {
        QUALITY_2X => (FIR_LEN - 1) / 2,                       // 20
        QUALITY_4X => (FIR_LEN - 1) / 2 + (FIR_LEN - 1) / 4,   // 30
        _ => 0,
    }
}

/// Smoothed shaper parameters for one sample.
#[derive(Clone, Copy)]
pub struct ShapeParams {
    pub gain: f32,
    pub bias: f32,
    pub norm: f32,
    pub slope: f32,
}

impl ShapeParams {
    pub fn new(gain: f32, character: u32) -> Self {
        let bias = match character {
            CHAR_TUBE => 0.35,
            CHAR_TRANSFORMER => 0.10,
            _ => 0.0,
        };
        let norm = (0.5 * ((gain + bias).tanh() + (gain - bias).tanh())).max(1e-4);
        let slope = (gain * (1.0 - bias.tanh().powi(2))) / norm;
        Self { gain, bias, norm, slope }
    }
}

#[inline]
pub fn shape(x: f32, p: &ShapeParams) -> f32 {
    ((p.gain * x + p.bias).tanh() - p.bias.tanh()) / p.norm
}

/// ln(cosh(u)) computed without overflow.
#[inline]
pub fn log_cosh(u: f32) -> f32 {
    let a = u.abs();
    a + (-2.0 * a).exp().ln_1p() - std::f32::consts::LN_2
}

/// Antiderivative of the shaper.
#[inline]
pub fn shape_antiderivative(x: f32, p: &ShapeParams) -> f32 {
    (log_cosh(p.gain * x + p.bias) / p.gain - p.bias.tanh() * x) / p.norm
}

/// HybridEQ-style Residual ADAA engine (Source/DSP/Adaa.h `dsp::adaa::Residual`).
///
/// Plain first-order ADAA replaces f(x_n) with the mean of f over [x_{n-1}, x_n],
/// which acts as a two-sample average introducing a half-sample delay and significant
/// high-frequency droop (approx -2 dB at 10 kHz, -11.7 dB at 20 kHz at 48 kHz).
///
/// The HybridEQ Residual method splits the function into linear and nonlinear parts:
///   y = slope * x + r(x),   where r(x) = f(x) - slope * x.
/// ADAA is applied to the residual r(x) only. The linear part goes through a first-order
/// all-pass filter (1/3 + z^-1) / (1 + 1/3 z^-1), which has the exact same half-sample group
/// delay at low frequencies but identically 1.000000 (0.000 dB) magnitude response up to Nyquist!
/// As the stage saturates, a knee weight smoothly crossfades the linear part into the plain
/// two-sample average so anti-aliasing cancellation remains exact in hard clipping.
#[derive(Clone, Default)]
pub struct ResidualAdaa {
    x1: f32,
    f1: f32,
    r1: f32,
    apx: f32,
    apy: f32,
    primed: bool,
}

impl ResidualAdaa {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.primed = false;
        self.x1 = 0.0;
        self.f1 = 0.0;
        self.r1 = 0.0;
        self.apx = 0.0;
        self.apy = 0.0;
    }

    #[inline]
    pub fn step(&mut self, x: f32, f: f32, f_int: f32, slope: f32) -> f32 {
        let lin_in = slope * x;
        if !self.primed {
            self.apx = lin_in;
            self.apy = lin_in;
        }
        // First-order all-pass (a + z^-1) / (1 + a z^-1), a = 1/3
        let lin = (1.0 / 3.0) * (lin_in - self.apy) + self.apx;
        self.apx = lin_in;
        self.apy = lin;

        let r = f - lin_in;
        let fr = f_int - 0.5 * slope * x * x;
        let mut result = lin + r;

        if self.primed {
            let dx = x - self.x1;
            if dx.abs() > 1e-5 {
                let mut mean_r = (fr - self.f1) / dx;
                let lo = r.min(self.r1);
                let hi = r.max(self.r1);
                let tol = (hi - lo) + 1e-6 * (1.0 + r.abs());
                if mean_r < lo - tol || mean_r > hi + tol {
                    mean_r = r;
                }
                let c = if x.abs() > 1e-9 && lin_in.abs() > 1e-9 {
                    (1.0 - f / lin_in).abs()
                } else {
                    0.0
                };
                let c2 = c * c;
                let c4 = c2 * c2;
                let c8 = c4 * c4;
                const K_KNEE: f32 = 0.3;
                const K8: f32 = K_KNEE * K_KNEE * K_KNEE * K_KNEE * K_KNEE * K_KNEE * K_KNEE * K_KNEE;
                let w = c8 / (c8 + K8);
                result = lin + mean_r + w * (0.5 * slope * (x + self.x1) - lin);
            }
        }
        self.x1 = x;
        self.f1 = fr;
        self.r1 = r;
        self.primed = true;
        result
    }
}

#[derive(Clone)]
struct Hist {
    buf: [f32; FIR_LEN * 2],
    pos: usize,
}

impl Hist {
    fn new() -> Self {
        Self { buf: [0.0; FIR_LEN * 2], pos: 0 }
    }
    #[inline]
    fn push(&mut self, x: f32) {
        self.pos = if self.pos == 0 { FIR_LEN - 1 } else { self.pos - 1 };
        self.buf[self.pos] = x;
        self.buf[self.pos + FIR_LEN] = x;
    }
    #[inline]
    fn convolve(&self, taps: &[f32; FIR_LEN]) -> f32 {
        let w = &self.buf[self.pos..self.pos + FIR_LEN];
        let mut acc = 0.0f32;
        for k in 0..FIR_LEN {
            acc += taps[k] * w[k];
        }
        acc
    }
}

#[derive(Clone)]
struct Resampler2 {
    up: Hist,
    down: Hist,
}

impl Resampler2 {
    fn new() -> Self {
        Self { up: Hist::new(), down: Hist::new() }
    }
    #[inline]
    fn upsample(&mut self, x: f32, taps: &[f32; FIR_LEN]) -> [f32; 2] {
        self.up.push(x);
        let a = 2.0 * self.up.convolve(taps);
        self.up.push(0.0);
        let b = 2.0 * self.up.convolve(taps);
        [a, b]
    }
    #[inline]
    fn downsample(&mut self, a: f32, b: f32, taps: &[f32; FIR_LEN]) -> f32 {
        // Evaluate on the even (first) phase so the group delay is exactly (FIR_LEN-1) high-rate samples.
        self.down.push(a);
        let y = self.down.convolve(taps);
        self.down.push(b);
        y
    }
}

#[derive(Clone)]
struct Channel {
    adaa: ResidualAdaa,
    r1: Resampler2,
    r2: Resampler2,
    dc_x1: f32,
    dc_y1: f32,
    lf: f32,
    delay: [f32; DELAY_LEN],
    delay_pos: usize,
}

impl Channel {
    fn new() -> Self {
        Self {
            adaa: ResidualAdaa::new(),
            r1: Resampler2::new(),
            r2: Resampler2::new(),
            dc_x1: 0.0,
            dc_y1: 0.0,
            lf: 0.0,
            delay: [0.0; DELAY_LEN],
            delay_pos: 0,
        }
    }
}

pub struct DriveStage {
    taps: [f32; FIR_LEN],
    channels: Vec<Channel>,
    dc_r: f32,
    lf_a: f32,
}

fn bessel_i0(x: f32) -> f32 {
    let mut sum = 1.0f32;
    let mut term = 1.0f32;
    let q = x * x * 0.25;
    for k in 1..30 {
        term *= q / (k * k) as f32;
        sum += term;
    }
    sum
}

impl DriveStage {
    pub fn new(channels: usize, sample_rate: f64) -> Self {
        // Kaiser-windowed sinc low-pass at the 2x-stage rate; cutoff 0.22 of that rate.
        let fc = 0.22f32;
        let beta = 7.0f32;
        let m = (FIR_LEN - 1) as f32 * 0.5;
        let mut taps = [0.0f32; FIR_LEN];
        let denom = bessel_i0(beta);
        for (n, t) in taps.iter_mut().enumerate() {
            let x = n as f32 - m;
            let sinc = if x.abs() < 1e-6 {
                2.0 * fc
            } else {
                (2.0 * std::f32::consts::PI * fc * x).sin() / (std::f32::consts::PI * x)
            };
            let r = x / m;
            let win = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / denom;
            *t = sinc * win;
        }
        let sum: f32 = taps.iter().sum();
        for t in taps.iter_mut() {
            *t /= sum; // unity DC gain
        }
        let sr = sample_rate.max(8000.0) as f32;
        Self {
            taps,
            channels: vec![Channel::new(); channels.max(1)],
            dc_r: (-2.0 * std::f32::consts::PI * 8.0 / sr).exp(),
            lf_a: 1.0 - (-2.0 * std::f32::consts::PI * 120.0 / sr).exp(),
        }
    }

    /// Pure alignment delay used while the master is bypassed, so toggling bypass
    /// never shifts timing against the reported latency.
    #[inline]
    pub fn delay_only(&mut self, ch: usize, x: f32, quality: u32) -> f32 {
        let lat = quality_latency(quality);
        let c = &mut self.channels[ch];
        if lat == 0 {
            return x;
        }
        c.delay[c.delay_pos] = x;
        let read = (c.delay_pos + DELAY_LEN - lat) % DELAY_LEN;
        c.delay_pos = (c.delay_pos + 1) % DELAY_LEN;
        c.delay[read]
    }

    /// Process one sample. At `QUALITY_ADAA` the stage is skipped when `active` is false
    /// (bit-transparent); oversampled qualities always run so the delay stays constant.
    #[inline]
    pub fn process(
        &mut self,
        ch: usize,
        x: f32,
        p: &ShapeParams,
        quality: u32,
        character: u32,
        active: bool,
    ) -> f32 {
        if quality == QUALITY_ADAA && !active {
            return x;
        }
        let lf_a = self.lf_a;
        let dc_r = self.dc_r;
        let taps = &self.taps;
        let c = &mut self.channels[ch];

        // Transformer: low-frequency-weighted drive (core saturates harder on LF).
        let xs = if character == CHAR_TRANSFORMER {
            c.lf += lf_a * (x - c.lf);
            x + 0.8 * c.lf
        } else {
            x
        };

        let y = match quality {
            QUALITY_2X => {
                let [a, b] = c.r1.upsample(xs, taps);
                c.r1.downsample(shape(a, p), shape(b, p), taps)
            }
            QUALITY_4X => {
                let [a, b] = c.r1.upsample(xs, taps);
                let [a0, a1] = c.r2.upsample(a, taps);
                let sa = c.r2.downsample(shape(a0, p), shape(a1, p), taps);
                // Second half of the 2x pair needs its own pass through the 4x stage.
                let [b0, b1] = c.r2.upsample(b, taps);
                let sb = c.r2.downsample(shape(b0, p), shape(b1, p), taps);
                c.r1.downsample(sa, sb, taps)
            }
            _ => {
                // HybridEQ Residual ADAA: preserves high-frequency response with unity magnitude
                // (0.000 dB loss) across the entire 20 Hz .. 20 kHz audio band for small/linear signals.
                let f = shape(xs, p);
                let f_int = shape_antiderivative(xs, p);
                c.adaa.step(xs, f, f_int, p.slope)
            }
        };

        if character == CHAR_CLEAN {
            y
        } else {
            // DC blocker (~8 Hz) removes the offset created by the asymmetric bias.
            let out = y - c.dc_x1 + dc_r * c.dc_y1;
            c.dc_x1 = y;
            c.dc_y1 = out;
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(quality: u32, character: u32, freq: f32, gain: f32) -> Vec<f32> {
        let sr = 48000.0f64;
        let mut s = DriveStage::new(1, sr);
        let p = ShapeParams::new(gain, character);
        (0..12000)
            .map(|n| {
                let x = 0.9 * (2.0 * std::f32::consts::PI * freq * n as f32 / sr as f32).sin();
                s.process(0, x, &p, quality, character, true)
            })
            .collect()
    }

    fn bin_mag(sig: &[f32], freq: f32) -> f32 {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, v) in sig.iter().enumerate() {
            let w = 2.0 * std::f32::consts::PI * freq * n as f32 / 48000.0;
            re += v * w.cos();
            im += v * w.sin();
        }
        (re * re + im * im).sqrt() / sig.len() as f32
    }

    #[test]
    fn latency_values_are_integers() {
        assert_eq!(quality_latency(QUALITY_ADAA), 0);
        assert_eq!(quality_latency(QUALITY_2X), 20);
        assert_eq!(quality_latency(QUALITY_4X), 30);
    }

    #[test]
    fn oversampling_reduces_aliasing() {
        // 9.1 kHz: 3rd harmonic 27.3k, 5th 45.5k (aliases to 2.5k), 7th 63.7k (aliases to 15.7k)
        let g = 10.0;
        let alias = 2500.0 / 1.0; // 48000 - 45500
        let a = bin_mag(&run(QUALITY_ADAA, CHAR_CLEAN, 9100.0, g)[4800..], alias);
        let x4 = bin_mag(&run(QUALITY_4X, CHAR_CLEAN, 9100.0, g)[4800..], alias);
        assert!(x4 < a, "4x ({x4}) should alias less than ADAA ({a})");
    }

    #[test]
    fn clean_is_symmetric_and_biased_modes_have_no_dc() {
        for q in [QUALITY_ADAA, QUALITY_2X, QUALITY_4X] {
            let tb = run(q, CHAR_TUBE, 1000.0, 5.0);
            let mean: f32 = tb[6000..].iter().sum::<f32>() / 6000.0;
            assert!(mean.abs() < 0.01, "tube DC {mean}");
            let h2 = bin_mag(&tb[4800..], 2000.0);
            let clean = run(q, CHAR_CLEAN, 1000.0, 5.0);
            let h2c = bin_mag(&clean[4800..], 2000.0);
            assert!(h2 > 10.0 * h2c.max(1e-6), "tube should add 2nd harmonic");
        }
    }

    #[test]
    fn oversampled_delay_matches_reported_latency() {
        for q in [QUALITY_2X, QUALITY_4X] {
            let mut s = DriveStage::new(1, 48000.0);
            let p = ShapeParams::new(1.0, CHAR_CLEAN); // near-linear for tiny signal
            let mut out = vec![];
            for n in 0..400 {
                let x = if n == 0 { 0.01 } else { 0.0 };
                out.push(s.process(0, x, &p, q, CHAR_CLEAN, true));
            }
            let peak = out
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
                .unwrap()
                .0;
            assert_eq!(peak, quality_latency(q), "quality {q}");
        }
    }

    #[test]
    fn adaa_residual_preserves_high_frequencies_flat_to_nyquist() {
        // Just like HybridEQ tests/adaa.cpp test 2:
        // A linear/small-signal stage must keep its level with < 0.01 dB loss from 100 Hz to 20 kHz.
        let sr = 48000.0f32;
        let p = ShapeParams::new(1.0, CHAR_CLEAN);
        for f in [100.0, 1000.0, 5000.0, 10000.0, 15000.0, 20000.0] {
            let mut adaa = ResidualAdaa::new();
            let w = 2.0 * std::f32::consts::PI * f / sr;
            let mut num = 0.0f32;
            let mut den = 0.0f32;
            for i in 0..4800 {
                let x = 0.05 * (w * i as f32).sin();
                let fn_val = shape(x, &p);
                let fn_int = shape_antiderivative(x, &p);
                let y = adaa.step(x, fn_val, fn_int, p.slope);
                if i > 200 {
                    num += y * y;
                    den += (p.slope * x) * (p.slope * x);
                }
            }
            let diff_db = (10.0 * (num / den).log10()).abs();
            assert!(
                diff_db < 0.01,
                "At frequency {} Hz, level change was {} dB (should be < 0.01 dB)",
                f, diff_db
            );
        }
    }
}
