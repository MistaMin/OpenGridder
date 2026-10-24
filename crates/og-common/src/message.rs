//! Port of `Common/Source/Message.hpp` and `Common/Source/Message.cpp` from `marcosfork-1.2.1` (commit `559863f`).
//! Implements the exact AudioGridder protocol v13 wire framing, binary Handshake, AudioMessage,
//! and all command message types including `SetPluginOversampling` (Type 75).

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::key_and_mouse::MouseEvType;

pub const AG_PROTOCOL_VERSION: i32 = 13;
pub const MAX_MESSAGE_SIZE: usize = 1024 * 1024 * 60; // 60 MB

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    None,
    Data,
    Timeout,
    State,
    Syscall,
    Size,
}

#[derive(Debug, Clone)]
pub struct MessageError {
    pub code: ErrorCode,
    pub text: String,
}

impl MessageError {
    pub fn new(code: ErrorCode, text: impl Into<String>) -> Self {
        Self {
            code,
            text: text.into(),
        }
    }
}

impl std::fmt::Display for MessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({:?})", self.text, self.code)
    }
}

pub const HANDSHAKE_FLAG_NO_PLUGINLIST_FILTER: u8 = 1;
pub const HANDSHAKE_FLAGS_LOCAL_MODE: u8 = 2;
/// OpenGridder extension, negotiated so legacy peers keep their original framing.
pub const HANDSHAKE_FLAG_STATE_ACK: u8 = 4;
pub const HANDSHAKE_RESP_FLAG_STATE_ACK: u32 = 1 << 8;
pub const HANDSHAKE_RESP_FLAG_SANDBOX_ENABLED: u32 = 1;
pub const HANDSHAKE_FLAGS_SANDBOX_ENABLED: u32 = HANDSHAKE_RESP_FLAG_SANDBOX_ENABLED;
pub const HANDSHAKE_RESP_FLAG_LOCAL_MODE: u32 = 2;

/// Binary & JSON representation of `e47::HandshakeRequest` (56 bytes on the wire).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
            version: AG_PROTOCOL_VERSION,
            channels_in: 2,
            channels_out: 2,
            channels_sc: 0,
            sample_rate: 48000.0,
            samples_per_block: 512,
            double_precision: false,
            client_id: 0,
            flags: 0,
            active_channels: 0,
        }
    }
}

impl HandshakeRequest {
    pub const WIRE_SIZE: usize = 56;
    pub const SIZE: usize = Self::WIRE_SIZE;

    pub fn set_flag(&mut self, flag: u8) {
        self.flags |= flag;
    }

    pub fn is_flag(&self, flag: u8) -> bool {
        (self.flags & flag) == flag
    }

    pub fn to_bytes(&self) -> [u8; Self::WIRE_SIZE] {
        let mut buf = [0u8; Self::WIRE_SIZE];
        buf[0..4].copy_from_slice(&self.version.to_le_bytes());
        buf[4..8].copy_from_slice(&self.channels_in.to_le_bytes());
        buf[8..12].copy_from_slice(&self.channels_out.to_le_bytes());
        buf[12..16].copy_from_slice(&self.channels_sc.to_le_bytes());
        buf[16..24].copy_from_slice(&self.sample_rate.to_le_bytes());
        buf[24..28].copy_from_slice(&self.samples_per_block.to_le_bytes());
        buf[28] = u8::from(self.double_precision);
        // 3 bytes padding before u64 clientId
        buf[32..40].copy_from_slice(&self.client_id.to_le_bytes());
        buf[40] = self.flags;
        // unused1 + 6 bytes padding before u64 activeChannels
        buf[48..56].copy_from_slice(&self.active_channels.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self, MessageError> {
        if buf.len() < Self::WIRE_SIZE {
            return Err(MessageError::new(
                ErrorCode::Size,
                "HandshakeRequest too short",
            ));
        }
        Ok(Self {
            version: i32::from_le_bytes(buf[0..4].try_into().unwrap()),
            channels_in: i32::from_le_bytes(buf[4..8].try_into().unwrap()),
            channels_out: i32::from_le_bytes(buf[8..12].try_into().unwrap()),
            channels_sc: i32::from_le_bytes(buf[12..16].try_into().unwrap()),
            sample_rate: f64::from_le_bytes(buf[16..24].try_into().unwrap()),
            samples_per_block: i32::from_le_bytes(buf[24..28].try_into().unwrap()),
            double_precision: buf[28] != 0,
            client_id: u64::from_le_bytes(buf[32..40].try_into().unwrap()),
            flags: buf[40],
            active_channels: u64::from_le_bytes(buf[48..56].try_into().unwrap()),
        })
    }

    pub fn to_json(&self) -> Value {
        json!({
            "version": self.version,
            "channelsIn": self.channels_in,
            "channelsOut": self.channels_out,
            "channelsSC": self.channels_sc,
            "rate": self.sample_rate,
            "samplesPerBlock": self.samples_per_block,
            "doublePrecision": self.double_precision,
            "clientId": self.client_id,
            "flags": self.flags,
            "activeChannels": self.active_channels,
        })
    }

    pub fn from_json(j: &Value) -> Self {
        Self {
            version: j
                .get("version")
                .and_then(|v| v.as_i64())
                .unwrap_or(AG_PROTOCOL_VERSION as i64) as i32,
            channels_in: j.get("channelsIn").and_then(|v| v.as_i64()).unwrap_or(2) as i32,
            channels_out: j.get("channelsOut").and_then(|v| v.as_i64()).unwrap_or(2) as i32,
            channels_sc: j.get("channelsSC").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            sample_rate: j.get("rate").and_then(|v| v.as_f64()).unwrap_or(48000.0),
            samples_per_block: j
                .get("samplesPerBlock")
                .and_then(|v| v.as_i64())
                .unwrap_or(512) as i32,
            double_precision: j
                .get("doublePrecision")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            client_id: j.get("clientId").and_then(|v| v.as_u64()).unwrap_or(0),
            flags: j.get("flags").and_then(|v| v.as_u64()).unwrap_or(0) as u8,
            active_channels: j
                .get("activeChannels")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        }
    }
}

/// Binary representation of `e47::HandshakeResponse` (36 bytes on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HandshakeResponse {
    pub version: i32,
    pub flags: u32,
    pub port: i32,
    pub unused: [u32; 6],
}

impl HandshakeResponse {
    pub const WIRE_SIZE: usize = 36;
    pub const SIZE: usize = Self::WIRE_SIZE;

    pub fn set_flag(&mut self, flag: u32) {
        self.flags |= flag;
    }

    pub fn is_flag(&self, flag: u32) -> bool {
        (self.flags & flag) == flag
    }

