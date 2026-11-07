//! AudioGridder host for protocol v13.
//!
//! The worker hosts a builtin gain plus scanned AudioUnit, LV2, and VST3
//! plug-ins. Socket reads and writes stay in the worker. The insert's audio
//! callback does not touch a socket.
//!
//! One socket: the handshake answer carries the listen port, and the same
//! connection carries the audio block. Four sockets wait until a screen
//! exists. The handshake does not set the OpenGridder `STATE_ACK` flag.

mod chain;
mod channels;
mod clipboard;
mod screen;
mod cpu;
mod host;
mod native;
pub mod menu;
pub mod scanner;
pub mod mdns;
pub mod vst2;
mod worker;

pub use worker::{restart_requested, sandbox_chain_child, sandbox_plugin_child, serve_chain, serve_clients, serve_plugin, serve_worker};

#[cfg(test)]
mod feature_tests;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use ag_protocol::{
    audio_shape_ok, read_audio_request, write_audio_response, AudioRequestHeader, HandshakeRequest,
    HandshakeResponse, PositionInfo, PROTOCOL_VERSION,
};
use openplugin::prelude::*;

const IO_TIMEOUT: Duration = Duration::from_secs(2);

static GAIN_PARAMS: [ParamInfo; 1] = [ParamInfo::new("gain", "Gain", -24.0, 24.0, 0.0, Unit::Db)];

static GAIN_INFO: PluginInfo = PluginInfo {
    id: "com.opengrid.audiogridder.hostgain",
    name: "AudioGridder Host Gain",
    vendor: "OpenGrid",
    url: "https://opengrid.studio",
    email: "contact@mistamin.com",
    version: "2.0.0-alpha.1",
    category: Category::Utility,
    params: &GAIN_PARAMS,
    is_instrument: false,
    has_editor: false,
};

/// Builtin gain the alpha host can run. Not a substitute for a plugin scanner.
pub struct HostedGain {
    gain_db: f32,
    linear: f32,
    transport: Transport,
}

impl Default for HostedGain {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedGain {
    pub fn new() -> Self {
        Self {
            gain_db: 0.0,
            linear: 1.0,
            transport: Transport::default(),
        }
    }

    pub fn set_gain_db(&mut self, db: f32) {
        let db = if db.is_nan() { 0.0 } else { db.clamp(-24.0, 24.0) };
        self.gain_db = db;
        self.linear = if db.abs() < 1.0e-6 { 1.0 } else { 10.0f32.powf(db / 20.0) };
    }

    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    /// 0 dB is exactly 1.0. Other gains use an f64 power, not the f32 coefficient.
    pub fn linear_f64(&self) -> f64 {
        if self.gain_db.abs() < 1.0e-6 {
            1.0
        } else {
            10.0f64.powf(f64::from(self.gain_db) / 20.0)
        }
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }
}

impl Plugin for HostedGain {
    fn info(&self) -> &'static PluginInfo {
        &GAIN_INFO
    }

    fn prepare(&mut self, _sample_rate: f32, _max_block: usize, _channels: usize) {}

    fn reset(&mut self) {}

    fn set_param(&mut self, id: &str, value: f32) -> bool {
        if id != "gain" {
            return false;
        }
        self.set_gain_db(value);
        true
    }

    fn param(&self, id: &str) -> Option<f32> {
        (id == "gain").then_some(self.gain_db)
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        let linear = self.linear;
        for channel in io.iter_mut() {
            let count = frames.min(channel.len());
            for sample in &mut channel[..count] {
                *sample *= linear;
            }
        }
    }

