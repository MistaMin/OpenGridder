//! AudioGridder protocol v13, as explicit little-endian bytes.
//!
//! The layout matches the OpenGridder port of Andreas Pohl's AudioGridder
//! structs. Audio frames are not a `sizeof` of a JUCE type.

pub mod command;
pub mod realtime;

pub const PROTOCOL_VERSION: i32 = 13;
pub const SERVER_PORT: u16 = 55056;
pub const TRAY_PORT: u16 = 55055;

#[derive(Clone, Debug, PartialEq)]
pub struct HandshakeRequest {
    pub version: i32,
    pub channels_in: i32,
    pub channels_out: i32,
    pub channels_sc: i32,
    pub sample_rate: f64,
    pub samples_per_block: i32,
    pub double_precision: bool,
    pub client_id: u64,
    pub flags: u8,
    pub active_channels: u64,
}

impl Default for HandshakeRequest {
    fn default() -> Self {
        Self {
            version: PROTOCOL_VERSION,
            channels_in: 2,
            channels_out: 2,
            channels_sc: 0,
            sample_rate: 48_000.0,
            samples_per_block: 512,
            double_precision: false,
            client_id: 0,
            flags: 0,
            active_channels: u64::MAX,
        }
    }
}

impl HandshakeRequest {
    pub const WIRE_SIZE: usize = 64;