    pub fn to_bytes(&self) -> [u8; Self::WIRE_SIZE] {
        let mut buf = [0u8; Self::WIRE_SIZE];
        buf[0..4].copy_from_slice(&self.version.to_le_bytes());
        buf[4..8].copy_from_slice(&self.flags.to_le_bytes());
        buf[8..12].copy_from_slice(&self.port.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self, MessageError> {
        if buf.len() < Self::WIRE_SIZE {
            return Err(MessageError::new(
                ErrorCode::Size,
                "HandshakeResponse too short",
            ));
        }
        Ok(Self {
            version: i32::from_le_bytes(buf[0..4].try_into().unwrap()),
            flags: u32::from_le_bytes(buf[4..8].try_into().unwrap()),
            port: i32::from_le_bytes(buf[8..12].try_into().unwrap()),
            unused: [0; 6],
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MidiEvent {
    pub sample_number: i32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionInfo {
    pub bpm: f64,
    pub time_sig_numerator: i32,
    pub time_sig_denominator: i32,
    pub time_in_samples: i64,
    pub time_in_seconds: f64,
    pub ppq_position: f64,
    pub is_playing: bool,
    pub is_recording: bool,
    pub is_looping: bool,
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
            is_playing: false,
            is_recording: false,
            is_looping: false,
        }
    }
}

impl PositionInfo {
    pub const WIRE_SIZE: usize = 48;

    pub fn to_bytes(&self) -> [u8; Self::WIRE_SIZE] {
        let mut buf = [0u8; Self::WIRE_SIZE];
        buf[0..8].copy_from_slice(&self.bpm.to_le_bytes());
        buf[8..12].copy_from_slice(&self.time_sig_numerator.to_le_bytes());
        buf[12..16].copy_from_slice(&self.time_sig_denominator.to_le_bytes());
        buf[16..24].copy_from_slice(&self.time_in_samples.to_le_bytes());
        buf[24..32].copy_from_slice(&self.time_in_seconds.to_le_bytes());
        buf[32..40].copy_from_slice(&self.ppq_position.to_le_bytes());
        buf[40] = u8::from(self.is_playing);
        buf[41] = u8::from(self.is_recording);
        buf[42] = u8::from(self.is_looping);
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self, MessageError> {
        if buf.len() < Self::WIRE_SIZE {
            return Err(MessageError::new(ErrorCode::Size, "PositionInfo too short"));
        }
        Ok(Self {
            bpm: f64::from_le_bytes(buf[0..8].try_into().unwrap()),
            time_sig_numerator: i32::from_le_bytes(buf[8..12].try_into().unwrap()),
            time_sig_denominator: i32::from_le_bytes(buf[12..16].try_into().unwrap()),
            time_in_samples: i64::from_le_bytes(buf[16..24].try_into().unwrap()),
            time_in_seconds: f64::from_le_bytes(buf[24..32].try_into().unwrap()),
            ppq_position: f64::from_le_bytes(buf[32..40].try_into().unwrap()),
            is_playing: buf[40] != 0,
            is_recording: buf[41] != 0,
            is_looping: buf[42] != 0,
        })
    }
}

/// Wire representation of `AudioMessage::RequestHeader` (40 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AudioRequestHeader {
    pub channels: i32,
    pub samples: i32,
    pub channels_requested: i32,
    pub samples_requested: i32,
    pub num_midi_events: i32,
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
        buf[16..20].copy_from_slice(&self.num_midi_events.to_le_bytes());
        buf[20] = u8::from(self.is_double);
        buf[21..37].copy_from_slice(&self.trace_id);
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self, MessageError> {
        if buf.len() < Self::WIRE_SIZE {
            return Err(MessageError::new(
                ErrorCode::Size,
                "AudioRequestHeader too short",
            ));
        }
        let mut trace_id = [0u8; 16];
        trace_id.copy_from_slice(&buf[21..37]);
        Ok(Self {
            channels: i32::from_le_bytes(buf[0..4].try_into().unwrap()),
            samples: i32::from_le_bytes(buf[4..8].try_into().unwrap()),
            channels_requested: i32::from_le_bytes(buf[8..12].try_into().unwrap()),
            samples_requested: i32::from_le_bytes(buf[12..16].try_into().unwrap()),
            num_midi_events: i32::from_le_bytes(buf[16..20].try_into().unwrap()),
            is_double: buf[20] != 0,
            trace_id,
        })
    }
}

/// Wire representation of `AudioMessage::ResponseHeader` (16 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AudioResponseHeader {
    pub channels: i32,
    pub samples: i32,
    pub num_midi_events: i32,
    pub latency_samples: i32,
}

impl AudioResponseHeader {
    pub const WIRE_SIZE: usize = 16;

    pub fn to_bytes(&self) -> [u8; Self::WIRE_SIZE] {
        let mut buf = [0u8; Self::WIRE_SIZE];
        buf[0..4].copy_from_slice(&self.channels.to_le_bytes());
        buf[4..8].copy_from_slice(&self.samples.to_le_bytes());
        buf[8..12].copy_from_slice(&self.num_midi_events.to_le_bytes());
        buf[12..16].copy_from_slice(&self.latency_samples.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self, MessageError> {
        if buf.len() < Self::WIRE_SIZE {
            return Err(MessageError::new(
                ErrorCode::Size,
                "AudioResponseHeader too short",
            ));
        }
        Ok(Self {
            channels: i32::from_le_bytes(buf[0..4].try_into().unwrap()),
            samples: i32::from_le_bytes(buf[4..8].try_into().unwrap()),
            num_midi_events: i32::from_le_bytes(buf[8..12].try_into().unwrap()),
            latency_samples: i32::from_le_bytes(buf[12..16].try_into().unwrap()),
        })
    }
}

/// Port of `e47::AudioMessage` streaming over a byte stream (TCP or Unix domain socket).
#[derive(Debug, Default)]
pub struct AudioMessage {
    pub req_header: AudioRequestHeader,
    pub res_header: AudioResponseHeader,
    pub pos_info: PositionInfo,
    wire: Vec<u8>,
}

impl AudioMessage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_double(&self) -> bool {
        self.req_header.is_double
    }
    pub fn send_to_server_f32<W: Write>(
        &mut self,
        stream: &mut W,
        audio: &[Vec<f32>],
        midi: &[MidiEvent],
        pos_info: &PositionInfo,
        channels_requested: i32,
        samples_requested: i32,
    ) -> Result<(), MessageError> {
        let channels = audio.len() as i32;
        let samples = audio.first().map(|c| c.len()).unwrap_or(0) as i32;
        validate_audio_shape(audio.len() as i32, samples, midi.len() as i32)?;
        if audio.iter().any(|ch| ch.len() != samples as usize) {
            return Err(MessageError::new(
                ErrorCode::Size,
                "Unequal audio channel lengths",
            ));
        }
        self.wire.clear();
        self.req_header = AudioRequestHeader {
            channels,
            samples,
            channels_requested: if channels_requested > -1 {
                channels_requested
            } else {
                channels
            },
            samples_requested: if samples_requested > -1 {
                samples_requested
            } else {
                samples
            },
            num_midi_events: midi.len() as i32,
            is_double: false,
            trace_id: [0; 16],
        };
        validate_audio_shape(
            self.req_header.channels_requested,
            self.req_header.samples_requested,
            self.req_header.num_midi_events,
        )?;
        (&mut self.wire)
            .write_all(&self.req_header.to_bytes())
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        for chan in audio {
            for &sample in chan {
                (&mut self.wire)
                    .write_all(&sample.to_le_bytes())
                    .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
            }
        }
        write_midi_events(&mut self.wire, midi)?;
        (&mut self.wire)
            .write_all(&pos_info.to_bytes())
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        stream
            .write_all(&self.wire)
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        Ok(())
    }

    pub fn send_to_server_f64<W: Write>(
        &mut self,
        stream: &mut W,
        audio: &[Vec<f64>],
        midi: &[MidiEvent],
        pos_info: &PositionInfo,
        channels_requested: i32,
        samples_requested: i32,
    ) -> Result<(), MessageError> {
        let channels = audio.len() as i32;
        let samples = audio.first().map(|c| c.len()).unwrap_or(0) as i32;
        validate_audio_shape(audio.len() as i32, samples, midi.len() as i32)?;
        if audio.iter().any(|ch| ch.len() != samples as usize) {
            return Err(MessageError::new(
                ErrorCode::Size,
                "Unequal audio channel lengths",
            ));
        }
        self.wire.clear();
        self.req_header = AudioRequestHeader {
            channels,
            samples,
            channels_requested: if channels_requested > -1 {
                channels_requested
            } else {
                channels
            },
            samples_requested: if samples_requested > -1 {
                samples_requested
            } else {
                samples
            },
            num_midi_events: midi.len() as i32,
            is_double: true,
            trace_id: [0; 16],
        };
        validate_audio_shape(
            self.req_header.channels_requested,
            self.req_header.samples_requested,
            self.req_header.num_midi_events,
        )?;
        (&mut self.wire)
            .write_all(&self.req_header.to_bytes())
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        for chan in audio {
            for &sample in chan {
                (&mut self.wire)
                    .write_all(&sample.to_le_bytes())
                    .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
            }
        }
        write_midi_events(&mut self.wire, midi)?;
        (&mut self.wire)
            .write_all(&pos_info.to_bytes())
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        stream
            .write_all(&self.wire)
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        Ok(())
    }

    pub fn read_from_client<R: Read>(
        &mut self,
        stream: &mut R,
        buffer_f: &mut Vec<Vec<f32>>,
        buffer_d: &mut Vec<Vec<f64>>,
        midi: &mut Vec<MidiEvent>,
        pos_info: &mut PositionInfo,
    ) -> Result<(), MessageError> {
        let mut hdr_bytes = [0u8; AudioRequestHeader::WIRE_SIZE];
        stream
            .read_exact(&mut hdr_bytes)
            .map_err(|e| MessageError::new(ErrorCode::Data, format!("request header: {e}")))?;
        self.req_header = AudioRequestHeader::from_bytes(&hdr_bytes)?;

        validate_audio_shape(
            self.req_header.channels,
            self.req_header.samples,
            self.req_header.num_midi_events,
        )?;
        validate_audio_shape(
            self.req_header.channels_requested,
            self.req_header.samples_requested,
            self.req_header.num_midi_events,
        )?;
        let total_ch = self
            .req_header
            .channels
            .max(self.req_header.channels_requested)
            .max(0) as usize;
        let total_smp = self
            .req_header
            .samples
            .max(self.req_header.samples_requested)
            .max(0) as usize;
        let incoming_ch = self.req_header.channels.max(0) as usize;
        let incoming_smp = self.req_header.samples.max(0) as usize;

        if self.req_header.is_double {
            buffer_d.resize_with(total_ch, || vec![0.0; total_smp]);
            for ch in buffer_d.iter_mut() {
                ch.resize(total_smp, 0.0);
                ch.fill(0.0);
            }
            let mut raw = vec![0u8; incoming_smp * 8];
            for ch in 0..incoming_ch {
                stream.read_exact(&mut raw).map_err(|e| {
                    MessageError::new(ErrorCode::Data, format!("audio data f64: {e}"))
                })?;
                for (i, chunk) in raw.chunks_exact(8).enumerate() {
                    buffer_d[ch][i] = f64::from_le_bytes(chunk.try_into().unwrap());
                }
            }
        } else {
            buffer_f.resize_with(total_ch, || vec![0.0; total_smp]);
            for ch in buffer_f.iter_mut() {
                ch.resize(total_smp, 0.0);
                ch.fill(0.0);
            }
            let mut raw = vec![0u8; incoming_smp * 4];
            for ch in 0..incoming_ch {
                stream.read_exact(&mut raw).map_err(|e| {
                    MessageError::new(ErrorCode::Data, format!("audio data f32: {e}"))
                })?;
                for (i, chunk) in raw.chunks_exact(4).enumerate() {
                    buffer_f[ch][i] = f32::from_le_bytes(chunk.try_into().unwrap());
                }
            }
        }

        *midi = read_midi_events(stream, self.req_header.num_midi_events.max(0) as usize)?;
        let mut pos_bytes = [0u8; PositionInfo::WIRE_SIZE];
        stream
            .read_exact(&mut pos_bytes)
            .map_err(|e| MessageError::new(ErrorCode::Data, format!("pos info: {e}")))?;
        *pos_info = PositionInfo::from_bytes(&pos_bytes)?;
        Ok(())
    }

    pub fn send_to_client_f32<W: Write>(
        &mut self,
        stream: &mut W,
        audio: &[Vec<f32>],
        midi: &[MidiEvent],
        latency_samples: i32,
        channels_to_send: usize,
    ) -> Result<(), MessageError> {
        let ch_count = channels_to_send.min(audio.len());
        let samples = audio.first().map(|c| c.len()).unwrap_or(0) as i32;
        validate_audio_shape(audio.len() as i32, samples, midi.len() as i32)?;
        if audio.iter().any(|ch| ch.len() != samples as usize) {
            return Err(MessageError::new(
                ErrorCode::Size,
                "Unequal audio channel lengths",
            ));
        }
        self.wire.clear();
        self.res_header = AudioResponseHeader {
            channels: ch_count as i32,
            samples,
            num_midi_events: midi.len() as i32,
            latency_samples,
        };
        (&mut self.wire)
            .write_all(&self.res_header.to_bytes())
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        for ch in 0..ch_count {
            for &sample in &audio[ch] {
                (&mut self.wire)
                    .write_all(&sample.to_le_bytes())
                    .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
            }
        }
        write_midi_events(&mut self.wire, midi)?;
        stream
            .write_all(&self.wire)
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        Ok(())
    }

    pub fn send_to_client_f64<W: Write>(
        &mut self,
        stream: &mut W,
        audio: &[Vec<f64>],
        midi: &[MidiEvent],
        latency_samples: i32,
        channels_to_send: usize,
    ) -> Result<(), MessageError> {
        let ch_count = channels_to_send.min(audio.len());
        let samples = audio.first().map(|c| c.len()).unwrap_or(0) as i32;
        validate_audio_shape(audio.len() as i32, samples, midi.len() as i32)?;
        if audio.iter().any(|ch| ch.len() != samples as usize) {
            return Err(MessageError::new(
                ErrorCode::Size,
                "Unequal audio channel lengths",
            ));
        }
        self.wire.clear();
        self.res_header = AudioResponseHeader {
            channels: ch_count as i32,
            samples,
            num_midi_events: midi.len() as i32,
            latency_samples,
        };
        (&mut self.wire)
            .write_all(&self.res_header.to_bytes())
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        for ch in 0..ch_count {
            for &sample in &audio[ch] {
                (&mut self.wire)
                    .write_all(&sample.to_le_bytes())
                    .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
            }
        }
        write_midi_events(&mut self.wire, midi)?;
        stream
            .write_all(&self.wire)
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        Ok(())
    }

    pub fn read_from_server_f32<R: Read>(
        &mut self,
        stream: &mut R,
        audio: &mut Vec<Vec<f32>>,
        midi: &mut Vec<MidiEvent>,
    ) -> Result<(), MessageError> {
        let mut hdr_bytes = [0u8; AudioResponseHeader::WIRE_SIZE];
        stream
            .read_exact(&mut hdr_bytes)
            .map_err(|e| MessageError::new(ErrorCode::Data, format!("response header: {e}")))?;
        self.res_header = AudioResponseHeader::from_bytes(&hdr_bytes)?;
        validate_audio_shape(
            self.res_header.channels,
            self.res_header.samples,
            self.res_header.num_midi_events,
        )?;
        let ch_count = self.res_header.channels.max(0) as usize;
        let samples = self.res_header.samples.max(0) as usize;

        audio.resize_with(ch_count, || vec![0.0; samples]);
        let mut raw = vec![0u8; samples * 4];
        for ch in 0..ch_count {
            audio[ch].resize(samples, 0.0);
            stream
                .read_exact(&mut raw)
                .map_err(|e| MessageError::new(ErrorCode::Data, format!("audio data: {e}")))?;
            for (i, chunk) in raw.chunks_exact(4).enumerate() {
                audio[ch][i] = f32::from_le_bytes(chunk.try_into().unwrap());
            }
        }
        *midi = read_midi_events(stream, self.res_header.num_midi_events.max(0) as usize)?;
        Ok(())
    }

    pub fn read_from_server_f64<R: Read>(
        &mut self,
        stream: &mut R,
        audio: &mut Vec<Vec<f64>>,
        midi: &mut Vec<MidiEvent>,
    ) -> Result<(), MessageError> {
        let mut hdr_bytes = [0u8; AudioResponseHeader::WIRE_SIZE];
        stream
            .read_exact(&mut hdr_bytes)
            .map_err(|e| MessageError::new(ErrorCode::Data, format!("response header: {e}")))?;
        self.res_header = AudioResponseHeader::from_bytes(&hdr_bytes)?;
        validate_audio_shape(
            self.res_header.channels,
            self.res_header.samples,
            self.res_header.num_midi_events,
        )?;
        let ch_count = self.res_header.channels.max(0) as usize;
        let samples = self.res_header.samples.max(0) as usize;

        audio.resize_with(ch_count, || vec![0.0; samples]);
        let mut raw = vec![0u8; samples * 8];
        for ch in 0..ch_count {
            audio[ch].resize(samples, 0.0);
            stream
                .read_exact(&mut raw)
                .map_err(|e| MessageError::new(ErrorCode::Data, format!("audio data f64: {e}")))?;
            for (i, chunk) in raw.chunks_exact(8).enumerate() {
                audio[ch][i] = f64::from_le_bytes(chunk.try_into().unwrap());
            }
        }
        *midi = read_midi_events(stream, self.res_header.num_midi_events.max(0) as usize)?;
        Ok(())
    }
}

fn validate_audio_shape(channels: i32, samples: i32, events: i32) -> Result<(), MessageError> {
    if !(0..=32).contains(&channels)
        || !(0..=32768).contains(&samples)
        || !(0..=4096).contains(&events)
    {
        return Err(MessageError::new(
            ErrorCode::Size,
            "Audio message exceeds channel/frame/MIDI limits",
        ));
    }
    Ok(())
}

fn write_midi_events<W: Write>(stream: &mut W, midi: &[MidiEvent]) -> Result<(), MessageError> {
    let mut total_bytes = 0usize;
    for ev in midi {
        total_bytes = total_bytes.saturating_add(ev.data.len());
        if ev.sample_number < 0 || ev.data.len() > 65536 || total_bytes > 1024 * 1024 {
            return Err(MessageError::new(
                ErrorCode::Size,
                "Invalid MIDI event size/offset",
            ));
        }
        let size = ev.data.len() as i32;
        stream
            .write_all(&ev.sample_number.to_le_bytes())
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        stream
            .write_all(&size.to_le_bytes())
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        stream
            .write_all(&ev.data)
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
    }
    Ok(())
}

fn read_midi_events<R: Read>(stream: &mut R, count: usize) -> Result<Vec<MidiEvent>, MessageError> {
    if count > 4096 {
        return Err(MessageError::new(ErrorCode::Size, "Too many MIDI events"));
    }
    let mut total_bytes = 0usize;
    let mut events = Vec::with_capacity(count);
    for _ in 0..count {
        let mut hdr = [0u8; 8];
        stream
            .read_exact(&mut hdr)
            .map_err(|e| MessageError::new(ErrorCode::Data, format!("midi header: {e}")))?;
        let sample_number = i32::from_le_bytes(hdr[0..4].try_into().unwrap());
        let declared_size = i32::from_le_bytes(hdr[4..8].try_into().unwrap());
        if !(0..=65536).contains(&declared_size) || sample_number < 0 {
            return Err(MessageError::new(
                ErrorCode::Size,
                "Invalid MIDI event size/offset",
            ));
        }
        let size = declared_size as usize;
        total_bytes += size;
        if total_bytes > 1024 * 1024 {
            return Err(MessageError::new(ErrorCode::Size, "MIDI payload too large"));
        }
        let mut data = vec![0u8; size];
        if size > 0 {
            stream
                .read_exact(&mut data)
                .map_err(|e| MessageError::new(ErrorCode::Data, format!("midi data: {e}")))?;
        }
        events.push(MidiEvent {
            sample_number,
            data,
        });
    }
    Ok(events)
}

/// All command message types from `Common/Source/Message.hpp` (`marcosfork-1.2.1`).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Any = 0,
    Quit = 1,
    Result = 2,
    PluginList = 10,
    AddPlugin = 20,
    AddPluginResult = 21,
    DelPlugin = 30,
    PluginStatus = 40,
    SetMonoChannels = 50,
    EditPlugin = 60,
    HidePlugin = 61,
    GetScreenBounds = 62,
    ScreenBounds = 63,
    ScreenCapture = 64,
    UpdateScreenCaptureArea = 65,
    Mouse = 66,
    Key = 67,
    Clipboard = 68,
    GetPluginSettings = 70,
    SetPluginSettings = 71,
    PluginSettings = 72,
    BypassPlugin = 73,
    UnbypassPlugin = 74,
    SetPluginOversampling = 75,
    ExchangePlugins = 80,
    RecentsList = 90,
    Parameters = 100,
    ParameterValue = 101,
    GetParameterValue = 102,
    GetAllParameterValues = 103,
    ParameterGesture = 104,
    Presets = 110,
    Preset = 111,
    Rescan = 120,
    Restart = 121,
    CPULoad = 130,
    Latency = 140,
    PluginLayouts = 141,
    AllParameterValues = 142,
    ServerError = 200,
}

#[allow(non_upper_case_globals)]
impl MessageType {
    pub const ParameterList: Self = Self::Parameters;
    pub const ScreenCaptureArea: Self = Self::UpdateScreenCaptureArea;
    pub const CpuLoad: Self = Self::CPULoad;
}

impl MessageType {
    pub fn from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::Any),
            1 => Some(Self::Quit),
            2 => Some(Self::Result),
            10 => Some(Self::PluginList),
            20 => Some(Self::AddPlugin),
            21 => Some(Self::AddPluginResult),
            30 => Some(Self::DelPlugin),
            40 => Some(Self::PluginStatus),
            50 => Some(Self::SetMonoChannels),
            60 => Some(Self::EditPlugin),
            61 => Some(Self::HidePlugin),
            62 => Some(Self::GetScreenBounds),
            63 => Some(Self::ScreenBounds),
            64 => Some(Self::ScreenCapture),
            65 => Some(Self::UpdateScreenCaptureArea),
            66 => Some(Self::Mouse),
            67 => Some(Self::Key),
            68 => Some(Self::Clipboard),
            70 => Some(Self::GetPluginSettings),
            71 => Some(Self::SetPluginSettings),
            72 => Some(Self::PluginSettings),
            73 => Some(Self::BypassPlugin),
            74 => Some(Self::UnbypassPlugin),
            75 => Some(Self::SetPluginOversampling),
            80 => Some(Self::ExchangePlugins),
            90 => Some(Self::RecentsList),
            100 => Some(Self::Parameters),
            101 => Some(Self::ParameterValue),
            102 => Some(Self::GetParameterValue),
            103 => Some(Self::GetAllParameterValues),
            104 => Some(Self::ParameterGesture),
            110 => Some(Self::Presets),
            111 => Some(Self::Preset),
            120 => Some(Self::Rescan),
            121 => Some(Self::Restart),
            130 => Some(Self::CPULoad),
            140 => Some(Self::Latency),
            141 => Some(Self::PluginLayouts),
            142 => Some(Self::AllParameterValues),
            200 => Some(Self::ServerError),
            _ => None,
        }
    }
}

/// Raw command message `[type: i32][size: i32][payload_bytes...]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMessage {
    pub msg_type: i32,
    pub payload: Vec<u8>,
}

impl RawMessage {
    pub fn new(msg_type: MessageType, payload: Vec<u8>) -> Self {
        Self {
            msg_type: msg_type as i32,
            payload,
        }
    }

