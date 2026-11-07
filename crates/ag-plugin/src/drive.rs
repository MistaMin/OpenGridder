//! Console drive. Quality 0 is residual ADAA and is skipped when drive is off.
//! 2x and 4x are linear-phase FIRs. Their group delay is 20 and 30 samples.
//! The insert removes that delay from the wet/dry alignment, so `latency()`
//! stays on the profile.

use openplugin::prelude::{SaturationMode, saturate_sample};

const FIR_LEN: usize = 41;
const DELAY_LEN: usize = 64;

pub fn quality_latency(quality: i32) -> usize {
    match quality {
        1 => (FIR_LEN - 1) / 2,
        2 => (FIR_LEN - 1) / 2 + (FIR_LEN - 1) / 4,
        _ => 0,
    }
}

#[derive(Clone)]
struct ResidualAdaa {
    x1: f32,
    f1: f32,
    r1: f32,
    apx: f32,
    apy: f32,
    primed: bool,
}

impl ResidualAdaa {
    fn new() -> Self {
        Self { x1: 0.0, f1: 0.0, r1: 0.0, apx: 0.0, apy: 0.0, primed: false }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn step(&mut self, x: f32, f: f32, f_int: f32, slope: f32) -> f32 {
        let lin_in = slope * x;
        if !self.primed {
            self.apx = lin_in;
            self.apy = lin_in;
        }
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

    fn push(&mut self, x: f32) {
        self.pos = if self.pos == 0 { FIR_LEN - 1 } else { self.pos - 1 };
        self.buf[self.pos] = x;
        self.buf[self.pos + FIR_LEN] = x;
    }

    fn convolve(&self, taps: &[f32; FIR_LEN]) -> f32 {
        let window = &self.buf[self.pos..self.pos + FIR_LEN];
        let mut acc = 0.0f32;
        for index in 0..FIR_LEN {
            acc += taps[index] * window[index];
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

    fn upsample(&mut self, x: f32, taps: &[f32; FIR_LEN]) -> [f32; 2] {
        self.up.push(x);
        let even = 2.0 * self.up.convolve(taps);
        self.up.push(0.0);
        let odd = 2.0 * self.up.convolve(taps);
        [even, odd]
    }

    fn downsample(&mut self, even: f32, odd: f32, taps: &[f32; FIR_LEN]) -> f32 {
        self.down.push(even);
        let y = self.down.convolve(taps);
        self.down.push(odd);
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

    fn reset(&mut self) {
        *self = Self::new();
    }
}

pub struct DriveStage {
    taps: [f32; FIR_LEN],
    channels: [Channel; 2],
    dc_r: f32,
    lf_a: f32,
}

fn bessel_i0(x: f32) -> f32 {
    let mut sum = 1.0f32;
    let mut term = 1.0f32;
    let quarter = x * x * 0.25;
    for k in 1..30 {
        term *= quarter / (k * k) as f32;
        sum += term;
    }
    sum
}

fn design_taps() -> [f32; FIR_LEN] {
    let cutoff = 0.22f32;
    let beta = 7.0f32;
    let middle = (FIR_LEN - 1) as f32 * 0.5;
    let mut taps = [0.0f32; FIR_LEN];
    let denom = bessel_i0(beta);
    for (index, tap) in taps.iter_mut().enumerate() {
        let x = index as f32 - middle;
        let sinc = if x.abs() < 1e-6 {
            2.0 * cutoff
        } else {
            (2.0 * std::f32::consts::PI * cutoff * x).sin() / (std::f32::consts::PI * x)
        };
        let ratio = x / middle;
        let window = bessel_i0(beta * (1.0 - ratio * ratio).max(0.0).sqrt()) / denom;
        *tap = sinc * window;
    }
    let sum: f32 = taps.iter().sum();
    for tap in &mut taps {
        *tap /= sum;
    }
    taps
}

fn log_cosh(u: f32) -> f32 {
    let magnitude = u.abs();
    magnitude + (-2.0 * magnitude).exp().ln_1p() - std::f32::consts::LN_2
}

fn shaper(x: f32, amount: f32, mode: SaturationMode) -> (f32, f32, f32) {
    let k = amount.max(1.0e-4);
    match mode {
        SaturationMode::Tube => {
            let f = saturate_sample(x, k, mode);
            let antiderivative = if x >= 0.0 {
                x - (1.0 + k * x).ln() / k
            } else {
                let a = 0.6f32;
                -(1.0 - a * k * x).abs().ln() / (a * a * k) - x / a
            };
            (f, antiderivative, k)
        }
        _ => {
            let f = saturate_sample(x, k, mode);
            (f, log_cosh(k * x) / k, k)
        }
    }
}

impl DriveStage {
    pub fn new() -> Self {
        Self::with_sample_rate(48_000.0)
    }

    pub fn with_sample_rate(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        Self {
            taps: design_taps(),
            channels: [Channel::new(), Channel::new()],
            dc_r: (-2.0 * std::f32::consts::PI * 8.0 / sr).exp(),
            lf_a: 1.0 - (-2.0 * std::f32::consts::PI * 120.0 / sr).exp(),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sample_rate.max(8000.0);
        self.dc_r = (-2.0 * std::f32::consts::PI * 8.0 / sr).exp();
        self.lf_a = 1.0 - (-2.0 * std::f32::consts::PI * 120.0 / sr).exp();
    }

    pub fn reset(&mut self) {
        for channel in &mut self.channels {
            channel.reset();
        }
    }

    /// Integer delay matching `quality_latency`. Bypass uses this instead of the
    /// FIR so the alignment buffer does not rewind when the stage turns off.
    pub fn delay_only(&mut self, channel: usize, x: f32, quality: i32) -> f32 {
        let latency = quality_latency(quality);
        if latency == 0 {
            return x;
        }
        let channel = &mut self.channels[channel.min(1)];
        channel.delay[channel.delay_pos] = x;
        let read = (channel.delay_pos + DELAY_LEN - latency) % DELAY_LEN;
        channel.delay_pos = (channel.delay_pos + 1) % DELAY_LEN;
        channel.delay[read]
    }

    pub fn process_sample(&mut self, channel: usize, x: f32, drive: f32, quality: i32, character: i32) -> f32 {
        let active = drive > 1.0e-4;
        if quality <= 0 && !active {
            return x;
        }
        let amount = 1.0 + drive.max(0.0) * 0.9;
        let ch = channel.min(1);
        let mode = match character {
            1 => SaturationMode::Tube,
            _ => SaturationMode::Tape,
        };

        let lf_a = self.lf_a;
        let dc_r = self.dc_r;
        let c = &mut self.channels[ch];

        // Transformer (character 2): low-frequency-weighted drive (120 Hz pre-emphasis)
        let xs = if character == 2 {
            c.lf += lf_a * (x - c.lf);
            x + 0.8 * c.lf
        } else {
            x
        };

        let y = if quality >= 2 {
            let taps = &self.taps;
            let [a, b] = c.r1.upsample(xs, taps);
            let [a0, a1] = c.r2.upsample(a, taps);
            let shaped_a = c.r2.downsample(saturate_sample(a0, amount, mode), saturate_sample(a1, amount, mode), taps);
            let [b0, b1] = c.r2.upsample(b, taps);
            let shaped_b = c.r2.downsample(saturate_sample(b0, amount, mode), saturate_sample(b1, amount, mode), taps);
            c.r1.downsample(shaped_a, shaped_b, taps)
        } else if quality == 1 {
            let taps = &self.taps;
            let [a, b] = c.r1.upsample(xs, taps);
            c.r1.downsample(saturate_sample(a, amount, mode), saturate_sample(b, amount, mode), taps)
        } else {
            let (f, f_int, slope) = shaper(xs, amount, mode);
            c.adaa.step(xs, f, f_int, slope)
        };

        if character == 0 {
            y
        } else {
            // DC blocker (~8 Hz) removes offset created by asymmetric bias / transformer LF saturation
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

    #[test]
    fn impulse_peak_matches_quality_latency() {
        let mut stage = DriveStage::new();
        for quality in [1, 2] {
            stage.reset();
            let mut output = Vec::with_capacity(400);
            for n in 0..400 {
                let x = if n == 0 { 0.2 } else { 0.0 };
                output.push(stage.process_sample(0, x, 4.0, quality, 0));
            }
            let peak = output
                .iter()
                .enumerate()
                .max_by(|left, right| left.1.abs().partial_cmp(&right.1.abs()).unwrap())
                .unwrap()
                .0;
            assert_eq!(peak, quality_latency(quality), "quality {quality}");
        }
    }
}