    pub fn to_bytes(&self) -> [u8; Self::WIRE_SIZE] {
        let mut buf = [0u8; Self::WIRE_SIZE];
        buf[0..4].copy_from_slice(&self.version.to_le_bytes());
        buf[4..8].copy_from_slice(&self.channels_in.to_le_bytes());
        buf[8..12].copy_from_slice(&self.channels_out.to_le_bytes());
        buf[12..16].copy_from_slice(&self.channels_sc.to_le_bytes());
        buf[16..24].copy_from_slice(&self.sample_rate.to_le_bytes());
        buf[24..28].copy_from_slice(&self.samples_per_block.to_le_bytes());
        buf[28] = u8::from(self.double_precision);
        buf[32..40].copy_from_slice(&self.client_id.to_le_bytes());
        buf[40] = self.flags;
        buf[48..56].copy_from_slice(&self.active_channels.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::WIRE_SIZE {
            return None;
        }
        Some(Self {
            version: i32::from_le_bytes(buf[0..4].try_into().ok()?),
            channels_in: i32::from_le_bytes(buf[4..8].try_into().ok()?),
            channels_out: i32::from_le_bytes(buf[8..12].try_into().ok()?),
            channels_sc: i32::from_le_bytes(buf[12..16].try_into().ok()?),
            sample_rate: f64::from_le_bytes(buf[16..24].try_into().ok()?),
            samples_per_block: i32::from_le_bytes(buf[24..28].try_into().ok()?),
            double_precision: buf[28] != 0,
            client_id: u64::from_le_bytes(buf[32..40].try_into().ok()?),
            flags: buf[40],
            active_channels: u64::from_le_bytes(buf[48..56].try_into().ok()?),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandshakeResponse {
    pub version: i32,
    pub flags: u32,
    pub port: i32,
}

impl HandshakeResponse {
    pub const WIRE_SIZE: usize = 36;

    pub fn to_bytes(self) -> [u8; Self::WIRE_SIZE] {
        let mut buf = [0u8; Self::WIRE_SIZE];
        buf[0..4].copy_from_slice(&self.version.to_le_bytes());
        buf[4..8].copy_from_slice(&self.flags.to_le_bytes());
        buf[8..12].copy_from_slice(&self.port.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::WIRE_SIZE {
            return None;
        }
        Some(Self {
            version: i32::from_le_bytes(buf[0..4].try_into().ok()?),
            flags: u32::from_le_bytes(buf[4..8].try_into().ok()?),
            port: i32::from_le_bytes(buf[8..12].try_into().ok()?),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioRequestHeader {
    pub channels: i32,
    pub samples: i32,
    pub channels_requested: i32,
    pub samples_requested: i32,
    pub midi_events: i32,
    pub is_double: bool,
    pub trace_id: [u8; 16],
}

impl AudioRequestHeader {
    pub const WIRE_SIZE: usize = 40;

    pub fn to_bytes(&self) -> [u8; Self::WIRE_SIZE] {
        let mut buf = [0u8; Self::WIRE_SIZE];
        buf[0..4].copy_from_slice(&self.channels.to_le_bytes());
        buf[4..8].copy_from_slice(&self.samples.to_le_bytes());
        buf[8..12].copy_from_slice(&self.channels_requested.to_le_bytes());
        buf[12..16].copy_from_slice(&self.samples_requested.to_le_bytes());
        buf[16..20].copy_from_slice(&self.midi_events.to_le_bytes());
        buf[20] = u8::from(self.is_double);
        buf[21..37].copy_from_slice(&self.trace_id);
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::WIRE_SIZE {
            return None;
        }
        let mut trace_id = [0u8; 16];
        trace_id.copy_from_slice(&buf[21..37]);
        Some(Self {
            channels: i32::from_le_bytes(buf[0..4].try_into().ok()?),
            samples: i32::from_le_bytes(buf[4..8].try_into().ok()?),
            channels_requested: i32::from_le_bytes(buf[8..12].try_into().ok()?),
            samples_requested: i32::from_le_bytes(buf[12..16].try_into().ok()?),
            midi_events: i32::from_le_bytes(buf[16..20].try_into().ok()?),
            is_double: buf[20] != 0,
            trace_id,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioResponseHeader {
    pub channels: i32,
    pub samples: i32,
    pub midi_events: i32,
    pub latency_samples: i32,
}

impl AudioResponseHeader {
    pub const WIRE_SIZE: usize = 16;

    pub fn to_bytes(self) -> [u8; Self::WIRE_SIZE] {
        let mut buf = [0u8; Self::WIRE_SIZE];
        buf[0..4].copy_from_slice(&self.channels.to_le_bytes());
        buf[4..8].copy_from_slice(&self.samples.to_le_bytes());
        buf[8..12].copy_from_slice(&self.midi_events.to_le_bytes());
        buf[12..16].copy_from_slice(&self.latency_samples.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::WIRE_SIZE {
            return None;
        }
        Some(Self {
            channels: i32::from_le_bytes(buf[0..4].try_into().ok()?),
            samples: i32::from_le_bytes(buf[4..8].try_into().ok()?),
            midi_events: i32::from_le_bytes(buf[8..12].try_into().ok()?),
            latency_samples: i32::from_le_bytes(buf[12..16].try_into().ok()?),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionInfo {
    pub bpm: f64,
    pub time_sig_numerator: i32,
    pub time_sig_denominator: i32,
    pub time_in_samples: i64,
    pub time_in_seconds: f64,
    pub ppq_position: f64,
    pub playing: bool,
    pub recording: bool,
    pub looping: bool,
}

impl Default for PositionInfo {
    fn default() -> Self {
        Self {
            bpm: 120.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            time_in_samples: 0,
            time_in_seconds: 0.0,
            ppq_position: 0.0,
            playing: false,
            recording: false,
            looping: false,
        }
    }
}

impl PositionInfo {
    pub const WIRE_SIZE: usize = 48;

    pub fn to_bytes(self) -> [u8; Self::WIRE_SIZE] {
        let mut buf = [0u8; Self::WIRE_SIZE];
        buf[0..8].copy_from_slice(&self.bpm.to_le_bytes());
        buf[8..12].copy_from_slice(&self.time_sig_numerator.to_le_bytes());
        buf[12..16].copy_from_slice(&self.time_sig_denominator.to_le_bytes());
        buf[16..24].copy_from_slice(&self.time_in_samples.to_le_bytes());
        buf[24..32].copy_from_slice(&self.time_in_seconds.to_le_bytes());
        buf[32..40].copy_from_slice(&self.ppq_position.to_le_bytes());
        buf[40] = u8::from(self.playing);
        buf[41] = u8::from(self.recording);
        buf[42] = u8::from(self.looping);
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::WIRE_SIZE {
            return None;
        }
        Some(Self {
            bpm: f64::from_le_bytes(buf[0..8].try_into().ok()?),
            time_sig_numerator: i32::from_le_bytes(buf[8..12].try_into().ok()?),
            time_sig_denominator: i32::from_le_bytes(buf[12..16].try_into().ok()?),
            time_in_samples: i64::from_le_bytes(buf[16..24].try_into().ok()?),
            time_in_seconds: f64::from_le_bytes(buf[24..32].try_into().ok()?),
            ppq_position: f64::from_le_bytes(buf[32..40].try_into().ok()?),
            playing: buf[40] != 0,
            recording: buf[41] != 0,
            looping: buf[42] != 0,
        })
    }
}

/// Mono or stereo, 1 to 8192 frames. Double precision and MIDI are legal shapes.
/// The f32 helpers still refuse them because they read 4-byte samples.
pub fn audio_shape_ok(channels: i32, samples: i32, midi_events: i32, _is_double: bool) -> bool {
    (channels == 1 || channels == 2) && (1..=8192).contains(&samples) && midi_events >= 0
}

fn f32_audio_shape(channels: i32, samples: i32, midi_events: i32, is_double: bool) -> bool {
    audio_shape_ok(channels, samples, midi_events, is_double) && midi_events == 0 && !is_double
}

pub fn write_audio_request(channels: &[Vec<f32>], position: &PositionInfo) -> Option<Vec<u8>> {
    let count = channels.len() as i32;
    let samples = channels.first().map(|c| c.len()).unwrap_or(0) as i32;
    if !f32_audio_shape(count, samples, 0, false) || channels.iter().any(|c| c.len() != samples as usize) {
        return None;
    }
    let header = AudioRequestHeader {
        channels: count,
        samples,
        channels_requested: count,
        samples_requested: samples,
        midi_events: 0,
        is_double: false,
        trace_id: [0; 16],
    };
    let mut out = Vec::with_capacity(AudioRequestHeader::WIRE_SIZE + channels.len() * samples as usize * 4 + PositionInfo::WIRE_SIZE);
    out.extend_from_slice(&header.to_bytes());
    for channel in channels {
        for sample in channel {
            out.extend_from_slice(&sample.to_le_bytes());
        }
    }
    out.extend_from_slice(&position.to_bytes());
    Some(out)
}

pub fn read_audio_request(bytes: &[u8]) -> Option<(AudioRequestHeader, Vec<Vec<f32>>, PositionInfo)> {
    let header = AudioRequestHeader::from_bytes(bytes)?;
    if !f32_audio_shape(header.channels, header.samples, header.midi_events, header.is_double) {
        return None;
    }
    let channels = header.channels as usize;
    let samples = header.samples as usize;
    let audio_bytes = channels * samples * 4;
    let audio_at = AudioRequestHeader::WIRE_SIZE;
    let pos_at = audio_at + audio_bytes;
    if bytes.len() < pos_at + PositionInfo::WIRE_SIZE {
        return None;
    }
    let mut audio = vec![vec![0.0f32; samples]; channels];
    for (ch, channel) in audio.iter_mut().enumerate() {
        for (i, sample) in channel.iter_mut().enumerate() {
            let at = audio_at + (ch * samples + i) * 4;
            *sample = f32::from_le_bytes(bytes[at..at + 4].try_into().ok()?);
        }
    }
    let position = PositionInfo::from_bytes(&bytes[pos_at..])?;
    Some((header, audio, position))
}

pub fn write_audio_response(channels: &[Vec<f32>], latency_samples: i32) -> Option<Vec<u8>> {
    let count = channels.len() as i32;
    let samples = channels.first().map(|c| c.len()).unwrap_or(0) as i32;
    if !f32_audio_shape(count, samples, 0, false) || channels.iter().any(|c| c.len() != samples as usize) {
        return None;
    }
    let header = AudioResponseHeader {
        channels: count,
        samples,
        midi_events: 0,
        latency_samples,
    };
    let mut out = Vec::with_capacity(AudioResponseHeader::WIRE_SIZE + channels.len() * samples as usize * 4);
    out.extend_from_slice(&header.to_bytes());
    for channel in channels {
        for sample in channel {
            out.extend_from_slice(&sample.to_le_bytes());
        }
    }
    Some(out)
}

pub fn read_audio_response(bytes: &[u8]) -> Option<(AudioResponseHeader, Vec<Vec<f32>>)> {
    let header = AudioResponseHeader::from_bytes(bytes)?;
    // The MIDI body follows the samples. Callers that only need the audio
    // can stop here. `numMidiEvents` is allowed to be nonzero.
    if !audio_shape_ok(header.channels, header.samples, header.midi_events, false) {
        return None;
    }
    let channels = header.channels as usize;
    let samples = header.samples as usize;
    let audio_at = AudioResponseHeader::WIRE_SIZE;
    if bytes.len() < audio_at + channels * samples * 4 {
        return None;
    }
    let mut audio = vec![vec![0.0f32; samples]; channels];
    for (ch, channel) in audio.iter_mut().enumerate() {
        for (i, sample) in channel.iter_mut().enumerate() {
            let at = audio_at + (ch * samples + i) * 4;
            *sample = f32::from_le_bytes(bytes[at..at + 4].try_into().ok()?);
        }
    }
    Some((header, audio))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_keeps_the_v13_offsets() {
        let req = HandshakeRequest {
            client_id: 0x4F47_5253_5431_3231,
            flags: 1,
            channels_sc: 2,
            active_channels: 0x03,
            ..HandshakeRequest::default()
        };
        let bytes = req.to_bytes();
        assert_eq!(bytes.len(), 64);
        assert_eq!(&bytes[0..4], &13i32.to_le_bytes());
        assert_eq!(bytes[28], 0);
        assert_eq!(&bytes[32..40], &req.client_id.to_le_bytes());
        assert_eq!(bytes[40], 1);
        assert_eq!(HandshakeRequest::from_bytes(&bytes), Some(req));

        let resp = HandshakeResponse { version: 13, flags: 2, port: 55056 };
        let raw = resp.to_bytes();
        assert_eq!(raw.len(), 36);
        assert_eq!(HandshakeResponse::from_bytes(&raw), Some(resp));
    }

    #[test]
    fn protocol_codecs_and_wire_offsets_roundtrip() {
        // P49 & P51: HandshakeRequest unused1 (byte 41..48) and unused2 (byte 56..64) stay zero
        let req = HandshakeRequest {
            version: PROTOCOL_VERSION,
            channels_in: 2,
            channels_out: 2,
            channels_sc: 0,
            sample_rate: 48000.0,
            samples_per_block: 512,
            double_precision: false,
            client_id: 0x1234_5678_9ABC_DEF0,
            flags: 0x01,
            active_channels: 0x03,
        };
        let req_bytes = req.to_bytes();
        assert_eq!(req_bytes.len(), 64);
        // unused1 at byte 41..48
        assert_eq!(&req_bytes[41..48], &[0u8; 7], "P49: unused1 at byte 41..48 must stay zero");
        // unused2 at byte 56..64
        assert_eq!(&req_bytes[56..64], &[0u8; 8], "P51: unused2 at byte 56..64 must stay zero");

        // P53, P55, P57: HandshakeResponse wire structure
        // flags at byte 4, port at byte 8, 6 unused u32s at bytes 12..36
        let resp_default = HandshakeResponse {
            version: PROTOCOL_VERSION,
            flags: 0, // P53: default server writes 0
            port: 55056,
        };
        let resp_bytes = resp_default.to_bytes();
        assert_eq!(resp_bytes.len(), 36, "P57: response wire size is 36 bytes");
        assert_eq!(&resp_bytes[0..4], &PROTOCOL_VERSION.to_le_bytes(), "version at byte 0");
        assert_eq!(&resp_bytes[4..8], &0u32.to_le_bytes(), "P53: flags at byte 4 is 0");
        assert_eq!(&resp_bytes[8..12], &55056i32.to_le_bytes(), "port at byte 8");
        // P55: RESP_LOCAL_MODE (bit 2) is not set
        assert_eq!(resp_default.flags & command::RESP_LOCAL_MODE, 0, "P55: RESP_LOCAL_MODE is not set");
        // P57: Six unused u32s (24 bytes from byte 12 to 36) stay zero on wire
        assert_eq!(&resp_bytes[12..36], &[0u8; 24], "P57: six unused response u32s stay zero");

        // P64: AudioRequestHeader trace_id is 16 bytes. Default write writes zero id
        let pos = PositionInfo::default();
        let audio_req_bytes = write_audio_request(&[vec![0.0; 16], vec![0.0; 16]], &pos).unwrap();
        let (audio_header, _, _) = read_audio_request(&audio_req_bytes).unwrap();
        assert_eq!(audio_header.trace_id, [0u8; 16], "P64: default written traceId is zero");
        assert_eq!(&audio_req_bytes[21..37], &[0u8; 16], "P64: traceId byte offset 21..37 is 16 zero bytes");

        // P16 & P19: UPDATE_SCREEN_CAPTURE_AREA = 65 and CLIPBOARD = 68 command message codes
        assert_eq!(command::UPDATE_SCREEN_CAPTURE_AREA, 65, "P16: UpdateScreenCaptureArea = 65");
        assert_eq!(command::CLIPBOARD, 68, "P19: Clipboard = 68");
        // Verify roundtrip encoding/decoding of UPDATE_SCREEN_CAPTURE_AREA payload
        let nudge_msg = command::encode(command::UPDATE_SCREEN_CAPTURE_AREA, &command::number(30));
        let nudge_ty = i32::from_le_bytes(nudge_msg[0..4].try_into().unwrap());
        let nudge_len = i32::from_le_bytes(nudge_msg[4..8].try_into().unwrap()) as usize;
        assert_eq!(nudge_ty, 65);
        assert_eq!(command::read_number(&nudge_msg[8..8 + nudge_len]), Some(30));
        // Verify roundtrip encoding/decoding of CLIPBOARD payload
        let clip_text = "test_clipboard_data";
        let clip_msg = command::encode(command::CLIPBOARD, &command::length_prefixed(clip_text.as_bytes()));
        let clip_ty = i32::from_le_bytes(clip_msg[0..4].try_into().unwrap());
        let clip_len = i32::from_le_bytes(clip_msg[4..8].try_into().unwrap()) as usize;
        assert_eq!(clip_ty, 68);
        assert_eq!(command::read_length_prefixed(&clip_msg[8..8 + clip_len]), Some(clip_text.as_bytes()));
    }

    #[test]
    fn audio_block_roundtrip_is_planar_f32() {
        let channels = vec![vec![0.25f32, -0.5], vec![0.0, 1.0]];
        let mut position = PositionInfo::default();
        position.playing = true;
        position.recording = true;
        position.looping = true;
        position.bpm = 140.0;
        position.time_sig_numerator = 3;
        position.time_sig_denominator = 8;
        position.time_in_samples = 48000;
        position.time_in_seconds = 1.0;
        position.ppq_position = 2.5;

        let bytes = write_audio_request(&channels, &position).unwrap();
        let (header, audio, pos) = read_audio_request(&bytes).unwrap();
        assert_eq!(header.channels, 2);
        assert_eq!(header.samples, 2);
        assert!(!header.is_double);
        assert_eq!(audio, channels);
        assert_eq!(pos, position);

        let response = write_audio_response(&channels, 18).unwrap();
        let (res, back) = read_audio_response(&response).unwrap();
        assert_eq!(res.latency_samples, 18);
        assert_eq!(back, channels);
    }

    #[test]
    fn sidechain_and_wide_layouts_are_refused() {
        assert!(audio_shape_ok(2, 64, 0, true));
        assert!(audio_shape_ok(2, 64, 1, false));
        assert!(!audio_shape_ok(8, 64, 0, false));
        assert!(!audio_shape_ok(2, 0, 0, false));
        assert!(!audio_shape_ok(2, 64, -1, false));
    }
}