    pub fn empty(msg_type: MessageType) -> Self {
        Self::new(msg_type, Vec::new())
    }

    pub fn from_number(msg_type: MessageType, num: i32) -> Self {
        Self::new(msg_type, num.to_le_bytes().to_vec())
    }

    pub fn get_number(&self) -> i32 {
        if self.payload.len() >= 4 {
            i32::from_le_bytes(self.payload[0..4].try_into().unwrap())
        } else {
            0
        }
    }

    pub fn from_float(msg_type: MessageType, val: f32) -> Self {
        Self::new(msg_type, val.to_le_bytes().to_vec())
    }

    pub fn get_float(&self) -> f32 {
        if self.payload.len() >= 4 {
            f32::from_le_bytes(self.payload[0..4].try_into().unwrap())
        } else {
            0.0
        }
    }

    pub fn from_string(msg_type: MessageType, s: &str) -> Self {
        let bytes = s.as_bytes();
        let len = bytes.len() as i32;
        let mut payload = Vec::with_capacity(4 + bytes.len());
        payload.extend_from_slice(&len.to_le_bytes());
        payload.extend_from_slice(bytes);
        Self::new(msg_type, payload)
    }

    pub fn get_string(&self) -> String {
        if self.payload.len() < 4 {
            return String::new();
        }
        let len = i32::from_le_bytes(self.payload[0..4].try_into().unwrap()).max(0) as usize;
        let end = (4 + len).min(self.payload.len());
        String::from_utf8_lossy(&self.payload[4..end]).to_string()
    }