    fn set_transport(&mut self, transport: &Transport) {
        self.transport = *transport;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    Version,
    Sidechain,
    Channels,
    Rate,
    Block,
    Double,
    AudioShape,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SessionKind {
    Rejected(RejectReason),
    Processed { latency: i32, first_sample: f32, playing: bool, tempo: f64 },
}

pub fn validate_handshake(req: &HandshakeRequest) -> Result<(), RejectReason> {
    if req.version != PROTOCOL_VERSION {
        return Err(RejectReason::Version);
    }
    if req.channels_sc != 0 {
        return Err(RejectReason::Sidechain);
    }
    if req.channels_in != req.channels_out || !(1..=2).contains(&req.channels_in) {
        return Err(RejectReason::Channels);
    }
    if !req.sample_rate.is_finite() || !(8_000.0..=192_000.0).contains(&req.sample_rate) {
        return Err(RejectReason::Rate);
    }
    if !(1..=8192).contains(&req.samples_per_block) {
        return Err(RejectReason::Block);
    }
    Ok(())
}

pub(crate) fn transport_from(position: &PositionInfo) -> Transport {
    Transport {
        playing: position.playing,
        recording: position.recording,
        tempo: position.bpm,
        numerator: position.time_sig_numerator.max(0) as u32,
        denominator: position.time_sig_denominator.max(0) as u32,
        sample_pos: position.time_in_samples,
        seconds: position.time_in_seconds,
        quarter_notes: position.ppq_position,
        bar_start: 0.0,
        bar_number: 0,
    }
}

/// Accept one client on `listener`, answer the handshake with that listen
/// port, run one audio block through `plugin`, and write the response.
pub fn serve_one(listener: TcpListener, plugin: &mut HostedGain) -> std::io::Result<SessionKind> {
    listener.set_nonblocking(false)?;
    let (mut stream, _) = listener.accept()?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let listen_port = listener.local_addr()?.port();

    let mut raw = [0u8; HandshakeRequest::WIRE_SIZE];
    if stream.read_exact(&mut raw).is_err() {
        return Ok(SessionKind::Rejected(RejectReason::Version));
    }
    let Some(req) = HandshakeRequest::from_bytes(&raw) else {
        return Ok(SessionKind::Rejected(RejectReason::Version));
    };
    if let Err(reason) = validate_handshake(&req) {
        let _ = stream.shutdown(std::net::Shutdown::Both);
        return Ok(SessionKind::Rejected(reason));
    }

    let response = HandshakeResponse {
        version: PROTOCOL_VERSION,
        flags: 0,
        port: i32::from(listen_port),
    };
    stream.write_all(&response.to_bytes())?;

    let mut header_raw = [0u8; AudioRequestHeader::WIRE_SIZE];
    stream.read_exact(&mut header_raw)?;
    let Some(header) = AudioRequestHeader::from_bytes(&header_raw) else {
        return Ok(SessionKind::Rejected(RejectReason::AudioShape));
    };
    if header.is_double
        || header.midi_events != 0
        || !audio_shape_ok(header.channels, header.samples, header.midi_events, false)
        || header.channels != req.channels_in
    {
        let _ = stream.shutdown(std::net::Shutdown::Both);
        return Ok(SessionKind::Rejected(RejectReason::AudioShape));
    }

    let body = header.channels as usize * header.samples as usize * 4 + PositionInfo::WIRE_SIZE;
    let mut message = Vec::with_capacity(AudioRequestHeader::WIRE_SIZE + body);
    message.extend_from_slice(&header_raw);
    message.resize(message.capacity(), 0);
    stream.read_exact(&mut message[AudioRequestHeader::WIRE_SIZE..])?;
    let Some((_header, mut audio, position)) = read_audio_request(&message) else {
        return Ok(SessionKind::Rejected(RejectReason::AudioShape));
    };

    plugin.prepare(req.sample_rate as f32, req.samples_per_block as usize, req.channels_in as usize);
    let transport = transport_from(&position);
    plugin.set_transport(&transport);
    let frames = header.samples as usize;
    plugin.process(&mut audio, frames);
    let latency = plugin.latency() as i32;
    let bytes = write_audio_response(&audio, latency).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "hosted plugin returned a shape this alpha cannot answer")
    })?;
    stream.write_all(&bytes)?;
    let first_sample = audio.first().and_then(|c| c.first().copied()).unwrap_or(0.0);
    Ok(SessionKind::Processed {
        latency,
        first_sample,
        playing: transport.playing,
        tempo: transport.tempo,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_protocol::{read_audio_response, write_audio_request, AudioResponseHeader, HandshakeResponse, PositionInfo, PROTOCOL_VERSION};
    use std::net::TcpStream;
    use std::thread;

    fn expected_linear(db: f32) -> f32 {
        10.0f32.powf(db / 20.0)
    }

    #[test]
    fn hosted_gain_is_minus_6_db() {
        let mut plugin = HostedGain::new();
        plugin.set_gain_db(-6.0);
        plugin.prepare(48_000.0, 64, 1);
        let mut io = vec![vec![1.0f32; 8]];
        plugin.process(&mut io, 8);
        let expected = expected_linear(-6.0);
        assert!((io[0][0] - expected).abs() < 1.0e-6, "{} vs {expected}", io[0][0]);
        assert!((plugin.gain_db() + 6.0).abs() < 1.0e-6);
    }

    fn handshake(channels: i32, sidechain: i32) -> HandshakeRequest {
        HandshakeRequest {
            channels_in: channels,
            channels_out: channels,
            channels_sc: sidechain,
            samples_per_block: 64,
            ..HandshakeRequest::default()
        }
    }

    #[test]
    fn one_socket_session_vs_worker_port_mode() {
        // Mode 1: One-socket session (serve_one)
        // Handshake answer carries listen_port and audio flows on the SAME socket connection.
        {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let listen_port = listener.local_addr().unwrap().port();
            let server = thread::spawn(move || {
                let mut plugin = HostedGain::new();
                plugin.set_gain_db(-6.0);
                serve_one(listener, &mut plugin).unwrap()
            });

            let mut stream = TcpStream::connect(("127.0.0.1", listen_port)).unwrap();
            stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
            stream.write_all(&handshake(2, 0).to_bytes()).unwrap();
            let mut raw = [0u8; HandshakeResponse::WIRE_SIZE];
            stream.read_exact(&mut raw).unwrap();
            let answer = HandshakeResponse::from_bytes(&raw).unwrap();
            assert_eq!(answer.version, PROTOCOL_VERSION);
            assert_eq!(answer.flags, 0);
            assert_eq!(answer.port, i32::from(listen_port), "serve_one answers with the listen port on the same socket");

            let mut position = PositionInfo::default();
            position.playing = true;
            position.bpm = 128.0;
            let request = write_audio_request(&[vec![0.5; 64], vec![0.25; 64]], &position).unwrap();
            stream.write_all(&request).unwrap();
            let mut response = vec![0u8; AudioResponseHeader::WIRE_SIZE + 2 * 64 * 4];
            stream.read_exact(&mut response).unwrap();
            let (header, audio) = read_audio_response(&response).unwrap();
            let expected = 0.5 * expected_linear(-6.0);
            assert_eq!(header.latency_samples, 0);
            assert_eq!(header.channels, 2);
            assert!((audio[0][10] - expected).abs() < 1.0e-5, "{}", audio[0][10]);
            assert!((audio[1][10] - 0.25 * expected_linear(-6.0)).abs() < 1.0e-5);

            match server.join().unwrap() {
                SessionKind::Processed { playing, tempo, first_sample, .. } => {
                    assert!(playing);
                    assert!((tempo - 128.0).abs() < 1.0e-9);
                    assert!((first_sample - expected).abs() < 1.0e-5);
                }
                other => panic!("expected a processed block, got {other:?}"),
            }
        }

        // Mode 2: Worker port mode (serve_clients / serve_worker)
        // Handshake answer moves client to a distinct worker port (answer.port != listen_port).
        {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let listen_port = listener.local_addr().unwrap().port();
            let _server = thread::spawn(move || serve_clients(listener));

            let mut stream = TcpStream::connect(("127.0.0.1", listen_port)).unwrap();
            stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
            stream.write_all(&handshake(2, 0).to_bytes()).unwrap();
            let mut raw = [0u8; HandshakeResponse::WIRE_SIZE];
            stream.read_exact(&mut raw).unwrap();
            let answer = HandshakeResponse::from_bytes(&raw).unwrap();
            assert_eq!(answer.version, PROTOCOL_VERSION);
            assert_eq!(answer.flags, 0);
            assert_ne!(
                answer.port,
                i32::from(listen_port),
                "worker port mode must assign a distinct ephemeral worker port"
            );
            assert!(answer.port > 0 && answer.port <= i32::from(u16::MAX));
            drop(stream);
        }
    }

    #[test]
    fn sidechain_and_wide_layouts_close_without_an_answer() {
        // Assert rejection on both serve_one (one-socket path) and serve_clients / serve_worker (worker port path)
        for (channels, sidechain, reason) in [(2, 1, RejectReason::Sidechain), (8, 0, RejectReason::Channels)] {
            // 1. One-socket path: serve_one
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = thread::spawn(move || serve_one(listener, &mut HostedGain::new()).unwrap());
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
            stream.write_all(&handshake(channels, sidechain).to_bytes()).unwrap();
            let mut raw = [0u8; 8];
            let err = stream.read_exact(&mut raw).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
            assert_eq!(server.join().unwrap(), SessionKind::Rejected(reason));

            // 2. Worker port path: serve_worker (rejects handshake and drops stream without answering)
            let listener_worker = TcpListener::bind("127.0.0.1:0").unwrap();
            let worker_listen_port = listener_worker.local_addr().unwrap().port();
            let worker_thread = thread::spawn(move || serve_worker(listener_worker));
            let mut worker_stream = TcpStream::connect(("127.0.0.1", worker_listen_port)).unwrap();
            worker_stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
            worker_stream.write_all(&handshake(channels, sidechain).to_bytes()).unwrap();
            let mut raw_worker = [0u8; 8];
            let err_worker = worker_stream.read_exact(&mut raw_worker).unwrap_err();
            assert_eq!(err_worker.kind(), std::io::ErrorKind::UnexpectedEof);
            let worker_res = worker_thread.join().unwrap();
            assert!(worker_res.is_err(), "serve_worker should return error on rejected handshake");
        }
    }
}
