//! Portable RTAS/TDM contract.
//!
//! Q1.23 samples and a 64-frame stereo schedule with 32 slots and a fixed
//! 16-block (1024-sample) delay. This crate does not link the Avid SDK, does
//! not build a `.dpm`, and is not a macOS 10.6 or i386 binary. Rust 1.80
//! cannot target that machine. The modern server does not depend on this crate.

pub mod packets;
pub mod timeslots;
pub mod transport;
/// PCM is shared by the portable transport and the isolated firmware harness.
pub mod pcm {
    pub use super::Pcm24;
}
/// Maximum words accepted by the standalone DSP harness, not the fixed card schedule.
pub const MAX_WORDS: usize = 1024;

pub const API_VERSION: u32 = 1;
pub const FRAME_COUNT: usize = 64;
pub const SLOT_COUNT: usize = 32;
pub const SCHEDULE_BLOCKS: usize = 16;
pub const SCHEDULE_SAMPLES: usize = FRAME_COUNT * SCHEDULE_BLOCKS;
const FULL_SCALE: f64 = 8_388_608.0;

/// Right-justified signed 24-bit Q1.23 sample. Range is -2^23..=2^23-1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pcm24(i32);

impl Pcm24 {
    pub const ZERO: Self = Self(0);

    pub fn from_word(word: u32) -> Self {
        Self(((word << 8) as i32) >> 8)
    }

    pub fn from_i32(value: i32) -> Result<Self, &'static str> {
        if (-8_388_608..=8_388_607).contains(&value) {
            Ok(Self(value))
        } else {
            Err("sample outside signed 24-bit range")
        }
    }

    /// Clip to Q1.23. Round to nearest, ties away from zero. Reject NaN and Inf.
    pub fn from_f64(value: f64) -> Result<Self, &'static str> {
        if !value.is_finite() {
            return Err("non-finite audio sample");
        }
        let scaled = (value.clamp(-1.0, 1.0) * FULL_SCALE).round();
        let clipped = scaled.clamp(-FULL_SCALE, FULL_SCALE - 1.0);
        Ok(Self(clipped as i32))
    }

    /// The legacy bridge's explicitly different policy: a nonfinite float is silence.
    /// Strict conversion above still rejects invalid samples for engineering tests.
    pub fn from_bridge_f64(value: f64) -> Self {
        Self::from_f64(value).unwrap_or(Self::ZERO)
    }

    pub fn signed(self) -> i32 {
        self.0
    }

    pub fn word(self) -> u32 {
        self.0 as u32 & 0x00ff_ffff
    }

    pub fn to_f64(self) -> f64 {
        self.0 as f64 / FULL_SCALE
    }

    pub fn to_be_bytes(self) -> [u8; 3] {
        let word = self.word();
        [(word >> 16) as u8, (word >> 8) as u8, word as u8]
    }

    pub fn from_be_bytes(bytes: [u8; 3]) -> Self {
        Self::from_word(u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]))
    }
}

pub type StereoFrame = [Pcm24; 2];
pub type StereoBlock = [StereoFrame; FRAME_COUNT];

/// 32-slot capture. Playback is the block captured 16 blocks earlier.
#[derive(Clone, Debug)]
pub struct TdmSchedule {
    slots: [StereoBlock; SLOT_COUNT],
    block: u64,
}

impl Default for TdmSchedule {
    fn default() -> Self {
        Self::new()
    }
}

impl TdmSchedule {
    pub fn new() -> Self {
        Self {
            slots: [[[Pcm24::ZERO; 2]; FRAME_COUNT]; SLOT_COUNT],
            block: 0,
        }
    }

    pub fn blocks_captured(&self) -> u64 {
        self.block
    }