    pub fn from_json(msg_type: MessageType, j: &Value) -> Self {
        let s = j.to_string();
        Self::from_string(msg_type, &s)
    }

    pub fn get_json(&self) -> Value {
        let s = self.get_string();
        serde_json::from_str(&s).unwrap_or_else(|_| json!({}))
    }

    pub fn from_msgpack(msg_type: MessageType, j: &Value) -> Self {
        let bytes = rmp_serde::to_vec(j).unwrap_or_default();
        let len = bytes.len() as i32;
        let mut payload = Vec::with_capacity(4 + bytes.len());
        payload.extend_from_slice(&len.to_le_bytes());
        payload.extend_from_slice(&bytes);
        Self::new(msg_type, payload)
    }

    pub fn get_msgpack(&self) -> Value {
        if self.payload.len() < 4 {
            return json!({});
        }
        let len = i32::from_le_bytes(self.payload[0..4].try_into().unwrap()).max(0) as usize;
        let end = (4 + len).min(self.payload.len());
        rmp_serde::from_slice(&self.payload[4..end]).unwrap_or_else(|_| json!({}))
    }

    pub fn result(rc: i32, text: &str) -> Self {
        let bytes = text.as_bytes();
        let size = bytes.len() as i32;
        let mut payload = Vec::with_capacity(8 + bytes.len());
        payload.extend_from_slice(&rc.to_le_bytes());
        payload.extend_from_slice(&size.to_le_bytes());
        payload.extend_from_slice(bytes);
        Self::new(MessageType::Result, payload)
    }