    /// Store `input` and return the block that is due. The first 16 calls
    /// return silence. Call 16 returns call 0.
    pub fn push(&mut self, input: &StereoBlock) -> StereoBlock {
        let slot = (self.block as usize) % SLOT_COUNT;
        self.slots[slot] = *input;
        let output = if self.block < SCHEDULE_BLOCKS as u64 {
            [[Pcm24::ZERO; 2]; FRAME_COUNT]
        } else {
            let due = ((self.block - SCHEDULE_BLOCKS as u64) as usize) % SLOT_COUNT;
            self.slots[due]
        };
        self.block += 1;
        output
    }
}

/// C name kept so a legacy binary can be told apart from the modern server.
#[no_mangle]
pub extern "C" fn ag_legacy_pcm_api_version() -> u32 {
    API_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silence() -> StereoBlock {
        [[Pcm24::ZERO; 2]; FRAME_COUNT]
    }

    #[test]
    fn q123_clips_rounds_and_rejects_nonfinite() {
        assert_eq!(Pcm24::from_f64(0.5).unwrap().signed(), 4_194_304);
        assert!((Pcm24::from_f64(0.5).unwrap().to_f64() - 0.5).abs() < 1.0e-12);
        assert_eq!(Pcm24::from_f64(1.0).unwrap().signed(), 8_388_607);
        assert_eq!(Pcm24::from_f64(-1.0).unwrap().signed(), -8_388_608);
        let half = 0.5 / FULL_SCALE;
        assert_eq!(Pcm24::from_f64(half).unwrap().signed(), 1);
        assert_eq!(Pcm24::from_f64(-half).unwrap().signed(), -1);
        assert!(Pcm24::from_f64(f64::NAN).is_err());
        assert!(Pcm24::from_f64(f64::INFINITY).is_err());
        assert_eq!(
            Pcm24::from_i32(8_388_608).unwrap_err(),
            "sample outside signed 24-bit range"
        );

        let negative = Pcm24::from_i32(-1).unwrap();
        assert_eq!(negative.word(), 0x00ff_ffff);
        assert_eq!(Pcm24::from_be_bytes(negative.to_be_bytes()), negative);
        assert_eq!(ag_legacy_pcm_api_version(), API_VERSION);
        assert_eq!(SCHEDULE_SAMPLES, 1024);
    }

    #[test]
    fn schedule_returns_audio_sixteen_blocks_later() {
        let mut schedule = TdmSchedule::new();
        let mut impulse = silence();
        impulse[0][0] = Pcm24::from_f64(0.5).unwrap();
        assert!(schedule
            .push(&impulse)
            .iter()
            .all(|frame| *frame == [Pcm24::ZERO; 2]));
        for _ in 1..SCHEDULE_BLOCKS {
            let quiet = schedule.push(&silence());
            assert!(quiet.iter().all(|frame| *frame == [Pcm24::ZERO; 2]));
        }
        let back = schedule.push(&silence());
        assert_eq!(back[0][0], impulse[0][0]);
        assert_eq!(back[0][1], Pcm24::ZERO);

        let mut marked = silence();
        marked[3][1] = Pcm24::from_f64(-0.25).unwrap();
        let at = schedule.blocks_captured();
        schedule.push(&marked);
        for _ in 1..SCHEDULE_BLOCKS {
            schedule.push(&silence());
        }
        let delayed = schedule.push(&silence());
        assert_eq!(delayed[3][1], marked[3][1]);
        assert_eq!(schedule.blocks_captured(), at + SCHEDULE_BLOCKS as u64 + 1);
        assert_eq!(SLOT_COUNT, 32);
        assert_eq!(FRAME_COUNT, 64);
    }
}

/// Deterministic fixed-point fixtures: signed edges followed by xorshift noise.
pub fn fixture(words: usize) -> Vec<Pcm24> {
    let edges = [-8388608, 8388607, -1, 0, 1, -3, 3, 4194304];
    let mut seed = 0x56300321u32;
    (0..words)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            Pcm24::from_word(if i < edges.len() {
                edges[i] as u32
            } else {
                seed
            })
        })
        .collect()
}