    pub fn parse_result(&self) -> (i32, String) {
        if self.payload.len() < 8 {
            return (0, String::new());
        }
        let rc = i32::from_le_bytes(self.payload[0..4].try_into().unwrap());
        let size = i32::from_le_bytes(self.payload[4..8].try_into().unwrap()).max(0) as usize;
        let end = (8 + size).min(self.payload.len());
        let text = String::from_utf8_lossy(&self.payload[8..end]).to_string();
        (rc, text)
    }

    pub fn set_plugin_oversampling(idx: i32, factor: i32) -> Self {
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&idx.to_le_bytes());
        payload.extend_from_slice(&factor.to_le_bytes());
        Self::new(MessageType::SetPluginOversampling, payload)
    }

    pub fn parse_oversampling(&self) -> (i32, i32) {
        if self.payload.len() < 8 {
            return (0, 1);
        }
        let idx = i32::from_le_bytes(self.payload[0..4].try_into().unwrap());
        let factor = i32::from_le_bytes(self.payload[4..8].try_into().unwrap());
        (idx, factor)
    }

    pub fn exchange_plugins(idx_a: i32, idx_b: i32) -> Self {
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&idx_a.to_le_bytes());
        payload.extend_from_slice(&idx_b.to_le_bytes());
        Self::new(MessageType::ExchangePlugins, payload)
    }

    pub fn parse_exchange_plugins(&self) -> (i32, i32) {
        if self.payload.len() < 8 {
            return (0, 0);
        }
        let a = i32::from_le_bytes(self.payload[0..4].try_into().unwrap());
        let b = i32::from_le_bytes(self.payload[4..8].try_into().unwrap());
        (a, b)
    }

    pub fn parameter_value(idx: i32, param_idx: i32, value: f32, channel: i32) -> Self {
        let mut payload = Vec::with_capacity(16);
        payload.extend_from_slice(&idx.to_le_bytes());
        payload.extend_from_slice(&param_idx.to_le_bytes());
        payload.extend_from_slice(&value.to_le_bytes());
        payload.extend_from_slice(&channel.to_le_bytes());
        Self::new(MessageType::ParameterValue, payload)
    }

    pub fn parse_parameter_value(&self) -> (i32, i32, f32, i32) {
        if self.payload.len() < 16 {
            return (0, 0, 0.0, 0);
        }
        let idx = i32::from_le_bytes(self.payload[0..4].try_into().unwrap());
        let param_idx = i32::from_le_bytes(self.payload[4..8].try_into().unwrap());
        let value = f32::from_le_bytes(self.payload[8..12].try_into().unwrap());
        let channel = i32::from_le_bytes(self.payload[12..16].try_into().unwrap());
        (idx, param_idx, value, channel)
    }

    pub fn get_parameter_value(idx: i32, param_idx: i32, channel: i32) -> Self {
        let mut payload = Vec::with_capacity(12);
        payload.extend_from_slice(&idx.to_le_bytes());
        payload.extend_from_slice(&param_idx.to_le_bytes());
        payload.extend_from_slice(&channel.to_le_bytes());
        Self::new(MessageType::GetParameterValue, payload)
    }

    pub fn parse_get_parameter_value(&self) -> (i32, i32, i32) {
        if self.payload.len() < 12 {
            return (0, 0, 0);
        }
        let idx = i32::from_le_bytes(self.payload[0..4].try_into().unwrap());
        let param_idx = i32::from_le_bytes(self.payload[4..8].try_into().unwrap());
        let channel = i32::from_le_bytes(self.payload[8..12].try_into().unwrap());
        (idx, param_idx, channel)
    }

    pub fn parameter_gesture(idx: i32, param_idx: i32, starting: bool, channel: i32) -> Self {
        let mut payload = Vec::with_capacity(16);
        payload.extend_from_slice(&idx.to_le_bytes());
        payload.extend_from_slice(&param_idx.to_le_bytes());
        payload.extend_from_slice(&(i32::from(starting)).to_le_bytes());
        payload.extend_from_slice(&channel.to_le_bytes());
        Self::new(MessageType::ParameterGesture, payload)
    }

    pub fn parse_parameter_gesture(&self) -> (i32, i32, bool, i32) {
        if self.payload.len() < 16 {
            return (0, 0, false, 0);
        }
        let idx = i32::from_le_bytes(self.payload[0..4].try_into().unwrap());
        let param_idx = i32::from_le_bytes(self.payload[4..8].try_into().unwrap());
        let starting = self.payload[8] != 0;
        let channel = i32::from_le_bytes(self.payload[12..16].try_into().unwrap());
        (idx, param_idx, starting, channel)
    }

    pub fn edit_plugin(index: i32, channel: i32, x: i32, y: i32) -> Self {
        let mut payload = Vec::with_capacity(16);
        payload.extend_from_slice(&index.to_le_bytes());
        payload.extend_from_slice(&channel.to_le_bytes());
        payload.extend_from_slice(&x.to_le_bytes());
        payload.extend_from_slice(&y.to_le_bytes());
        Self::new(MessageType::EditPlugin, payload)
    }

    pub fn parse_edit_plugin(&self) -> (i32, i32, i32, i32) {
        if self.payload.len() < 16 {
            return (0, 0, 0, 0);
        }
        (
            i32::from_le_bytes(self.payload[0..4].try_into().unwrap()),
            i32::from_le_bytes(self.payload[4..8].try_into().unwrap()),
            i32::from_le_bytes(self.payload[8..12].try_into().unwrap()),
            i32::from_le_bytes(self.payload[12..16].try_into().unwrap()),
        )
    }

    pub fn set_mono_channels(idx: i32, channels: u64) -> Self {
        let mut payload = vec![0u8; 16];
        payload[0..4].copy_from_slice(&idx.to_le_bytes());
        payload[8..16].copy_from_slice(&channels.to_le_bytes());
        Self::new(MessageType::SetMonoChannels, payload)
    }

    pub fn parse_set_mono_channels(&self) -> (i32, u64) {
        if self.payload.len() < 16 {
            return (0, 0);
        }
        (
            i32::from_le_bytes(self.payload[0..4].try_into().unwrap()),
            u64::from_le_bytes(self.payload[8..16].try_into().unwrap()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn screen_capture(
        width: i32,
        height: i32,
        width_padded: i32,
        height_padded: i32,
        scale: f64,
        image_bytes: &[u8],
    ) -> Self {
        let mut payload = Vec::with_capacity(32 + image_bytes.len());
        payload.extend_from_slice(&width.to_le_bytes());
        payload.extend_from_slice(&height.to_le_bytes());
        payload.extend_from_slice(&width_padded.to_le_bytes());
        payload.extend_from_slice(&height_padded.to_le_bytes());
        payload.extend_from_slice(&scale.to_le_bytes());
        payload.extend_from_slice(&(image_bytes.len() as u64).to_le_bytes());
        payload.extend_from_slice(image_bytes);
        Self::new(MessageType::ScreenCapture, payload)
    }

    pub fn parse_screen_capture(&self) -> Option<(i32, i32, i32, i32, f64, &[u8])> {
        if self.payload.len() < 32 {
            return None;
        }
        let width = i32::from_le_bytes(self.payload[0..4].try_into().unwrap());
        let height = i32::from_le_bytes(self.payload[4..8].try_into().unwrap());
        let width_padded = i32::from_le_bytes(self.payload[8..12].try_into().unwrap());
        let height_padded = i32::from_le_bytes(self.payload[12..16].try_into().unwrap());
        let scale = f64::from_le_bytes(self.payload[16..24].try_into().unwrap());
        let size = u64::from_le_bytes(self.payload[24..32].try_into().unwrap()) as usize;
        let end = (32 + size).min(self.payload.len());
        Some((
            width,
            height,
            width_padded,
            height_padded,
            scale,
            &self.payload[32..end],
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mouse_event(
        ev_type: MouseEvType,
        x: f32,
        y: f32,
        is_shift_down: bool,
        is_ctrl_down: bool,
        is_alt_down: bool,
        delta_x: f32,
        delta_y: f32,
        is_smooth: bool,
    ) -> Self {
        let mut payload = vec![0u8; 28];
        payload[0] = ev_type as u8;
        payload[4..8].copy_from_slice(&x.to_le_bytes());
        payload[8..12].copy_from_slice(&y.to_le_bytes());
        payload[12] = u8::from(is_shift_down);
        payload[13] = u8::from(is_ctrl_down);
        payload[14] = u8::from(is_alt_down);
        payload[16..20].copy_from_slice(&delta_x.to_le_bytes());
        payload[20..24].copy_from_slice(&delta_y.to_le_bytes());
        payload[24] = u8::from(is_smooth);
        Self::new(MessageType::Mouse, payload)
    }

    pub fn send<W: Write>(&self, stream: &mut W) -> Result<(), MessageError> {
        if self.payload.len() > MAX_MESSAGE_SIZE {
            return Err(MessageError::new(
                ErrorCode::Size,
                "max message size exceeded",
            ));
        }
        let mut hdr = [0u8; 8];
        hdr[0..4].copy_from_slice(&self.msg_type.to_le_bytes());
        hdr[4..8].copy_from_slice(&(self.payload.len() as i32).to_le_bytes());
        stream
            .write_all(&hdr)
            .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        if !self.payload.is_empty() {
            stream
                .write_all(&self.payload)
                .map_err(|e| MessageError::new(ErrorCode::Syscall, e.to_string()))?;
        }
        Ok(())
    }

    pub fn read<R: Read>(stream: &mut R) -> Result<Self, MessageError> {
        let mut hdr = [0u8; 8];
        stream.read_exact(&mut hdr).map_err(|e| {
            MessageError::new(
                ErrorCode::Data,
                format!("failed to read message header: {e}"),
            )
        })?;
        let msg_type = i32::from_le_bytes(hdr[0..4].try_into().unwrap());
        let size = i32::from_le_bytes(hdr[4..8].try_into().unwrap());
        if size < 0 || (size as usize) > MAX_MESSAGE_SIZE {
            return Err(MessageError::new(
                ErrorCode::Data,
                format!("invalid message size {size}"),
            ));
        }
        let mut payload = vec![0u8; size as usize];
        if size > 0 {
            stream.read_exact(&mut payload).map_err(|e| {
                MessageError::new(ErrorCode::Data, format!("failed to read message body: {e}"))
            })?;
        }
        Ok(Self { msg_type, payload })
    }
}

/// Port of `e47::JsonMessage`, `PluginTrayMessage`, and `SandboxMessage`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonMessage {
    #[serde(rename = "type")]
    pub msg_type: u16,
    pub uuid: String,
    pub data: Value,
}

impl JsonMessage {
    pub fn new(msg_type: u16, data: Value, uuid: impl Into<String>) -> Self {
        Self {
            msg_type,
            uuid: uuid.into(),
            data,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTrayMessageType {
    Status = 0,
    ChangeServer = 1,
    Reload = 2,
    GetRecents = 3,
    UpdateRecents = 4,
    ShowMonitor = 5,
    Stop = 6,
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMessageType {
    Config = 0,
    SandboxPort = 1,
    ShowEditor = 2,
    HideEditor = 3,
    Metrics = 4,
}

impl RawMessage {
    pub fn from_str(msg_type: MessageType, s: &str) -> Self {
        Self::new(msg_type, s.as_bytes().to_vec())
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.payload).unwrap_or("")
    }

    pub fn msg_type_enum(&self) -> MessageType {
        MessageType::from_i32(self.msg_type).unwrap_or(MessageType::Any)
    }
}

pub struct MessageFactory;

impl MessageFactory {
    pub fn get_next_message<R: Read>(
        stream: &mut R,
        _abort_flag: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<RawMessage, MessageError> {
        RawMessage::read(stream)
    }
}

pub fn write_value<W: Write, const N: usize>(
    stream: &mut W,
    bytes: &[u8; N],
) -> std::io::Result<()> {
    stream.write_all(bytes)
}

pub fn read_value<R: Read, const N: usize>(stream: &mut R) -> std::io::Result<[u8; N]> {
    let mut buf = [0u8; N];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

pub fn send_json_message<W: Write, T: Serialize>(
    stream: &mut W,
    msg_type: MessageType,
    data: &T,
) -> Result<(), MessageError> {
    let json_bytes =
        serde_json::to_vec(data).map_err(|e| MessageError::new(ErrorCode::Data, e.to_string()))?;
    RawMessage::new(msg_type, json_bytes).send(stream)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenHeader {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub size: u32,
}

impl ScreenHeader {
    pub const TYPE: MessageType = MessageType::ScreenCapture;
    pub const SIZE: usize = 20;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.width.to_le_bytes());
        b[4..8].copy_from_slice(&self.height.to_le_bytes());
        b[8..16].copy_from_slice(&self.scale.to_le_bytes());
        b[16..20].copy_from_slice(&self.size.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < Self::SIZE {
            return None;
        }
        Some(Self {
            width: u32::from_le_bytes(b[0..4].try_into().ok()?),
            height: u32::from_le_bytes(b[4..8].try_into().ok()?),
            scale: f64::from_le_bytes(b[8..16].try_into().ok()?),
            size: u32::from_le_bytes(b[16..20].try_into().ok()?),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AddPluginResult {
    pub ok: bool,
    pub latency_samples: i32,
    pub disabled_sidechain: bool,
    pub err: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OversamplingPayload {
    pub idx: i32,
    pub factor: i32,
}

impl OversamplingPayload {
    pub const SIZE: usize = 8;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.idx.to_le_bytes());
        b[4..8].copy_from_slice(&self.factor.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < Self::SIZE {
            return None;
        }
        Some(Self {
            idx: i32::from_le_bytes(b[0..4].try_into().ok()?),
            factor: i32::from_le_bytes(b[4..8].try_into().ok()?),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExchangePayload {
    pub idx_a: i32,
    pub idx_b: i32,
}

impl ExchangePayload {
    pub const SIZE: usize = 8;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.idx_a.to_le_bytes());
        b[4..8].copy_from_slice(&self.idx_b.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < Self::SIZE {
            return None;
        }
        Some(Self {
            idx_a: i32::from_le_bytes(b[0..4].try_into().ok()?),
            idx_b: i32::from_le_bytes(b[4..8].try_into().ok()?),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetPayload {
    pub idx: i32,
    pub preset: i32,
}

impl PresetPayload {
    pub const SIZE: usize = 8;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.idx.to_le_bytes());
        b[4..8].copy_from_slice(&self.preset.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < Self::SIZE {
            return None;
        }
        Some(Self {
            idx: i32::from_le_bytes(b[0..4].try_into().ok()?),
            preset: i32::from_le_bytes(b[4..8].try_into().ok()?),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParameterValuePayload {
    pub idx: i32,
    pub param_idx: i32,
    pub value: f32,
}

impl ParameterValuePayload {
    pub const SIZE: usize = 12;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.idx.to_le_bytes());
        b[4..8].copy_from_slice(&self.param_idx.to_le_bytes());
        b[8..12].copy_from_slice(&self.value.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < Self::SIZE {
            return None;
        }
        Some(Self {
            idx: i32::from_le_bytes(b[0..4].try_into().ok()?),
            param_idx: i32::from_le_bytes(b[4..8].try_into().ok()?),
            value: f32::from_le_bytes(b[8..12].try_into().ok()?),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenBoundsPayload {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl ScreenBoundsPayload {
    pub const SIZE: usize = 16;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.x.to_le_bytes());
        b[4..8].copy_from_slice(&self.y.to_le_bytes());
        b[8..12].copy_from_slice(&self.w.to_le_bytes());
        b[12..16].copy_from_slice(&self.h.to_le_bytes());
        b
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CpuLoadPayload {
    pub val: f32,
}

impl CpuLoadPayload {
    pub const SIZE: usize = 4;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        self.val.to_le_bytes()
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < Self::SIZE {
            return None;
        }
        Some(Self {
            val: f32::from_le_bytes(b[0..4].try_into().ok()?),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MousePayload {
    pub ev_type: MouseEvType,
    pub x: f32,
    pub y: f32,
    pub is_shift_down: bool,
    pub is_ctrl_down: bool,
    pub is_alt_down: bool,
    pub delta_x: f32,
    pub delta_y: f32,
    pub is_smooth: bool,
}

impl MousePayload {
    pub const SIZE: usize = 28;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut payload = [0u8; Self::SIZE];
        payload[0] = self.ev_type as u8;
        payload[4..8].copy_from_slice(&self.x.to_le_bytes());
        payload[8..12].copy_from_slice(&self.y.to_le_bytes());
        payload[12] = u8::from(self.is_shift_down);
        payload[13] = u8::from(self.is_ctrl_down);
        payload[14] = u8::from(self.is_alt_down);
        payload[16..20].copy_from_slice(&self.delta_x.to_le_bytes());
        payload[20..24].copy_from_slice(&self.delta_y.to_le_bytes());
        payload[24] = u8::from(self.is_smooth);
        payload
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPayload {
    pub key_codes: Vec<u16>,
}

impl KeyPayload {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.key_codes.len() * 2);
        for &kc in &self.key_codes {
            out.extend_from_slice(&kc.to_le_bytes());
        }
        out
    }
}

impl PluginTrayMessageType {
    pub fn from_u16(v: u16) -> Option<Self> {
        match v {
            0 => Some(Self::Status),
            1 => Some(Self::ChangeServer),
            2 => Some(Self::Reload),
            3 => Some(Self::GetRecents),
            4 => Some(Self::UpdateRecents),
            5 => Some(Self::ShowMonitor),
            6 => Some(Self::Stop),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PluginTrayMessage {
    pub msg_type: PluginTrayMessageType,
    pub data: Value,
}

impl PluginTrayMessage {
    pub fn new(msg_type: PluginTrayMessageType, data: Value) -> Self {
        Self { msg_type, data }
    }

    pub fn send_to<W: Write>(&self, stream: &mut W) -> Result<(), MessageError> {
        let jm = JsonMessage::new(self.msg_type as u16, self.data.clone(), "");
        let raw = RawMessage::new(MessageType::Any, jm.serialize());
        raw.send(stream)
    }

    pub fn read_from<R: Read>(stream: &mut R) -> Result<Self, MessageError> {
        let raw = RawMessage::read(stream)?;
        let jm = JsonMessage::deserialize(&raw.payload)
            .map_err(|e| MessageError::new(ErrorCode::Data, e))?;
        let mt =
            PluginTrayMessageType::from_u16(jm.msg_type).unwrap_or(PluginTrayMessageType::Status);
        Ok(Self {
            msg_type: mt,
            data: jm.data,
        })
    }
}

#[cfg(test)]
mod audio_transport_limits_tests {
    use super::*;
    #[derive(Default)]
    struct CountWrites {
        calls: usize,
        bytes: Vec<u8>,
    }
    impl Write for CountWrites {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.calls += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn audio_blocks_are_single_writes_and_preserve_wire_data() {
        let mut message = AudioMessage::new();
        let mut stream = CountWrites::default();
        let audio = vec![vec![0.25f32; 64], vec![-0.5; 64]];
        message
            .send_to_server_f32(&mut stream, &audio, &[], &PositionInfo::default(), 2, 64)
            .unwrap();
        assert_eq!(stream.calls, 1);
        let (mut single, mut double, mut midi, mut position) =
            (Vec::new(), Vec::new(), Vec::new(), PositionInfo::default());
        message
            .read_from_client(
                &mut std::io::Cursor::new(&stream.bytes),
                &mut single,
                &mut double,
                &mut midi,
                &mut position,
            )
            .unwrap();
        assert_eq!(single, audio);
        let mut stream = CountWrites::default();
        let audio = vec![vec![0.125f64; 127], vec![-0.25; 127]];
        message
            .send_to_client_f64(&mut stream, &audio, &[], 18, 2)
            .unwrap();
        assert_eq!(stream.calls, 1);
        let mut decoded = Vec::new();
        message
            .read_from_server_f64(
                &mut std::io::Cursor::new(&stream.bytes),
                &mut decoded,
                &mut midi,
            )
            .unwrap();
        assert_eq!(decoded, audio);
        assert_eq!(message.res_header.latency_samples, 18);
    }
    #[test]
    fn oversized_audio_and_midi_are_rejected_before_allocation() {
        let mut message = AudioMessage::new();
        let header = AudioRequestHeader {
            channels: i32::MAX,
            samples: i32::MAX,
            ..Default::default()
        };
        let (mut single, mut double, mut midi, mut position) =
            (Vec::new(), Vec::new(), Vec::new(), PositionInfo::default());
        assert!(message
            .read_from_client(
                &mut std::io::Cursor::new(header.to_bytes()),
                &mut single,
                &mut double,
                &mut midi,
                &mut position
            )
            .is_err());
        assert!(single.is_empty() && double.is_empty());
        let response = AudioResponseHeader {
            channels: 2,
            samples: -1,
            ..Default::default()
        };
        assert!(message
            .read_from_server_f32(
                &mut std::io::Cursor::new(response.to_bytes()),
                &mut single,
                &mut midi
            )
            .is_err());
        let mut stream = CountWrites::default();
        assert!(message
            .send_to_server_f32(
                &mut stream,
                &[vec![0.0; 64], vec![0.0; 63]],
                &[],
                &position,
                2,
                64
            )
            .is_err());
        assert_eq!(stream.calls, 0);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&i32::MAX.to_le_bytes());
        assert!(read_midi_events(&mut std::io::Cursor::new(bytes), 1).is_err());
    }
}
