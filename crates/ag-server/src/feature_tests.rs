//! AudioGridder README feature list, exercised through `serve_worker`.
//!
//! Each test drives the shipped server. Expected behavior:
//! - multi-slot chain, bypass, reorder, parameter round-trip
//! - 2x/4x oversampling changes an impulse and raises reported latency
//! - a 64-bit float block keeps a value f32 cannot hold
//! - MIDI note-on reaches a scanned instrument and the block is no longer silent
//! - a scanned effect (not Builtin-gain) processes audio, and one parameter moves the next block
//! - the screen socket carries a decodable editor image, and a mouse event changes a parameter

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use ag_protocol::command::{
    self, ADD_PLUGIN, ADD_PLUGIN_RESULT, BYPASS_PLUGIN, DEL_PLUGIN, EDIT_PLUGIN, EXCHANGE_PLUGINS, GET_PARAMETER_VALUE,
    GET_ALL_PARAMETER_VALUES, GET_PLUGIN_SETTINGS, GET_SCREEN_BOUNDS, HIDE_PLUGIN, KEY, MOUSE, PARAMETER_GESTURE,
    CLIPBOARD, CPU_LOAD, PARAMETER_VALUE, PARAMETERS, PLUGIN_LIST, PLUGIN_SETTINGS, PLUGIN_STATUS, PRESET, PRESETS, RECENTS_LIST,
    RESCAN, RESTART, RESULT, SCREEN_BOUNDS, SCREEN_CAPTURE, SET_PLUGIN_OVERSAMPLING, SET_PLUGIN_SETTINGS,
    UPDATE_SCREEN_CAPTURE_AREA,
    UNBYPASS_PLUGIN, QUIT,
};
use ag_protocol::{AudioRequestHeader, AudioResponseHeader, HandshakeRequest, HandshakeResponse, PositionInfo, PROTOCOL_VERSION};
use serde_json::Value;

use crate::chain::BUILTIN_GAIN_ID;
use crate::{restart_requested, serve_worker};

const COMMAND_WAIT: Duration = Duration::from_secs(30);

struct Session {
    server: Option<thread::JoinHandle<std::io::Result<()>>>,
    command: TcpStream,
    inbound: TcpStream,
    audio: TcpStream,
    screen: TcpStream,
}

impl Session {
    fn start(samples_per_block: i32, double_precision: bool) -> Self {
        Self::open(HandshakeRequest {
            samples_per_block,
            double_precision,
            ..HandshakeRequest::default()
        })
    }

    fn open(request: HandshakeRequest) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let mut handshake = TcpStream::connect(("127.0.0.1", port)).unwrap();
        handshake.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
        handshake.set_write_timeout(Some(COMMAND_WAIT)).unwrap();
        handshake.write_all(&request.to_bytes()).unwrap();
        let mut raw = [0u8; HandshakeResponse::WIRE_SIZE];
        handshake
            .read_exact(&mut raw)
            .unwrap_or_else(|err| panic!("handshake refused ({err}); double_precision={}", request.double_precision));
        let answer = HandshakeResponse::from_bytes(&raw).unwrap();
        assert_eq!(answer.version, PROTOCOL_VERSION, "protocol version");
        assert_ne!(answer.port as u16, port, "worker port must differ from the server port");
        drop(handshake);
        let mut sockets = Vec::new();
        for _ in 0..4 {
            let sock = TcpStream::connect(("127.0.0.1", answer.port as u16)).unwrap();
            sock.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
            sock.set_write_timeout(Some(COMMAND_WAIT)).unwrap();
            sockets.push(sock);
        }
        let mut command = sockets.remove(0);
        let inbound = sockets.remove(0);
        let audio = sockets.remove(0);
        let screen = sockets.remove(0);
        let (ty, body) = read_cmd(&mut command);
        assert_eq!(ty, PLUGIN_LIST, "worker did not push the plugin list when the client connected");
        let packed = command::read_length_prefixed(&body).expect("connect-time plugin list");
        let catalog = decode_msgpack(packed).expect("connect-time plugin list msgpack");
        assert!(
            plugins_of(&catalog).iter().any(|plugin| plugin.get("id").and_then(|value| value.as_str()) == Some(BUILTIN_GAIN_ID)),
            "connect-time plugin list has no builtin gain: {catalog}"
        );
        Self { server: Some(server), command, inbound, audio, screen }
    }

    fn finish(mut self) {
        let _ = self.command.shutdown(Shutdown::Both);
        let _ = self.audio.shutdown(Shutdown::Both);
        let _ = self.screen.shutdown(Shutdown::Both);
        let _ = self.inbound.shutdown(Shutdown::Both);
        if let Some(server) = self.server.take() {
            server.join().unwrap().unwrap();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.command.shutdown(Shutdown::Both);
        let _ = self.audio.shutdown(Shutdown::Both);
        let _ = self.screen.shutdown(Shutdown::Both);
        let _ = self.inbound.shutdown(Shutdown::Both);
    }
}

fn write_cmd(socket: &mut TcpStream, message_type: i32, body: &[u8]) {
    socket.write_all(&command::encode(message_type, body)).unwrap();
}

fn read_cmd(socket: &mut TcpStream) -> (i32, Vec<u8>) {
    let mut header = [0u8; command::CMD_HEADER];
    socket.read_exact(&mut header).unwrap();
    let message_type = i32::from_le_bytes(header[0..4].try_into().unwrap());
    let size = i32::from_le_bytes(header[4..8].try_into().unwrap());
    assert!(size >= 0, "negative command size");
    let mut body = vec![0u8; size as usize];
    if size > 0 {
        socket.read_exact(&mut body).unwrap();
    }
    (message_type, body)
}

fn add_layout(session: &mut Session, id: &str, settings: &str, layout: &str) -> Value {
    let body = command::json_payload(&serde_json::json!({
        "id": id,
        "settings": settings,
        "layout": layout
    }));
    write_cmd(&mut session.command, ADD_PLUGIN, &body);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, ADD_PLUGIN_RESULT);
    let result = command::read_json_payload(&body).unwrap();
    assert_eq!(result["success"], true, "add {id} failed: {result}");
    let (ty, _) = read_cmd(&mut session.command);
    assert_eq!(ty, PRESETS);
    let (ty, _) = read_cmd(&mut session.command);
    assert_eq!(ty, PARAMETERS);
    result
}

fn add_id(session: &mut Session, id: &str, settings: &str) -> Value {
    let body = command::json_payload(&serde_json::json!({
        "id": id,
        "settings": settings,
        "layout": "Stereo"
    }));
    write_cmd(&mut session.command, ADD_PLUGIN, &body);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, ADD_PLUGIN_RESULT);
    let result = command::read_json_payload(&body).unwrap();
    assert_eq!(result["success"], true, "add {id} failed: {result}");
    let (ty, _) = read_cmd(&mut session.command);
    assert_eq!(ty, PRESETS);
    let (ty, params) = read_cmd(&mut session.command);
    assert_eq!(ty, PARAMETERS);
    let packed = command::read_length_prefixed(&params).unwrap();
    decode_msgpack(packed).unwrap_or(Value::Null)
}

fn plugin_list(session: &mut Session) -> Value {
    write_cmd(&mut session.command, PLUGIN_LIST, &[]);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, PLUGIN_LIST);
    let packed = command::read_length_prefixed(&body).expect("plugin list payload");
    decode_msgpack(packed).expect("plugin list msgpack")
}

fn pair(a: i32, b: i32) -> [u8; 8] {
    let mut body = [0u8; 8];
    body[..4].copy_from_slice(&a.to_le_bytes());
    body[4..].copy_from_slice(&b.to_le_bytes());
    body
}

fn get_param(session: &mut Session, index: i32, param: i32) -> f32 {
    let mut body = [0u8; 12];
    body[..4].copy_from_slice(&index.to_le_bytes());
    body[4..8].copy_from_slice(&param.to_le_bytes());
    write_cmd(&mut session.command, GET_PARAMETER_VALUE, &body);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, PARAMETER_VALUE);
    command::read_parameter_value(&body).unwrap().2
}

fn exchange_f32(session: &mut Session, channels: &[Vec<f32>], midi: &[(i32, &[u8])]) -> (AudioResponseHeader, Vec<Vec<f32>>) {
    exchange_f32_at(session, channels, midi, PositionInfo::default())
}

fn write_f32_block(session: &mut Session, channels: &[Vec<f32>], midi: &[(i32, &[u8])], position: PositionInfo) {
    let samples = channels[0].len() as i32;
    let header = AudioRequestHeader {
        channels: channels.len() as i32,
        samples,
        channels_requested: channels.len() as i32,
        samples_requested: samples,
        midi_events: midi.len() as i32,
        is_double: false,
        trace_id: [0; 16],
    };
    let mut out = Vec::new();
    out.extend_from_slice(&header.to_bytes());
    for channel in channels {
        for sample in channel {
            out.extend_from_slice(&sample.to_le_bytes());
        }
    }
    for (sample_number, data) in midi {
        out.extend_from_slice(&sample_number.to_le_bytes());
        out.extend_from_slice(&(data.len() as i32).to_le_bytes());
        out.extend_from_slice(data);
    }
    out.extend_from_slice(&position.to_bytes());
    session.audio.write_all(&out).unwrap();
}

fn exchange_f32_at(
    session: &mut Session,
    channels: &[Vec<f32>],
    midi: &[(i32, &[u8])],
    position: PositionInfo,
) -> (AudioResponseHeader, Vec<Vec<f32>>) {
    write_f32_block(session, channels, midi, position);
    let samples = channels[0].len() as i32;
    let mut response = vec![0u8; AudioResponseHeader::WIRE_SIZE + channels.len() * samples as usize * 4];
    session.audio.read_exact(&mut response).unwrap();
    let header = AudioResponseHeader::from_bytes(&response).unwrap();
    let mut audio = vec![vec![0.0f32; samples as usize]; channels.len()];
    for (ch, channel) in audio.iter_mut().enumerate() {
        for (i, sample) in channel.iter_mut().enumerate() {
            let at = AudioResponseHeader::WIRE_SIZE + (ch * samples as usize + i) * 4;
            *sample = f32::from_le_bytes(response[at..at + 4].try_into().unwrap());
        }
    }
    (header, audio)
}

fn lin(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

fn max_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

fn plugins_of(list: &Value) -> Vec<&Value> {
    list.get("plugins").and_then(|v| v.as_array()).map(|items| items.iter().collect()).unwrap_or_default()
}

fn prefer_effect<'a>(plugins: &[&'a Value]) -> Option<&'a Value> {
    let mut best: Option<&Value> = None;
    let mut best_rank = i32::MAX;
    for plugin in plugins {
        let format = plugin.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let instrument = plugin.get("isInstrument").and_then(|v| v.as_bool()).unwrap_or(false);
        if instrument || format == "Builtin" || format.is_empty() {
            continue;
        }
        if !matches!(format, "AudioUnit" | "VST3" | "LV2" | "VST2") {
            continue;
        }
        let name = plugin.get("name").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
        let rank = if name.contains("lowpass") || name.contains("low pass") || name.contains("delay") || name.contains("limiter") || name.contains("filter") {
            0
        } else if name.contains("eq") || name.contains("compress") || name.contains("distort") || name.contains("reverb") {
            1
        } else {
            2
        };
        if rank < best_rank {
            best_rank = rank;
            best = Some(*plugin);
        }
    }
    best
}

fn prefer_instrument<'a>(plugins: &[&'a Value]) -> Option<&'a Value> {
    plugins.iter().copied().find(|plugin| {
        plugin.get("isInstrument").and_then(|v| v.as_bool()) == Some(true)
            && plugin.get("name").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase().contains("dls")
    }).or_else(|| {
        plugins.iter().copied().find(|plugin| plugin.get("isInstrument").and_then(|v| v.as_bool()) == Some(true))
    })
}

#[test]
fn multi_slot_chain_bypasses_reorders_and_roundtrips_a_parameter() {
    let mut session = Session::start(32, false);
    add_id(&mut session, BUILTIN_GAIN_ID, "-6");
    add_id(&mut session, BUILTIN_GAIN_ID, "0");
    let block = vec![vec![1.0f32; 32], vec![1.0; 32]];
    let (_header, audio) = exchange_f32(&mut session, &block, &[]);
    assert!((audio[0][0] - lin(-6.0)).abs() < 1.0e-4, "two slots should be -6 dB then 0 dB, got {}", audio[0][0]);

    write_cmd(&mut session.command, BYPASS_PLUGIN, &command::number(0));
    let _ = get_param(&mut session, 1, 0);
    let (_header, bypassed) = exchange_f32(&mut session, &block, &[]);
    assert!((bypassed[0][0] - 1.0).abs() < 1.0e-4, "bypassing the -6 dB slot should leave unity, got {}", bypassed[0][0]);

    write_cmd(&mut session.command, UNBYPASS_PLUGIN, &command::number(0));
    write_cmd(&mut session.command, EXCHANGE_PLUGINS, &pair(0, 1));
    let first = get_param(&mut session, 0, 0);
    let second = get_param(&mut session, 1, 0);
    assert!((first - 0.5).abs() < 1.0e-5, "reorder should put the 0 dB slot first, got {first}");
    assert!((second - 0.375).abs() < 1.0e-5, "reorder should put the -6 dB slot second, got {second}");

    write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, 0, 0.25, 0));
    let read_back = get_param(&mut session, 0, 0);
    assert!((read_back - 0.25).abs() < 1.0e-5, "parameter round-trip, got {read_back}");
    let (_header, after) = exchange_f32(&mut session, &block, &[]);
    let expected = lin(-12.0) * lin(-6.0);
    assert!((after[0][8] - expected).abs() < 1.0e-4, "parameter must change the next block, got {} want {expected}", after[0][8]);
    assert!((after[1][8] - expected).abs() < 1.0e-4, "both channels");
    session.finish();
}

#[test]
fn oversampling_changes_the_impulse_and_the_latency() {
    let mut session = Session::start(128, false);
    add_id(&mut session, BUILTIN_GAIN_ID, "0");
    let mut impulse = vec![0.0f32; 128];
    impulse[0] = 1.0;
    let channels = vec![impulse.clone(), impulse.clone()];

    let render = |session: &mut Session, factor: i32| {
        write_cmd(&mut session.command, SET_PLUGIN_OVERSAMPLING, &pair(0, factor));
        let _ = get_param(session, 0, 0);
        exchange_f32(session, &channels, &[])
    };
    let (h1, y1) = render(&mut session, 1);
    let (h2, y2) = render(&mut session, 2);
    let (h4, y4) = render(&mut session, 4);
    let diff2 = max_abs_diff(&y1[0], &y2[0]);
    let diff4 = max_abs_diff(&y1[0], &y4[0]);
    assert!(diff2 > 0.05, "2x oversampling left the impulse unchanged (max diff {diff2})");
    assert!(h2.latency_samples > h1.latency_samples, "2x latency {} did not rise above {}", h2.latency_samples, h1.latency_samples);
    assert!(diff4 > 0.05 || h4.latency_samples > h1.latency_samples, "4x oversampling changed neither level nor latency");
    assert!(diff4 > 0.02 || h4.latency_samples != h2.latency_samples, "4x matched 2x");
    session.finish();
}

#[test]
fn double_precision_block_keeps_a_value_f32_cannot_hold() {
    let mut session = Session::start(8, true);
    add_id(&mut session, BUILTIN_GAIN_ID, "0");
    let sample = 1.0f64 + f64::from(f32::EPSILON) / 2.0;
    assert_ne!(sample as f32 as f64, sample, "chosen sample must not survive an f32 round trip");
    let header = AudioRequestHeader {
        channels: 2,
        samples: 8,
        channels_requested: 2,
        samples_requested: 8,
        midi_events: 0,
        is_double: true,
        trace_id: [0; 16],
    };
    let mut out = Vec::new();
    out.extend_from_slice(&header.to_bytes());
    for _channel in 0..2 {
        for _ in 0..8 {
            out.extend_from_slice(&sample.to_le_bytes());
        }
    }
    out.extend_from_slice(&PositionInfo::default().to_bytes());
    session.audio.write_all(&out).unwrap();
    let mut response = vec![0u8; AudioResponseHeader::WIRE_SIZE + 2 * 8 * 8];
    session.audio.read_exact(&mut response).expect("64-bit float block was refused");
    let header = AudioResponseHeader::from_bytes(&response).unwrap();
    assert_eq!(header.channels, 2);
    assert_eq!(header.samples, 8);
    let mut got = [0u8; 8];
    got.copy_from_slice(&response[AudioResponseHeader::WIRE_SIZE..AudioResponseHeader::WIRE_SIZE + 8]);
    let value = f64::from_le_bytes(got);
    assert!((value - sample).abs() < 1.0e-12, "f64 processing collapsed {sample} to {value}");
    assert!((value - 1.0).abs() > 1.0e-9, "value was rounded to f32 1.0");
    session.finish();
}

/// AudioGridder sizes the process buffer to the greater of the sent shape and
/// `channelsRequested` / `samplesRequested`. The socket still carries only the
/// sent channels and samples. The extra channel and the extra samples stay 0.
#[test]
fn requested_channels_and_samples_widen_the_processed_block() {
    let mut session = Session::start(32, false);
    add_id(&mut session, BUILTIN_GAIN_ID, "0");
    let header = AudioRequestHeader {
        channels: 1,
        samples: 4,
        channels_requested: 2,
        samples_requested: 8,
        midi_events: 0,
        is_double: false,
        trace_id: [0; 16],
    };
    let mut out = Vec::new();
    out.extend_from_slice(&header.to_bytes());
    for _ in 0..4 {
        out.extend_from_slice(&1.0f32.to_le_bytes());
    }
    out.extend_from_slice(&PositionInfo::default().to_bytes());
    session.audio.write_all(&out).unwrap();
    let mut raw = [0u8; AudioResponseHeader::WIRE_SIZE];
    session.audio.read_exact(&mut raw).expect("audio response header");
    let response = AudioResponseHeader::from_bytes(&raw).unwrap();
    eprintln!("response channels {} samples {}", response.channels, response.samples);
    assert_eq!(response.channels, 2, "channelsRequested did not widen the block");
    assert_eq!(response.samples, 8, "samplesRequested did not lengthen the block");
    let mut body = vec![0u8; 2 * 8 * 4];
    session.audio.read_exact(&mut body).unwrap();
    let sample = |channel: usize, index: usize| {
        let at = (channel * 8 + index) * 4;
        f32::from_le_bytes(body[at..at + 4].try_into().unwrap())
    };
    eprintln!("left0 {} left7 {} right0 {}", sample(0, 0), sample(0, 7), sample(1, 0));
    assert!((sample(0, 0) - 1.0).abs() < 1.0e-5, "sent sample {}", sample(0, 0));
    assert!(sample(0, 7).abs() < 1.0e-6, "extra sample {}", sample(0, 7));
    assert!(sample(1, 0).abs() < 1.0e-6 && sample(1, 7).abs() < 1.0e-6, "extra channel {}", sample(1, 0));
    session.finish();
}

/// `AudioWorker::processBlock` widens a buffer that is narrower than
/// `max(channelsIn, channelsOut)`. With no active input bit the proc buffer
/// is cleared and the reverse map clears the caller's block. Input bit 1 and
/// output bit 33 (offset 32) pack that one channel onto chain channel 1.
/// Two active input bits and a one-channel block close the audio socket.
#[test]
fn narrow_buffer_follows_the_active_channel_map() {
    let mono = vec![vec![1.0f32; 8]];
    let mut silent = Session::open(HandshakeRequest {
        samples_per_block: 8,
        active_channels: 0,
        ..HandshakeRequest::default()
    });
    add_id(&mut silent, BUILTIN_GAIN_ID, "0");
    let (_header, cleared) = exchange_f32(&mut silent, &mono, &[]);
    eprintln!("inactive map {}", cleared[0][0]);
    assert!(
        cleared[0].iter().all(|sample| sample.abs() < 1.0e-6),
        "no active input should clear the narrow block, got {}",
        cleared[0][0]
    );
    silent.finish();

    // ChannelSet output offset is PLUGIN_CHANNELS_MAX / 2 when the chain has an input.
    let channel_one = (1u64 << 1) | (1u64 << 33);
    let mut mapped = Session::open(HandshakeRequest {
        samples_per_block: 8,
        active_channels: channel_one,
        ..HandshakeRequest::default()
    });
    let added = add_layout(&mut mapped, BUILTIN_GAIN_ID, "-6|6", "Multi-Mono");
    assert_eq!(added["channelInstances"], 2, "multi-mono did not build both channels: {added}");
    let up = lin(6.0);
    let down = lin(-6.0);
    let (_header, packed) = exchange_f32(&mut mapped, &mono, &[]);
    eprintln!("mapped channel 1 {}", packed[0][0]);
    assert!(
        (packed[0][0] - up).abs() < 1.0e-4,
        "active input 1 should return the +6 dB instance, got {} want {up}",
        packed[0][0]
    );
    let stereo = vec![vec![1.0f32; 8], vec![1.0f32; 8]];
    let (_header, wide) = exchange_f32(&mut mapped, &stereo, &[]);
    eprintln!("wide left {} right {}", wide[0][0], wide[1][0]);
    assert!((wide[0][0] - down).abs() < 1.0e-4, "a full-width block stays in place, left {}", wide[0][0]);
    assert!((wide[1][0] - up).abs() < 1.0e-4, "a full-width block stays in place, right {}", wide[1][0]);
    mapped.finish();

    let both_inputs = (1u64 << 0) | (1u64 << 1);
    let mut closed = Session::open(HandshakeRequest {
        samples_per_block: 8,
        active_channels: both_inputs,
        ..HandshakeRequest::default()
    });
    add_id(&mut closed, BUILTIN_GAIN_ID, "0");
    write_f32_block(&mut closed, &mono, &[], PositionInfo::default());
    let mut response = [0u8; 16];
    let read = closed.audio.read(&mut response);
    eprintln!("narrow active inputs {read:?}");
    let refused = match read {
        Ok(0) => true,
        Err(_) => true,
        Ok(_) => false,
    };
    assert!(refused, "two active inputs on a one-channel block should close the audio socket, got {read:?}");
    closed.finish();
}

#[test]
fn scanned_third_party_effect_is_not_a_copy_of_the_input() {
    let mut session = Session::start(256, false);
    let list = plugin_list(&mut session);
    let plugins = plugins_of(&list);
    let effect = prefer_effect(&plugins).unwrap_or_else(|| {
        panic!("plugin list has no AudioUnit, VST3, LV2, or VST2 effect: {list}");
    });
    let id = effect["id"].as_str().unwrap();
    let format = effect["type"].as_str().unwrap();
    assert_ne!(id, BUILTIN_GAIN_ID);
    assert_ne!(format, "Builtin");
    let params = add_id(&mut session, id, "");
    let count = params.as_array().map(|items| items.len()).unwrap_or(0);
    assert!(count > 0, "scanned effect published no parameters: {params}");

    let signal: Vec<f32> = (0..256).map(|i| (i as f32 * 0.17).sin() * 0.8).collect();
    let input = vec![signal.clone(), signal.clone()];
    let (_header, dry) = exchange_f32(&mut session, &input, &[]);
    let mut changed_param = false;
    let mut heard = max_abs_diff(&dry[0], &signal) > 1.0e-3;
    for index in 0..count.min(8) as i32 {
        write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, index, 0.0, 0));
        let _ = get_param(&mut session, 0, index);
        let (_header, low) = exchange_f32(&mut session, &input, &[]);
        write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, index, 1.0, 0));
        let read_back = get_param(&mut session, 0, index);
        let (_header, high) = exchange_f32(&mut session, &input, &[]);
        if (read_back - 0.0).abs() > 0.2 && max_abs_diff(&low[0], &high[0]) > 1.0e-3 {
            changed_param = true;
            heard = true;
            break;
        }
        if max_abs_diff(&high[0], &signal) > 1.0e-3 || max_abs_diff(&low[0], &signal) > 1.0e-3 {
            heard = true;
        }
    }
    assert!(heard, "scanned {format} effect `{id}` returned a copy of the input");
    assert!(changed_param, "no parameter of `{id}` changed the next audio block");
    session.finish();
}

#[test]
fn lv2_effect_is_not_a_copy_of_the_input() {
    hosted_format_changes_the_next_block("LV2");
}

#[test]
fn vst3_effect_is_not_a_copy_of_the_input() {
    hosted_format_changes_the_next_block("VST3");
}

fn hosted_format_changes_the_next_block(format: &str) {
    let mut session = Session::start(256, false);
    let list = plugin_list(&mut session);
    // Prefer OpenGrid's own plugins: the full system catalog leads with third-party
    // plugins that pass audio through unchanged until they are authorised.
    let effects: Vec<_> = plugins_of(&list).into_iter().filter(|plugin| {
        plugin.get("type").and_then(|value| value.as_str()) == Some(format)
            && plugin.get("isInstrument").and_then(|value| value.as_bool()) != Some(true)
    }).collect();
    let effect = effects
        .iter()
        .find(|plugin| plugin.get("company").and_then(|value| value.as_str()) == Some("OpenGrid"))
        .or_else(|| effects.first())
        .cloned()
        .unwrap_or_else(|| panic!("plugin list has no {format} effect: {list}"));
    let id = effect["id"].as_str().unwrap();
    assert_ne!(id, BUILTIN_GAIN_ID);
    let params = add_id(&mut session, id, "");
    let count = params.as_array().map(|items| items.len()).unwrap_or(0);
    assert!(count > 0, "{format} effect `{id}` published no parameters: {params}");
    let signal: Vec<f32> = (0..256).map(|i| (i as f32 * 0.17).sin() * 0.8).collect();
    let input = vec![signal.clone(), signal.clone()];
    let mut changed_param = false;
    let mut heard = false;
    for index in 0..count.min(12) as i32 {
        write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, index, 0.0, 0));
        let _ = get_param(&mut session, 0, index);
        let (_header, low) = exchange_f32(&mut session, &input, &[]);
        write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, index, 1.0, 0));
        let read_back = get_param(&mut session, 0, index);
        let (_header, high) = exchange_f32(&mut session, &input, &[]);
        if (read_back - 0.0).abs() > 0.2 && max_abs_diff(&low[0], &high[0]) > 1.0e-3 {
            changed_param = true;
            heard = true;
            break;
        }
        if max_abs_diff(&high[0], &signal) > 1.0e-3 || max_abs_diff(&low[0], &signal) > 1.0e-3 {
            heard = true;
        }
    }
    assert!(heard, "{format} effect `{id}` returned a copy of the input");
    assert!(changed_param, "no parameter of `{id}` changed the next audio block");
    session.finish();
}

#[test]
fn plugin_list_hides_instruments_until_the_filter_flag_is_set() {
    let mut filtered = Session::open(HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() });
    let hidden = plugin_list(&mut filtered);
    filtered.finish();
    let mut open = Session::open(HandshakeRequest {
        samples_per_block: 32,
        flags: command::FLAG_NO_PLUGINLIST_FILTER,
        ..HandshakeRequest::default()
    });
    let shown = plugin_list(&mut open);
    open.finish();

    let hidden_instruments = instrument_ids(&hidden);
    let shown_instruments = instrument_ids(&shown);
    let effects = plugins_of(&hidden).iter().any(|plugin| {
        plugin.get("type").and_then(|value| value.as_str()) == Some("AudioUnit")
            && plugin.get("isInstrument").and_then(|value| value.as_bool()) != Some(true)
    });
    eprintln!(
        "filtered instruments {} open instruments {} effects {effects}",
        hidden_instruments.len(),
        shown_instruments.len()
    );
    assert!(!shown_instruments.is_empty(), "unfiltered list has no instrument");
    assert!(hidden_instruments.is_empty(), "filtered list kept {}", hidden_instruments.join(","));
    assert!(
        plugins_of(&hidden).iter().any(|plugin| plugin.get("id").and_then(|value| value.as_str()) == Some(BUILTIN_GAIN_ID)),
        "filtered list dropped builtin gain"
    );
    assert!(effects, "filtered list dropped AudioUnit effects");
}

/// `Worker::handleMessage(PluginList)` keeps a layout when its output count
/// equals the session. Multi-Mono is added only when a mono layout exists and
/// the session has more than one output. A one-channel session lists Mono.
#[test]
fn plugin_list_layouts_follow_the_session_output_count() {
    let mut stereo = Session::start(32, false);
    let stereo_list = plugin_list(&mut stereo);
    let stereo_gain = layout_names(&stereo_list, BUILTIN_GAIN_ID);
    eprintln!("stereo layouts {stereo_gain:?}");
    assert_eq!(stereo_gain, vec!["Multi-Mono".to_string(), "Stereo".to_string()]);
    let stereo_effect = plugins_of(&stereo_list).into_iter().find(|plugin| {
        plugin.get("type").and_then(|value| value.as_str()) == Some("AudioUnit")
            && plugin.get("isInstrument").and_then(|value| value.as_bool()) != Some(true)
    });
    if let Some(effect) = stereo_effect {
        let name = effect.get("id").and_then(|value| value.as_str()).unwrap_or("");
        let layouts = layout_names_of(effect);
        eprintln!("stereo effect {name} layouts {layouts:?}");
        assert_eq!(layouts, vec!["Stereo".to_string()]);
    }
    stereo.finish();

    let mut mono = Session::open(HandshakeRequest {
        channels_in: 1,
        channels_out: 1,
        samples_per_block: 32,
        ..HandshakeRequest::default()
    });
    let mono_list = plugin_list(&mut mono);
    let mono_gain = layout_names(&mono_list, BUILTIN_GAIN_ID);
    eprintln!("mono layouts {mono_gain:?}");
    assert_eq!(mono_gain, vec!["Mono".to_string()]);
    let mono_effect = plugins_of(&mono_list).into_iter().find(|plugin| {
        plugin.get("type").and_then(|value| value.as_str()) == Some("AudioUnit")
            && plugin.get("isInstrument").and_then(|value| value.as_bool()) != Some(true)
    });
    if let Some(effect) = mono_effect {
        let name = effect.get("id").and_then(|value| value.as_str()).unwrap_or("");
        let layouts = layout_names_of(effect);
        eprintln!("mono effect {name} layouts {layouts:?}");
        assert_ne!(layouts, vec!["Stereo".to_string()], "mono session kept a stereo-only layout for {name}");
    }
    mono.finish();
}

fn layout_names(list: &Value, id: &str) -> Vec<String> {
    plugins_of(list)
        .into_iter()
        .find(|plugin| plugin.get("id").and_then(|value| value.as_str()) == Some(id))
        .map(layout_names_of)
        .unwrap_or_default()
}

fn layout_names_of(plugin: &Value) -> Vec<String> {
    plugin
        .get("layouts")
        .and_then(|value| value.as_array())
        .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn instrument_ids(list: &Value) -> Vec<String> {
    plugins_of(list)
        .into_iter()
        .filter(|plugin| plugin.get("isInstrument").and_then(|value| value.as_bool()) == Some(true))
        .filter_map(|plugin| plugin.get("id").and_then(|value| value.as_str()).map(str::to_string))
        .collect()
}

#[test]
fn midi_note_on_an_instrument_is_not_silent() {
    let mut session = Session::open(HandshakeRequest {
        samples_per_block: 256,
        flags: command::FLAG_NO_PLUGINLIST_FILTER,
        ..HandshakeRequest::default()
    });
    let list = plugin_list(&mut session);
    let instrument = prefer_instrument(&plugins_of(&list)).unwrap_or_else(|| {
        panic!("plugin list has no instrument: {list}");
    });
    let id = instrument["id"].as_str().unwrap();
    add_id(&mut session, id, "");
    let silence = vec![vec![0.0f32; 256], vec![0.0; 256]];
    let note = [0x90u8, 60, 100];
    let mut peak = 0.0f32;
    for block in 0..4 {
        let midi: &[(i32, &[u8])] = if block == 0 { &[(0, &note)] } else { &[] };
        let (_header, audio) = exchange_f32(&mut session, &silence, midi);
        peak = peak.max(audio.iter().flatten().copied().map(f32::abs).fold(0.0, f32::max));
    }
    assert!(peak > 1.0e-4, "MIDI note-on on `{id}` left the chain silent (peak {peak})");
    session.finish();
}

/// `AudioMessage::sendToClient` writes the MIDI buffer left after `processBlock`.
/// Builtin gain does not touch it, so the note comes back with its sample offset.
/// A hosted plugin consumes the input and this host has no MIDI output callback,
/// so that reply stays empty. Bypass skips the plugin, and the note comes back.
#[test]
fn processed_midi_returns_on_the_audio_response() {
    let mut session = Session::start(32, false);
    add_id(&mut session, BUILTIN_GAIN_ID, "0");
    let block = vec![vec![1.0f32; 4], vec![1.0; 4]];
    let note = [0x90u8, 60, 100];
    let (header, audio, events) = exchange_f32_midi(&mut session, &block, &[(7, &note)]);
    eprintln!(
        "gain midi channels {} samples {} events {} first {:?}",
        header.channels, header.samples, header.midi_events, events.first()
    );
    assert_eq!((header.channels, header.samples), (2, 4));
    assert!((audio[0][0] - 1.0).abs() < 1.0e-5, "gain audio {}", audio[0][0]);
    assert_eq!(header.midi_events, 1, "builtin gain dropped the MIDI buffer");
    assert_eq!(events, vec![(7, note)]);

    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| panic!("plugin list has no hosted effect: {list}"));
    add_id(&mut session, effect["id"].as_str().unwrap(), "");
    let (header, _audio, events) = exchange_f32_midi(&mut session, &block, &[(7, &note)]);
    eprintln!("plugin midi events {} body {}", header.midi_events, events.len());
    assert_eq!(header.midi_events, 0, "hosted plugin echoed consumed MIDI");
    assert!(events.is_empty());

    write_cmd(&mut session.command, BYPASS_PLUGIN, &command::number(1));
    let _ = get_param(&mut session, 0, 0);
    let (header, _audio, events) = exchange_f32_midi(&mut session, &block, &[(7, &note)]);
    eprintln!("bypassed plugin midi events {} first {:?}", header.midi_events, events.first());
    assert_eq!(header.midi_events, 1, "bypass ate the MIDI buffer");
    assert_eq!(events, vec![(7, note)]);
    session.finish();
}

fn exchange_f32_midi(
    session: &mut Session,
    channels: &[Vec<f32>],
    midi: &[(i32, &[u8])],
) -> (AudioResponseHeader, Vec<Vec<f32>>, Vec<(i32, [u8; 3])>) {
    write_f32_block(session, channels, midi, PositionInfo::default());
    let mut raw = [0u8; AudioResponseHeader::WIRE_SIZE];
    session.audio.read_exact(&mut raw).unwrap();
    let header = AudioResponseHeader::from_bytes(&raw).unwrap();
    let samples = header.samples.max(0) as usize;
    let width = header.channels.max(0) as usize;
    let mut body = vec![0u8; width * samples * 4];
    if !body.is_empty() {
        session.audio.read_exact(&mut body).unwrap();
    }
    let mut audio = vec![vec![0.0f32; samples]; width];
    for (ch, channel) in audio.iter_mut().enumerate() {
        for (i, sample) in channel.iter_mut().enumerate() {
            let at = (ch * samples + i) * 4;
            *sample = f32::from_le_bytes(body[at..at + 4].try_into().unwrap());
        }
    }
    let mut events = Vec::new();
    for _ in 0..header.midi_events.max(0) {
        let mut midi_header = [0u8; 8];
        session.audio.read_exact(&mut midi_header).unwrap();
        let sample = i32::from_le_bytes(midi_header[0..4].try_into().unwrap());
        let size = i32::from_le_bytes(midi_header[4..8].try_into().unwrap());
        assert!((0..=65_536).contains(&size), "midi size {size}");
        let mut data = vec![0u8; size as usize];
        if size > 0 {
            session.audio.read_exact(&mut data).unwrap();
        }
        let mut bytes = [0u8; 3];
        if data.len() >= 3 {
            bytes.copy_from_slice(&data[..3]);
        }
        events.push((sample, bytes));
    }
    (header, audio, events)
}

#[test]
fn editor_image_and_mouse_change_the_hosted_plugin() {
    let mut session = Session::start(64, false);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| {
        panic!("plugin list has no hosted effect to edit: {list}");
    });
    let id = effect["id"].as_str().unwrap();
    add_id(&mut session, id, "");
    write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, 0, 0.0, 0));
    let before = get_param(&mut session, 0, 0);
    eprintln!("editor parameter 0 before mouse {before}");
    write_cmd(&mut session.command, EDIT_PLUGIN, &command::edit_plugin(0, 0, 0, 0));
    session.screen.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (ty, body) = read_cmd(&mut session.screen);
    assert_eq!(ty, SCREEN_CAPTURE, "screen socket did not carry a capture");
    assert_decodable_image(&body);

    let mut mouse = [0u8; 28];
    mouse[0] = 1;
    mouse[4..8].copy_from_slice(&10.0f32.to_le_bytes());
    mouse[8..12].copy_from_slice(&10.0f32.to_le_bytes());
    write_cmd(&mut session.command, MOUSE, &mouse);
    let after = get_param(&mut session, 0, 0);
    eprintln!("editor parameter 0 after mouse {before} -> {after}");
    assert!((after - before).abs() > 0.2, "mouse did not change parameter 0 ({before} -> {after})");
    session.finish();
}

/// `ScreenWorker::runNative` with diff detection on. Capture 0 and every 20th
/// capture are a full PNG. Other captures are a PNG of the changed pixels, and
/// an unchanged capture is not sent. A mostly-black capture is not sent.
#[test]
fn screen_diff_sends_a_delta_a_full_refresh_and_resets_a_black_frame() {
    let mut session = Session::start(64, false);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| {
        panic!("plugin list has no hosted effect to edit: {list}");
    });
    let id = effect["id"].as_str().unwrap();
    let params = add_id(&mut session, id, "");
    let param_count = params.as_array().map(|items| items.len()).unwrap_or(0);
    assert!(param_count >= 1, "{id} published no parameters");
    write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, 0, 0.25, 0));
    write_cmd(&mut session.command, EDIT_PLUGIN, &command::edit_plugin(0, 0, 0, 0));
    let (width, height, full) = read_png_frame(&mut session);
    let opaque = opaque_pixels(&full);
    eprintln!("full refresh {width}x{height} opaque {opaque} bytes {}", full.len());
    assert_eq!((width, height), (64, 16));
    assert_eq!(opaque, full.len() / 4, "the first capture should be a full frame");

    write_cmd(&mut session.command, MOUSE, &mouse_at(0, 10.0, 10.0));
    expect_no_screen(&mut session);
    eprintln!("identical capture skipped");

    write_cmd(&mut session.command, MOUSE, &mouse_at(1, 10.0, 10.0));
    let (_width, _height, delta) = read_png_frame(&mut session);
    let delta_opaque = opaque_pixels(&delta);
    let rows = if param_count == 1 { height as usize } else { 1 };
    eprintln!("delta opaque {delta_opaque} param_count {param_count}");
    assert_eq!(delta_opaque, rows * width as usize, "delta should carry only the rows parameter 0 paints");
    assert!(delta_opaque < full.len() / 4, "delta was the whole frame");

    for _ in 0..17 {
        write_cmd(&mut session.command, MOUSE, &mouse_at(0, 10.0, 10.0));
        expect_no_screen(&mut session);
    }
    write_cmd(&mut session.command, MOUSE, &mouse_at(0, 10.0, 10.0));
    let (_width, _height, refresh) = read_png_frame(&mut session);
    let refresh_opaque = opaque_pixels(&refresh);
    eprintln!("periodic full refresh opaque {refresh_opaque}");
    assert_eq!(refresh_opaque, refresh.len() / 4, "capture 20 should be a full frame");

    for index in 0..param_count {
        write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, index as i32, 0.0, 0));
    }
    write_cmd(&mut session.command, MOUSE, &mouse_at(0, 10.0, 10.0));
    expect_no_screen(&mut session);
    eprintln!("black capture reset");

    for index in 0..param_count {
        write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, index as i32, 0.5, 0));
    }
    write_cmd(&mut session.command, MOUSE, &mouse_at(0, 10.0, 10.0));
    let (_width, _height, recovered) = read_png_frame(&mut session);
    let recovered_opaque = opaque_pixels(&recovered);
    eprintln!("recovered frame opaque {recovered_opaque}");
    assert_eq!(recovered_opaque, recovered.len() / 4, "the frame after a reset should be a full image");
    assert!(recovered.iter().step_by(4).any(|pixel| *pixel != 0), "recovered frame is still black");
    session.finish();
}

#[test]
fn key_on_the_open_editor_steps_a_parameter() {
    let mut session = Session::start(64, false);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| {
        panic!("plugin list has no hosted effect to edit: {list}");
    });
    let id = effect["id"].as_str().unwrap();
    add_id(&mut session, id, "");
    write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, 0, 0.0, 0));
    let before = get_param(&mut session, 0, 0);
    assert!(before < 0.05, "parameter 0 did not start at 0 ({before})");
    write_cmd(&mut session.command, EDIT_PLUGIN, &command::edit_plugin(0, 0, 0, 0));
    session.screen.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (ty, body) = read_cmd(&mut session.screen);
    assert_eq!(ty, SCREEN_CAPTURE, "screen socket did not carry a capture");
    assert_decodable_image(&body);

    write_cmd(&mut session.command, KEY, &command::length_prefixed(&0x7Eu16.to_le_bytes()));
    let stepped = get_param(&mut session, 0, 0);
    eprintln!("editor parameter 0 after up arrow {before} -> {stepped}");
    assert!((stepped - before).abs() > 0.2, "up arrow did not change parameter 0 ({before} -> {stepped})");
    assert!(stepped < 0.5, "up arrow jumped to {stepped}, expected one step from {before}");

    write_cmd(&mut session.command, KEY, &command::length_prefixed(&0x38u16.to_le_bytes()));
    let shifted = get_param(&mut session, 0, 0);
    assert!((shifted - stepped).abs() < 1.0e-4, "shift key changed parameter 0 ({stepped} -> {shifted})");

    write_cmd(&mut session.command, HIDE_PLUGIN, &command::number(0));
    write_cmd(&mut session.command, KEY, &command::length_prefixed(&0x7Eu16.to_le_bytes()));
    let hidden = get_param(&mut session, 0, 0);
    assert!((hidden - shifted).abs() < 1.0e-4, "key after hide changed parameter 0 ({shifted} -> {hidden})");
    session.finish();
}

/// `MouseEvType::WHEEL` is 10. `Client::mouseWheelMove` writes deltaX and deltaY
/// into the 28-byte mouse body. One notch is about 1.0 and steps the generic
/// editor by the same 0.25 as the up arrow. A wheel is not a button gesture,
/// and a closed editor ignores it. This is not an OS scroll event.
#[test]
fn wheel_on_the_open_editor_steps_a_parameter() {
    let mut session = Session::start(64, false);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| {
        panic!("plugin list has no hosted effect to edit: {list}");
    });
    add_id(&mut session, effect["id"].as_str().unwrap(), "");
    drain_ready_status(&mut session);
    write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, 0, 0.5, 0));
    let start = get_param(&mut session, 0, 0);
    eprintln!("wheel start {start}");
    assert!((start - 0.5).abs() < 1.0e-4, "parameter 0 did not start at 0.5 ({start})");
    write_cmd(&mut session.command, EDIT_PLUGIN, &command::edit_plugin(0, 0, 0, 0));
    session.screen.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (ty, body) = read_cmd(&mut session.screen);
    assert_eq!(ty, SCREEN_CAPTURE, "screen socket did not carry a capture");
    assert_decodable_image(&body);

    write_cmd(&mut session.command, MOUSE, &mouse_wheel(10.0, 10.0, 0.0, 1.0));
    let up = get_param(&mut session, 0, 0);
    eprintln!("wheel up {start} -> {up}");
    assert!((up - 0.75).abs() < 1.0e-4, "wheel up did not step parameter 0 by one notch ({start} -> {up})");
    session.inbound.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let (ty, body) = read_cmd(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert_eq!(ty, PARAMETER_VALUE, "wheel did not push the new value, got type {ty}");
    let (idx, param, value, channel) = command::read_parameter_value(&body).expect("parameter value");
    assert_eq!((idx, param, channel), (0, 0, 0));
    assert!((value - 0.75).abs() < 1.0e-4, "pushed value {value}");
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let gesture = read_cmd_result(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert!(gesture.is_err(), "wheel started a gesture: {gesture:?}");

    write_cmd(&mut session.command, MOUSE, &mouse_wheel(10.0, 10.0, 0.0, -1.0));
    let down = get_param(&mut session, 0, 0);
    eprintln!("wheel down {up} -> {down}");
    assert!((down - 0.5).abs() < 1.0e-4, "wheel down did not step back ({up} -> {down})");
    session.inbound.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let (ty, _) = read_cmd(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert_eq!(ty, PARAMETER_VALUE, "wheel down did not push, got type {ty}");

    write_cmd(&mut session.command, MOUSE, &mouse_wheel(10.0, 10.0, 1.0, 0.0));
    let across = get_param(&mut session, 0, 0);
    eprintln!("wheel across {down} -> {across}");
    assert!((across - 0.75).abs() < 1.0e-4, "horizontal wheel did not step parameter 0 ({down} -> {across})");
    session.inbound.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let (ty, _) = read_cmd(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert_eq!(ty, PARAMETER_VALUE);

    write_cmd(&mut session.command, MOUSE, &mouse_wheel(2.0, 2.0, 0.0, 0.0));
    let still = get_param(&mut session, 0, 0);
    eprintln!("wheel zero {across} -> {still}");
    assert!((still - across).abs() < 1.0e-4, "a zero wheel moved parameter 0 ({across} -> {still})");
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let idle = read_cmd_result(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert!(idle.is_err(), "a zero wheel pushed command-in: {idle:?}");
    write_cmd(&mut session.command, GET_SCREEN_BOUNDS, &[]);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, SCREEN_BOUNDS);
    let bounds = command::read_edit_plugin(&body).unwrap();
    assert!(bounds.2 > 0 && bounds.3 > 0, "wheel at the close box hid the editor: {bounds:?}");

    write_cmd(&mut session.command, HIDE_PLUGIN, &command::number(0));
    write_cmd(&mut session.command, MOUSE, &mouse_wheel(10.0, 10.0, 0.0, 1.0));
    let hidden = get_param(&mut session, 0, 0);
    eprintln!("wheel hidden {still} -> {hidden}");
    assert!((hidden - still).abs() < 1.0e-4, "wheel after hide changed parameter 0 ({still} -> {hidden})");
    session.finish();
}

#[test]
fn server_forwards_applied_key_on_command_in() {
    let mut session = Session::start(64, false);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| {
        panic!("plugin list has no hosted effect to edit: {list}");
    });
    add_id(&mut session, effect["id"].as_str().unwrap(), "");
    drain_ready_status(&mut session);
    write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, 0, 0.0, 0));
    assert!(get_param(&mut session, 0, 0) < 0.05, "parameter 0 did not start at 0");
    write_cmd(&mut session.command, EDIT_PLUGIN, &command::edit_plugin(0, 0, 0, 0));
    session.screen.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (ty, body) = read_cmd(&mut session.screen);
    assert_eq!(ty, SCREEN_CAPTURE);
    assert_decodable_image(&body);

    write_cmd(&mut session.command, KEY, &key_codes(&[0x38]));
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let shift_only = read_cmd_result(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert!(shift_only.is_err(), "shift alone was pushed on command-in: {shift_only:?}");

    write_cmd(&mut session.command, KEY, &key_codes(&[0x38, 0x7E]));
    session.inbound.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let (ty, body) = read_cmd(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert_eq!(ty, KEY, "command-in did not carry the applied key");
    let payload = command::read_length_prefixed(&body).expect("key payload");
    let codes: Vec<u16> = payload.chunks_exact(2).map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]])).collect();
    assert_eq!(codes, vec![0x7E], "command-in should carry the applied up arrow, got {codes:?}");
    let stepped = get_param(&mut session, 0, 0);
    eprintln!("command-in key codes {codes:?}, parameter 0 -> {stepped}");
    assert!((stepped - 0.25).abs() < 1.0e-4, "applied key did not step parameter 0 ({stepped})");
    session.finish();
}

/// `Worker::sendParamValueChange` writes ParameterValue on the client's command-in
/// socket when the server editor changes a value. A client ParameterValue write
/// is the request itself and must not be echoed.
#[test]
fn editor_mouse_pushes_parameter_value_on_command_in() {
    let mut session = Session::start(64, false);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| {
        panic!("plugin list has no hosted effect to edit: {list}");
    });
    add_id(&mut session, effect["id"].as_str().unwrap(), "");
    drain_ready_status(&mut session);
    write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, 0, 0.0, 0));
    assert!(get_param(&mut session, 0, 0) < 0.05, "parameter 0 did not start at 0");
    write_cmd(&mut session.command, EDIT_PLUGIN, &command::edit_plugin(0, 0, 0, 0));
    session.screen.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (ty, body) = read_cmd(&mut session.screen);
    assert_eq!(ty, SCREEN_CAPTURE);
    assert_decodable_image(&body);

    let mut mouse = [0u8; 28];
    mouse[4..8].copy_from_slice(&10.0f32.to_le_bytes());
    mouse[8..12].copy_from_slice(&10.0f32.to_le_bytes());
    write_cmd(&mut session.command, MOUSE, &mouse);
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let moved = read_cmd_result(&mut session.inbound);
    assert!(moved.is_err(), "a mouse move was pushed on command-in: {moved:?}");

    mouse[0] = 1;
    write_cmd(&mut session.command, MOUSE, &mouse);
    session.inbound.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let (ty, _) = read_cmd(&mut session.inbound);
    assert_eq!(ty, PARAMETER_GESTURE, "left-down did not start a gesture");
    let (ty, body) = read_cmd(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert_eq!(ty, PARAMETER_VALUE, "command-in did not carry the editor parameter change");
    let (idx, param, value, channel) = command::read_parameter_value(&body).expect("parameter value");
    eprintln!("command-in parameter value idx {idx} param {param} value {value} channel {channel}");
    assert_eq!(idx, 0);
    assert_eq!(param, 0);
    assert_eq!(channel, 0);
    assert!((value - 1.0).abs() < 1.0e-3, "editor push was {value}, expected the toggled value 1");
    assert!((get_param(&mut session, 0, 0) - value).abs() < 1.0e-3, "pushed value disagrees with the slot");

    write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, 0, 0.5, 0));
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let echoed = read_cmd_result(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert!(echoed.is_err(), "a client parameter write was echoed on command-in: {echoed:?}");
    let written = get_param(&mut session, 0, 0);
    assert!((written - 0.5).abs() < 1.0e-3, "client write did not land ({written})");
    session.finish();
}

/// `GetAllParameterValues` answers on the request socket with one ParameterValue
/// per hosted parameter, not a single parameter-0 message.
#[test]
fn get_all_parameter_values_lists_every_hosted_parameter() {
    let mut session = Session::start(64, false);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| {
        panic!("plugin list has no hosted effect: {list}");
    });
    let id = effect["id"].as_str().unwrap();
    let params = add_id(&mut session, id, "");
    let count = params.as_array().map(|items| items.len()).unwrap_or(0);
    assert!(count > 1, "{id} published {count} parameters; need more than parameter 0");
    write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, 0, 0.25, 0));
    write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, 1, 0.75, 0));
    write_cmd(&mut session.command, GET_ALL_PARAMETER_VALUES, &command::number(0));
    let mut seen = Vec::with_capacity(count);
    for _ in 0..count {
        let (ty, body) = read_cmd(&mut session.command);
        assert_eq!(ty, PARAMETER_VALUE, "GetAllParameterValues reply was type {ty}");
        let (idx, param, value, channel) = command::read_parameter_value(&body).expect("parameter value");
        assert_eq!(idx, 0);
        assert_eq!(channel, 0);
        seen.push((param, value));
    }
    session.command.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let extra = read_cmd_result(&mut session.command);
    session.command.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert!(extra.is_err(), "GetAllParameterValues sent a message past the parameter list: {extra:?}");
    let indices: Vec<i32> = seen.iter().map(|(param, _)| *param).collect();
    let expected: Vec<i32> = (0..count as i32).collect();
    assert_eq!(indices, expected, "parameter indexes from {id}: {indices:?}");
    let live0 = get_param(&mut session, 0, 0);
    let live1 = get_param(&mut session, 0, 1);
    eprintln!("get-all {id} count {count} param0 {live0} param1 {live1}");
    assert!((seen[0].1 - live0).abs() < 1.0e-3, "param 0 get-all {} != live {live0}", seen[0].1);
    assert!((seen[1].1 - live1).abs() < 1.0e-3, "param 1 get-all {} != live {live1}", seen[1].1);
    assert!((live0 - live1).abs() > 0.2, "param 0 and param 1 collapsed to the same value ({live0}, {live1})");
    session.finish();
}

/// `GetScreenBounds` answers on the request socket with the open editor's
/// width and height. Hiding the editor returns zeros. The frame on the screen
/// socket does not clear the bounds.
#[test]
fn open_editor_reports_screen_bounds() {
    let mut session = Session::start(64, false);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| {
        panic!("plugin list has no hosted effect: {list}");
    });
    add_id(&mut session, effect["id"].as_str().unwrap(), "");
    write_cmd(&mut session.command, GET_SCREEN_BOUNDS, &[]);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, SCREEN_BOUNDS);
    let closed = command::read_edit_plugin(&body).unwrap();
    assert_eq!(closed, (0, 0, 0, 0), "bounds before edit were {closed:?}");

    write_cmd(&mut session.command, EDIT_PLUGIN, &command::edit_plugin(0, 0, 0, 0));
    session.screen.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (ty, frame) = read_cmd(&mut session.screen);
    assert_eq!(ty, SCREEN_CAPTURE);
    assert_decodable_image(&frame);
    write_cmd(&mut session.command, GET_SCREEN_BOUNDS, &[]);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, SCREEN_BOUNDS);
    let open = command::read_edit_plugin(&body).unwrap();
    eprintln!("screen bounds while open {open:?}");
    assert_eq!(open.2, 64, "open editor width");
    assert_eq!(open.3, 16, "open editor height");

    write_cmd(&mut session.command, HIDE_PLUGIN, &command::number(0));
    write_cmd(&mut session.command, GET_SCREEN_BOUNDS, &[]);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, SCREEN_BOUNDS);
    let hidden = command::read_edit_plugin(&body).unwrap();
    assert_eq!(hidden, (0, 0, 0, 0), "bounds after hide were {hidden:?}");
    session.finish();
}

/// Clipboard text lands on the macOS pasteboard and a later pasteboard change
/// is pushed on command-in while the editor is open. The capture margin grows
/// the screen bounds, and fullscreen reports the main display.
#[test]
fn clipboard_and_capture_area_follow_the_open_editor() {
    let original = crate::clipboard::get_text().expect("pbpaste");
    let _restore = ClipboardRestore(original);
    let mut session = Session::start(64, false);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| panic!("plugin list has no hosted effect: {list}"));
    add_id(&mut session, effect["id"].as_str().unwrap(), "");
    write_cmd(&mut session.command, EDIT_PLUGIN, &command::edit_plugin(0, 0, 0, 0));
    session.screen.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (ty, frame) = read_cmd(&mut session.screen);
    assert_eq!(ty, SCREEN_CAPTURE);
    assert_decodable_image(&frame);

    let open = screen_bounds(&mut session);
    eprintln!("capture open {open:?}");
    assert_eq!((open.2, open.3), (64, 16));
    write_cmd(&mut session.command, UPDATE_SCREEN_CAPTURE_AREA, &command::number(30));
    let grown = screen_bounds(&mut session);
    eprintln!("capture plus 30 {grown:?}");
    assert_eq!((grown.2, grown.3), (94, 46));
    write_cmd(&mut session.command, UPDATE_SCREEN_CAPTURE_AREA, &command::number(0));
    assert_eq!(screen_bounds(&mut session), grown, "a zero nudge must leave the margin");
    write_cmd(&mut session.command, UPDATE_SCREEN_CAPTURE_AREA, &command::number(30));
    let wider = screen_bounds(&mut session);
    assert_eq!((wider.2, wider.3), (124, 76));
    write_cmd(&mut session.command, UPDATE_SCREEN_CAPTURE_AREA, &command::number(0xFFFF));
    let fullscreen = screen_bounds(&mut session);
    let display = crate::clipboard::main_display();
    eprintln!("capture fullscreen {fullscreen:?} display {display:?}");
    assert_eq!((fullscreen.2, fullscreen.3), display);
    assert!(fullscreen.2 > 64 && fullscreen.3 > 16, "fullscreen stayed on the editor");
    write_cmd(&mut session.command, UPDATE_SCREEN_CAPTURE_AREA, &command::number(0xFFFF));
    assert_eq!(screen_bounds(&mut session), wider, "leaving fullscreen restores the margin");
    write_cmd(&mut session.command, UPDATE_SCREEN_CAPTURE_AREA, &command::number(-90));
    assert_eq!((screen_bounds(&mut session).2, screen_bounds(&mut session).3), (64, 16));
    write_cmd(&mut session.command, UPDATE_SCREEN_CAPTURE_AREA, &command::number(-30));
    assert_eq!((screen_bounds(&mut session).2, screen_bounds(&mut session).3), (64, 16), "margin floors at 0");

    // The watcher arms on its first poll after the editor opens.
    thread::sleep(Duration::from_millis(500));
    let token = format!("ag-open-clip-{}", std::process::id());
    let next = format!("ag-open-clip-next-{}", std::process::id());
    write_cmd(&mut session.command, CLIPBOARD, &command::length_prefixed(token.as_bytes()));
    let _ = screen_bounds(&mut session);
    let pasted = crate::clipboard::get_text().expect("pbpaste after set");
    eprintln!("pasted {pasted}");
    assert_eq!(pasted, token);
    assert!(wait_clipboard(&mut session, &token, Duration::from_secs(2)), "client clipboard was not pushed");
    crate::clipboard::set_text(&next).expect("pbcopy");
    assert!(wait_clipboard(&mut session, &next, Duration::from_secs(2)), "pasteboard change was not pushed");

    write_cmd(&mut session.command, HIDE_PLUGIN, &command::number(0));
    thread::sleep(Duration::from_millis(400));
    let hidden = format!("ag-open-clip-hidden-{}", std::process::id());
    crate::clipboard::set_text(&hidden).expect("pbcopy while hidden");
    assert!(!wait_clipboard(&mut session, &hidden, Duration::from_millis(700)), "hidden editor still pushed the pasteboard");
    session.finish();
}

struct ClipboardRestore(String);

impl Drop for ClipboardRestore {
    fn drop(&mut self) {
        let _ = crate::clipboard::set_text(&self.0);
    }
}

fn screen_bounds(session: &mut Session) -> (i32, i32, i32, i32) {
    write_cmd(&mut session.command, GET_SCREEN_BOUNDS, &[]);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, SCREEN_BOUNDS);
    command::read_edit_plugin(&body).expect("screen bounds")
}

fn wait_clipboard(session: &mut Session, expected: &str, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    session.inbound.set_read_timeout(Some(Duration::from_millis(250))).unwrap();
    let mut found = false;
    while start.elapsed() < timeout {
        match read_cmd_result(&mut session.inbound) {
            Ok((ty, body)) if ty == CLIPBOARD => {
                let text = command::read_length_prefixed(&body).map(|bytes| String::from_utf8_lossy(bytes).into_owned()).unwrap_or_default();
                eprintln!("clipboard push {text}");
                if text == expected {
                    found = true;
                    break;
                }
            }
            Ok((ty, _)) => eprintln!("inbound while waiting for clipboard {ty}"),
            Err(_) => {}
        }
    }
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    found
}

/// `Worker::sendParamGestureChange` brackets the editor's value push.
/// Left-down starts the gesture, the value follows, and left-up ends it.
#[test]
fn editor_mouse_pushes_a_gesture_around_the_value() {
    let mut session = Session::start(64, false);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| {
        panic!("plugin list has no hosted effect to edit: {list}");
    });
    add_id(&mut session, effect["id"].as_str().unwrap(), "");
    write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, 0, 0.0, 0));
    write_cmd(&mut session.command, EDIT_PLUGIN, &command::edit_plugin(0, 0, 0, 0));
    session.screen.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (ty, body) = read_cmd(&mut session.screen);
    assert_eq!(ty, SCREEN_CAPTURE);
    assert_decodable_image(&body);
    drain_ready_status(&mut session);

    let mut mouse = mouse_at(0, 10.0, 10.0);
    write_cmd(&mut session.command, MOUSE, &mouse);
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let moved = read_cmd_result(&mut session.inbound);
    assert!(moved.is_err(), "a mouse move was pushed on command-in: {moved:?}");

    mouse[0] = 1;
    write_cmd(&mut session.command, MOUSE, &mouse);
    session.inbound.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let (ty, body) = read_cmd(&mut session.inbound);
    assert_eq!(ty, PARAMETER_GESTURE, "left-down did not start a gesture, got type {ty}");
    let (idx, param, starting, channel) = read_gesture(&body);
    assert_eq!((idx, param, starting, channel), (0, 0, true, 0));
    let (ty, body) = read_cmd(&mut session.inbound);
    assert_eq!(ty, PARAMETER_VALUE, "gesture was not followed by the new value");
    let (idx, param, value, channel) = command::read_parameter_value(&body).unwrap();
    eprintln!("gesture start then value {value} idx {idx} param {param} channel {channel}");
    assert_eq!((idx, param, channel), (0, 0, 0));
    assert!((value - 1.0).abs() < 1.0e-3, "gesture value was {value}");

    mouse[0] = 2;
    write_cmd(&mut session.command, MOUSE, &mouse);
    let (ty, body) = read_cmd(&mut session.inbound);
    assert_eq!(ty, PARAMETER_GESTURE, "left-up did not end the gesture, got type {ty}");
    let end = read_gesture(&body);
    assert_eq!(end, (0, 0, false, 0), "gesture end was {end:?}");
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let extra = read_cmd_result(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert!(extra.is_err(), "left-up pushed more than the gesture end: {extra:?}");
    session.finish();
}

/// `Worker::sendStatusChange` writes PluginStatus on command-in when a plugin
/// is ready. A failed add stays on the request socket and does not push.
#[test]
fn loaded_plugin_pushes_status_on_command_in() {
    let mut session = Session::start(64, false);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| {
        panic!("plugin list has no hosted effect: {list}");
    });
    let id = effect["id"].as_str().unwrap();
    add_id(&mut session, id, "");
    session.inbound.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let (ty, body) = read_cmd(&mut session.inbound);
    assert_eq!(ty, PLUGIN_STATUS, "command-in did not carry plugin status");
    let status = command::read_json_payload(&body).expect("status json");
    eprintln!("plugin status {status}");
    assert_eq!(status["idx"], 0);
    assert_eq!(status["ok"], true);
    assert_eq!(status["err"], "");

    let missing = command::json_payload(&serde_json::json!({"id": "missing-plugin", "settings": "", "layout": "Stereo"}));
    write_cmd(&mut session.command, ADD_PLUGIN, &missing);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, ADD_PLUGIN_RESULT);
    let result = command::read_json_payload(&body).unwrap();
    assert_eq!(result["success"], false, "unknown id was accepted: {result}");
    assert!(result["err"].as_str().unwrap_or("").contains("missing-plugin"), "err was {result}");
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let pushed = read_cmd_result(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert!(pushed.is_err(), "a failed add pushed on command-in: {pushed:?}");
    session.finish();
}

/// Unbypass sends no reply, and the next block is the slot's audio again.
#[test]
fn unbypass_restores_the_bypassed_gain() {
    let mut session = Session::start(32, false);
    add_id(&mut session, BUILTIN_GAIN_ID, "-6");
    let block = vec![vec![1.0f32; 32], vec![1.0; 32]];
    let (_header, wet) = exchange_f32(&mut session, &block, &[]);
    assert!((wet[0][0] - lin(-6.0)).abs() < 1.0e-4, "slot should be -6 dB, got {}", wet[0][0]);

    write_cmd(&mut session.command, BYPASS_PLUGIN, &command::number(0));
    assert_no_reply(&mut session);
    let (_header, bypassed) = exchange_f32(&mut session, &block, &[]);
    assert!((bypassed[0][0] - 1.0).abs() < 1.0e-4, "bypass should be unity, got {}", bypassed[0][0]);

    write_cmd(&mut session.command, UNBYPASS_PLUGIN, &command::number(0));
    assert_no_reply(&mut session);
    let (_header, restored) = exchange_f32(&mut session, &block, &[]);
    assert!((restored[0][0] - lin(-6.0)).abs() < 1.0e-4, "unbypass should restore -6 dB, got {}", restored[0][0]);
    session.finish();
}

/// `Rescan` closes the worker and drops the catalog. Number 1 records a wipe.
/// `Restart` closes the worker and leaves the catalog generation where it is.
#[test]
fn rescan_and_restart_control_the_catalog() {
    let _ = crate::host::scan();
    let warm = crate::host::scan_generation();
    let _ = crate::host::scan();
    assert_eq!(crate::host::scan_generation(), warm, "a warm scan rebuilt the catalog");
    assert!(!restart_requested(&std::io::Error::new(std::io::ErrorKind::Interrupted, "scan")));
    assert!(!restart_requested(&std::io::Error::other("restart")));

    let mut session = Session::start(32, false);
    let before_wipe = crate::host::scan_generation();
    write_cmd(&mut session.command, RESCAN, &command::number(1));
    join_restart(&mut session, "restart-rescan-wipe");
    let _ = crate::host::scan();
    assert_eq!(crate::host::scan_generation(), before_wipe + 1, "wipe rescan did not rebuild on the next scan");

    let mut session = Session::start(32, false);
    let before_scan = crate::host::scan_generation();
    write_cmd(&mut session.command, RESCAN, &command::number(0));
    join_restart(&mut session, "restart-rescan");
    let _ = crate::host::scan();
    assert_eq!(crate::host::scan_generation(), before_scan + 1, "rescan did not rebuild on the next scan");

    let mut session = Session::start(32, false);
    let before_restart = crate::host::scan_generation();
    write_cmd(&mut session.command, RESTART, &[]);
    join_restart(&mut session, "restart");
    let _ = crate::host::scan();
    assert_eq!(
        crate::host::scan_generation(),
        before_restart,
        "restart cleared the catalog: generation moved from {before_restart}"
    );
}

/// `CPULoad` is the host percent, not a constant 0.
#[test]
fn cpu_load_reports_host_percent() {
    let mut session = Session::start(32, false);
    let cores = std::thread::available_parallelism().map(|count| count.get()).unwrap_or(1);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let burn: Vec<_> = (0..cores)
        .map(|_| {
            let stop = std::sync::Arc::clone(&stop);
            thread::spawn(move || {
                let mut state = 0x1234_5678_9abc_def0u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    std::hint::black_box(state);
                }
            })
        })
        .collect();
    thread::sleep(Duration::from_millis(1600));
    write_cmd(&mut session.command, CPU_LOAD, &[]);
    let (ty, body) = read_cmd(&mut session.command);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for handle in burn {
        handle.join().unwrap();
    }
    assert_eq!(ty, CPU_LOAD, "cpu reply type");
    let value = command::read_float(&body).unwrap_or_else(|| panic!("cpu body {body:?}"));
    eprintln!("cpu load {value} percent");
    assert!(value.is_finite(), "cpu load {value}");
    assert!(value > 0.5, "host cpu load stayed at {value}");
    assert!(value <= 100.0, "cpu load {value} is outside 0..=100");
    session.finish();
}

/// `Any` (type 0) is a wildcard read. The worker consumes it and sends nothing.
#[test]
fn any_message_gets_no_reply_and_the_next_command_still_answers() {
    let mut session = Session::start(32, false);
    write_cmd(&mut session.command, command::ANY, &[]);
    session.command.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let idle = read_cmd_result(&mut session.command);
    eprintln!("any reply {idle:?}");
    assert!(
        matches!(idle.as_ref().map(|_| ()).map_err(|err| err.kind()), Err(std::io::ErrorKind::TimedOut) | Err(std::io::ErrorKind::WouldBlock)),
        "type 0 replied {idle:?}"
    );
    session.command.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    write_cmd(&mut session.command, CPU_LOAD, &[]);
    let (ty, body) = read_cmd(&mut session.command);
    let value = command::read_float(&body).unwrap_or_else(|| panic!("cpu body {body:?}"));
    eprintln!("any then cpu {ty} {value}");
    assert_eq!(ty, CPU_LOAD, "the command after type 0 was {ty}");
    assert!(value.is_finite() && (0.0..=100.0).contains(&value), "cpu {value}");
    session.finish();
}

/// An unknown command is logged and not answered. Type 3 is not in Message.hpp.
/// AudioGridder does not send ServerError for it on the request socket or on command-in.
#[test]
fn unknown_command_gets_no_reply_and_the_next_command_still_answers() {
    let mut session = Session::start(32, false);
    write_cmd(&mut session.command, 3, &[]);
    session.command.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let idle = read_cmd_result(&mut session.command);
    eprintln!("unknown reply {idle:?}");
    assert!(
        matches!(idle.as_ref().map(|_| ()).map_err(|err| err.kind()), Err(std::io::ErrorKind::TimedOut) | Err(std::io::ErrorKind::WouldBlock)),
        "type 3 replied {idle:?}"
    );
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let pushed = read_cmd_result(&mut session.inbound);
    eprintln!("unknown push {pushed:?}");
    assert!(
        matches!(pushed.as_ref().map(|_| ()).map_err(|err| err.kind()), Err(std::io::ErrorKind::TimedOut) | Err(std::io::ErrorKind::WouldBlock)),
        "type 3 pushed {pushed:?}"
    );
    session.command.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    write_cmd(&mut session.command, CPU_LOAD, &[]);
    let (ty, body) = read_cmd(&mut session.command);
    let value = command::read_float(&body).unwrap_or_else(|| panic!("cpu body {body:?}"));
    eprintln!("unknown then cpu {ty} {value}");
    assert_eq!(ty, CPU_LOAD, "the command after type 3 was {ty}");
    assert!(value.is_finite() && (0.0..=100.0).contains(&value), "cpu {value}");
    session.finish();
}

fn join_restart(session: &mut Session, reason: &str) {
    session.command.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let after = read_cmd_result(&mut session.command);
    assert!(
        matches!(
            after.as_ref().map(|_| ()).map_err(|err| err.kind()),
            Err(std::io::ErrorKind::UnexpectedEof)
                | Err(std::io::ErrorKind::ConnectionReset)
                | Err(std::io::ErrorKind::ConnectionAborted)
                | Err(std::io::ErrorKind::BrokenPipe)
        ),
        "{reason} did not close the command socket: {after:?}"
    );
    let server = session.server.take().expect("worker thread");
    let err = server.join().expect("worker panicked").expect_err("restart returned Ok");
    assert_eq!(err.kind(), std::io::ErrorKind::Interrupted, "{err}");
    assert_eq!(err.to_string(), reason);
    assert!(restart_requested(&err), "{err}");
}

/// `Quit` closes the command socket. A worker that ignored it would keep the read waiting.
#[test]
fn quit_closes_the_worker() {
    let mut session = Session::start(32, false);
    write_cmd(&mut session.command, QUIT, &[]);
    session.command.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let after = read_cmd_result(&mut session.command);
    assert!(
        matches!(
            after.as_ref().map(|_| ()).map_err(|err| err.kind()),
            Err(std::io::ErrorKind::UnexpectedEof)
                | Err(std::io::ErrorKind::ConnectionReset)
                | Err(std::io::ErrorKind::ConnectionAborted)
                | Err(std::io::ErrorKind::BrokenPipe)
        ),
        "QUIT did not close the command socket: {after:?}"
    );
    let server = session.server.take().expect("worker thread");
    server.join().unwrap().unwrap();
}

/// Recents are JSON lines, newest first, capped at 10, for this client's host.
#[test]
fn recents_are_newest_first_and_capped_at_ten() {
    let mut session = Session::start(32, false);
    let list = plugin_list(&mut session);
    let mut candidates = vec![BUILTIN_GAIN_ID.to_string()];
    for plugin in plugins_of(&list) {
        if plugin.get("isInstrument").and_then(|value| value.as_bool()) == Some(true) {
            continue;
        }
        if plugin.get("type").and_then(|value| value.as_str()) == Some("Builtin") {
            continue;
        }
        let name = plugin.get("name").and_then(|value| value.as_str()).unwrap_or("").to_ascii_lowercase();
        if name.contains("speech") || name.contains("net") || name.contains("record") || name.contains("output") {
            continue;
        }
        if let Some(id) = plugin.get("id").and_then(|value| value.as_str()) {
            if !candidates.iter().any(|existing| existing == id) {
                candidates.push(id.to_string());
            }
        }
    }
    let mut loaded = Vec::new();
    for id in &candidates {
        if loaded.len() == 11 {
            break;
        }
        if try_add(&mut session, id) {
            loaded.push(id.clone());
        }
    }
    assert_eq!(loaded.len(), 11, "need 11 instantiable plugins to prove the cap, loaded {loaded:?}");
    let recents = recents_ids(&mut session);
    eprintln!("recents after 11 adds: {recents:?}");
    assert_eq!(recents.len(), 10, "cap is 10, got {}", recents.len());
    assert_eq!(recents[0], loaded[10], "newest id should be first");
    assert!(!recents.contains(&loaded[0]), "oldest id should have been dropped");
    assert!(try_add(&mut session, &loaded[1]), "re-add of {} failed", loaded[1]);
    let again = recents_ids(&mut session);
    assert_eq!(again.len(), 10, "re-add grew the list: {again:?}");
    assert_eq!(again[0], loaded[1], "re-add should move the id to the front");
    assert_eq!(again.iter().filter(|id| *id == &loaded[1]).count(), 1, "re-add duplicated {}", loaded[1]);
    session.finish();
}

/// Get and set plugin settings use a parameter chunk. A numeric string does not set dB.
#[test]
fn plugin_settings_round_trip_is_a_parameter_chunk() {
    let mut session = Session::start(256, false);
    add_id(&mut session, BUILTIN_GAIN_ID, "0");
    let ones = vec![vec![1.0f32; 256], vec![1.0; 256]];
    let (_header, unity) = exchange_f32(&mut session, &ones, &[]);
    assert!((unity[0][0] - 1.0).abs() < 1.0e-4, "0 dB gain changed the block: {}", unity[0][0]);

    let chunk = serde_json::json!({"v": 1, "params": [0.375]}).to_string();
    set_plugin_settings(&mut session, 0, &chunk);
    let got = plugin_settings(&mut session, 0);
    let parsed: serde_json::Value = serde_json::from_str(&got).expect("settings chunk");
    eprintln!("gain settings {got}");
    assert_eq!(parsed["v"], 1);
    let stored = parsed["params"][0].as_f64().unwrap() as f32;
    assert!((stored - 0.375).abs() < 1.0e-4, "chunk param was {stored}");
    let (_header, cut) = exchange_f32(&mut session, &ones, &[]);
    assert!((cut[0][0] - lin(-6.0)).abs() < 1.0e-4, "chunk should be -6 dB, got {}", cut[0][0]);

    set_plugin_settings(&mut session, 0, "-12");
    let (_header, still) = exchange_f32(&mut session, &ones, &[]);
    assert!((still[0][0] - lin(-6.0)).abs() < 1.0e-4, "numeric settings string changed gain to {}", still[0][0]);
    let again = plugin_settings(&mut session, 0);
    assert!(again.contains("params"), "get returned a gain string: {again}");
    let parsed: serde_json::Value = serde_json::from_str(&again).unwrap();
    let stored = parsed["params"][0].as_f64().unwrap() as f32;
    assert!((stored - 0.375).abs() < 1.0e-4, "numeric string replaced the chunk with {stored}");

    write_cmd(&mut session.command, DEL_PLUGIN, &command::number(0));
    let (ty, _) = read_cmd(&mut session.command);
    assert_eq!(ty, RESULT);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| panic!("plugin list has no hosted effect: {list}"));
    let id = effect["id"].as_str().unwrap();
    let params = add_id(&mut session, id, "");
    let count = params.as_array().map(|items| items.len()).unwrap_or(0);
    assert!(count > 0, "{id} published no parameters");
    let signal: Vec<f32> = (0..256).map(|i| (i as f32 * 0.17).sin() * 0.8).collect();
    let input = vec![signal.clone(), signal];
    let mut changed = false;
    for index in 0..count.min(8) {
        let mut low = vec![0.0f32; count];
        let mut high = vec![0.0f32; count];
        if let Ok(current) = serde_json::from_str::<serde_json::Value>(&plugin_settings(&mut session, 0)) {
            if let Some(items) = current["params"].as_array() {
                for (i, item) in items.iter().enumerate() {
                    let value = item.as_f64().unwrap_or(0.0) as f32;
                    if i < low.len() {
                        low[i] = value;
                        high[i] = value;
                    }
                }
            }
        }
        low[index] = 0.0;
        high[index] = 1.0;
        set_plugin_settings(&mut session, 0, &serde_json::json!({"v": 1, "params": low}).to_string());
        let (_header, quiet) = exchange_f32(&mut session, &input, &[]);
        set_plugin_settings(&mut session, 0, &serde_json::json!({"v": 1, "params": high}).to_string());
        let read_back = get_param(&mut session, 0, index as i32);
        let (_header, loud) = exchange_f32(&mut session, &input, &[]);
        if (read_back - 0.0).abs() > 0.2 && max_abs_diff(&quiet[0], &loud[0]) > 1.0e-3 {
            changed = true;
            eprintln!("settings chunk moved {id} param {index} and the next block");
            break;
        }
    }
    assert!(changed, "settings chunk did not change a hosted parameter of {id}");
    session.finish();
}

/// The generic editor's top-left close box is the stand-in for
/// `ProcessorWindow::closeButtonPressed`. That push is HidePlugin on command-in.
/// A client HidePlugin clears the editor and is not echoed.
#[test]
fn close_box_pushes_hide_plugin_and_client_hide_is_not_echoed() {
    let mut session = Session::start(64, false);
    add_id(&mut session, BUILTIN_GAIN_ID, "0");
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| panic!("plugin list has no hosted effect: {list}"));
    let id = effect["id"].as_str().unwrap();
    add_id(&mut session, id, "");
    drain_inbound(&mut session);
    write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(1, 0, 0.0, 0));
    assert!(get_param(&mut session, 1, 0) < 0.05, "parameter 0 did not start at 0");
    write_cmd(&mut session.command, EDIT_PLUGIN, &command::edit_plugin(1, 0, 0, 0));
    session.screen.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (ty, body) = read_cmd(&mut session.screen);
    assert_eq!(ty, SCREEN_CAPTURE);
    assert_decodable_image(&body);

    write_cmd(&mut session.command, MOUSE, &mouse_at(1, 2.0, 2.0));
    session.inbound.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let (ty, body) = read_cmd(&mut session.inbound);
    assert_eq!(ty, HIDE_PLUGIN, "close box did not push HidePlugin, got type {ty}");
    let index = command::read_number(&body).expect("hide index");
    eprintln!("close box hid slot {index}");
    assert_eq!(index, 1, "hide index was {index}, expected the hosted slot");
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let extra = read_cmd_result(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert!(extra.is_err(), "close box pushed more than HidePlugin: {extra:?}");
    assert!(get_param(&mut session, 1, 0) < 0.05, "close box toggled parameter 0");
    write_cmd(&mut session.command, GET_SCREEN_BOUNDS, &[]);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, SCREEN_BOUNDS);
    assert_eq!(command::read_edit_plugin(&body).unwrap(), (0, 0, 0, 0), "editor stayed open after the close box");

    write_cmd(&mut session.command, EDIT_PLUGIN, &command::edit_plugin(1, 0, 0, 0));
    let (ty, body) = read_cmd(&mut session.screen);
    assert_eq!(ty, SCREEN_CAPTURE);
    assert_decodable_image(&body);
    write_cmd(&mut session.command, HIDE_PLUGIN, &command::number(1));
    assert_no_reply(&mut session);
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let echoed = read_cmd_result(&mut session.inbound);
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert!(echoed.is_err(), "client HidePlugin was echoed on command-in: {echoed:?}");
    write_cmd(&mut session.command, GET_SCREEN_BOUNDS, &[]);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, SCREEN_BOUNDS);
    assert_eq!(command::read_edit_plugin(&body).unwrap(), (0, 0, 0, 0));
    session.finish();
}

/// Factory preset names arrive with the add. Preset is 12 bytes. Channel 0
/// selects the program. A different channel does not touch the one instance.
#[test]
fn factory_preset_on_channel_zero_changes_a_hosted_plugin() {
    let mut session = Session::start(256, false);
    let list = plugin_list(&mut session);
    let mut notes = Vec::new();
    let mut proved = false;
    for (id, _instrument) in au_candidates(&list) {
        let Some((names, count)) = add_presets(&mut session, &id) else {
            notes.push(format!("{id} failed to load"));
            continue;
        };
        drain_inbound(&mut session);
        if names.len() < 2 {
            notes.push(format!("{id} presets {}", names.len()));
            delete_slot(&mut session, 0);
            continue;
        }
        eprintln!("{id} factory presets {}", names.len());
        write_preset(&mut session, 0, 0, 0);
        let base = param_snapshot(&mut session, count);
        let mut moved = None;
        for preset in [1, names.len().saturating_sub(1) as i32] {
            write_preset(&mut session, 0, preset, 0);
            let next = param_snapshot(&mut session, count);
            if param_distance(&base, &next) > 1.0e-3 {
                moved = Some((preset, next));
                break;
            }
        }
        let Some((preset, next)) = moved else {
            notes.push(format!("{id} {} presets did not move a parameter", names.len()));
            delete_slot(&mut session, 0);
            continue;
        };
        write_preset(&mut session, 0, 0, 3);
        let kept = param_snapshot(&mut session, count);
        let kept_distance = param_distance(&next, &kept);
        assert!(
            kept_distance < 1.0e-3,
            "channel 3 preset 0 undid preset {preset} on {id} (distance {kept_distance})"
        );
        write_preset(&mut session, 0, 0, 0);
        let restored = param_snapshot(&mut session, count);
        let restored_distance = param_distance(&base, &restored);
        eprintln!("{id} preset {preset} of {} moved parameters; channel 3 kept them; channel 0 restored within {restored_distance}", names.len());
        assert!(
            restored_distance < 1.0e-3,
            "channel 0 preset 0 did not restore {id} (distance {restored_distance})"
        );
        proved = true;
        break;
    }
    assert!(proved, "no scanned AudioUnit factory preset changed a parameter: {notes:?}");
    session.finish();
}

/// The worker applies the audio-block playhead. GetPluginSettings reports it.
/// AudioGridder's own settings chunk has no playhead; this object is only how
/// the test observes the worker. Builtin gain must still pass the samples.
#[test]
fn worker_playhead_follows_each_audio_block() {
    let mut session = Session::start(64, false);
    add_id(&mut session, BUILTIN_GAIN_ID, "0");
    let initial = playhead_of(&plugin_settings(&mut session, 0));
    assert_eq!(initial, (120.0, false, 0, 0.0), "playhead before audio was {initial:?}");

    let ones = vec![vec![1.0f32; 64], vec![1.0; 64]];
    let mut first = PositionInfo::default();
    first.bpm = 173.0;
    first.playing = true;
    first.time_in_samples = 999;
    first.ppq_position = 3.25;
    let (_header, audio) = exchange_f32_at(&mut session, &ones, &[], first);
    assert!((audio[0][0] - 1.0).abs() < 1.0e-4, "playhead changed unity gain to {}", audio[0][0]);
    let got = plugin_settings(&mut session, 0);
    eprintln!("playhead after first block {got}");
    assert_eq!(playhead_of(&got), (173.0, true, 999, 3.25));
    let parsed: serde_json::Value = serde_json::from_str(&got).unwrap();
    assert_eq!(parsed["v"], 1);
    assert!((parsed["params"][0].as_f64().unwrap() - 0.5).abs() < 1.0e-4, "gain chunk changed");

    let mut second = PositionInfo::default();
    second.bpm = 90.0;
    second.playing = false;
    second.time_in_samples = 10;
    second.ppq_position = 0.5;
    let (_header, audio) = exchange_f32_at(&mut session, &ones, &[], second);
    assert!((audio[0][0] - 1.0).abs() < 1.0e-4, "second playhead changed unity gain to {}", audio[0][0]);
    let got = plugin_settings(&mut session, 0);
    eprintln!("playhead after second block {got}");
    assert_eq!(playhead_of(&got), (90.0, false, 10, 0.5));
    session.finish();
}

/// The handshake block size is the prepare size. A longer buffer is processed
/// only for that many frames. The tail stays equal to the input.
#[test]
fn handshake_block_size_limits_the_hosted_block() {
    let mut session = Session::start(32, false);
    let list = plugin_list(&mut session);
    let effect = prefer_effect(&plugins_of(&list)).unwrap_or_else(|| panic!("plugin list has no hosted effect: {list}"));
    let id = effect["id"].as_str().unwrap();
    let params = add_id(&mut session, id, "");
    let count = params.as_array().map(|items| items.len()).unwrap_or(0);
    assert!(count > 0, "{id} published no parameters");
    let signal: Vec<f32> = (0..128).map(|i| (i as f32 * 0.17).sin() * 0.8).collect();
    let input = vec![signal.clone(), signal.clone()];
    let mut limited = false;
    for index in 0..count.min(8) {
        // Command and audio sockets are independent: a reply on the command socket makes
        // sure the value is applied before the next block is sent.
        write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, index as i32, 0.0, 0));
        let _ = get_param(&mut session, 0, index as i32);
        let (_header, low) = exchange_f32(&mut session, &input, &[]);
        write_cmd(&mut session.command, PARAMETER_VALUE, &command::parameter_value(0, index as i32, 1.0, 0));
        let _ = get_param(&mut session, 0, index as i32);
        let (_header, high) = exchange_f32(&mut session, &input, &[]);
        let head = max_abs_diff(&low[0][..32], &high[0][..32]);
        let tail = max_abs_diff(&high[0][32..], &signal[32..]).max(max_abs_diff(&low[0][32..], &signal[32..]));
        if head > 1.0e-3 && tail < 1.0e-5 {
            eprintln!("{id} param {index} changed the first 32 samples and left the tail");
            limited = true;
            break;
        }
    }
    assert!(limited, "{id} processed past the handshake block of 32 or did not change the head");
    session.finish();
}

fn assert_no_reply(session: &mut Session) {
    session.command.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let reply = read_cmd_result(&mut session.command);
    session.command.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert!(reply.is_err(), "command replied when it should stay quiet: {reply:?}");
}

fn try_add(session: &mut Session, id: &str) -> bool {
    let body = command::json_payload(&serde_json::json!({"id": id, "settings": "", "layout": "Stereo"}));
    write_cmd(&mut session.command, ADD_PLUGIN, &body);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, ADD_PLUGIN_RESULT);
    let result = command::read_json_payload(&body).unwrap();
    if result["success"] != true {
        return false;
    }
    let (ty, _) = read_cmd(&mut session.command);
    assert_eq!(ty, PRESETS);
    let (ty, _) = read_cmd(&mut session.command);
    assert_eq!(ty, PARAMETERS);
    true
}

fn recents_ids(session: &mut Session) -> Vec<String> {
    write_cmd(&mut session.command, RECENTS_LIST, &[]);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, RECENTS_LIST);
    let packed = command::read_length_prefixed(&body).expect("recents payload");
    let text = std::str::from_utf8(packed).expect("recents utf-8");
    if text.is_empty() {
        return Vec::new();
    }
    text.lines()
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|err| panic!("recent line {line}: {err}"));
            value["id"].as_str().unwrap_or("").to_string()
        })
        .collect()
}

fn plugin_settings(session: &mut Session, index: i32) -> String {
    write_cmd(&mut session.command, GET_PLUGIN_SETTINGS, &command::number(index));
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, PLUGIN_SETTINGS);
    let packed = command::read_length_prefixed(&body).expect("settings payload");
    String::from_utf8(packed.to_vec()).expect("settings utf-8")
}

fn set_plugin_settings(session: &mut Session, index: i32, text: &str) {
    write_cmd(&mut session.command, SET_PLUGIN_SETTINGS, &command::number(index));
    write_cmd(&mut session.command, PLUGIN_SETTINGS, &command::length_prefixed(text.as_bytes()));
}

fn mouse_at(kind: u8, x: f32, y: f32) -> [u8; 28] {
    let mut mouse = [0u8; 28];
    mouse[0] = kind;
    mouse[4..8].copy_from_slice(&x.to_le_bytes());
    mouse[8..12].copy_from_slice(&y.to_le_bytes());
    mouse
}

/// `mouseevent_t` is 28 bytes. Type 10 is `MouseEvType::WHEEL`. deltaX sits at
/// byte 16 and deltaY at byte 20, after the three modifier bools and one pad.
fn mouse_wheel(x: f32, y: f32, delta_x: f32, delta_y: f32) -> [u8; 28] {
    let mut mouse = mouse_at(10, x, y);
    mouse[16..20].copy_from_slice(&delta_x.to_le_bytes());
    mouse[20..24].copy_from_slice(&delta_y.to_le_bytes());
    mouse[24] = 1;
    mouse
}

fn read_gesture(body: &[u8]) -> (i32, i32, bool, i32) {
    assert!(body.len() >= 16, "gesture is 16 bytes, got {}", body.len());
    let word = |at: usize| i32::from_le_bytes(body[at..at + 4].try_into().unwrap());
    (word(0), word(4), body[8] != 0, word(12))
}

fn drain_ready_status(session: &mut Session) {
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    if let Ok((ty, _)) = read_cmd_result(&mut session.inbound) {
        assert_eq!(ty, PLUGIN_STATUS, "unexpected command-in message {ty} while draining ready status");
    }
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
}

fn key_codes(codes: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(codes.len() * 2);
    for code in codes {
        bytes.extend_from_slice(&code.to_le_bytes());
    }
    command::length_prefixed(&bytes)
}

fn drain_inbound(session: &mut Session) {
    session.inbound.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    loop {
        match read_cmd_result(&mut session.inbound) {
            Ok((ty, _)) => assert_eq!(ty, PLUGIN_STATUS, "unexpected command-in message {ty} while draining"),
            Err(_) => break,
        }
    }
    session.inbound.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
}

fn delete_slot(session: &mut Session, index: i32) {
    write_cmd(&mut session.command, DEL_PLUGIN, &command::number(index));
    let (ty, _) = read_cmd(&mut session.command);
    assert_eq!(ty, RESULT);
}

fn add_presets(session: &mut Session, id: &str) -> Option<(Vec<String>, usize)> {
    let body = command::json_payload(&serde_json::json!({"id": id, "settings": "", "layout": "Stereo"}));
    write_cmd(&mut session.command, ADD_PLUGIN, &body);
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, ADD_PLUGIN_RESULT);
    let result = command::read_json_payload(&body).unwrap();
    if result["success"] != true {
        return None;
    }
    let (ty, body) = read_cmd(&mut session.command);
    assert_eq!(ty, PRESETS);
    let packed = command::read_length_prefixed(&body).expect("presets payload");
    let text = std::str::from_utf8(packed).expect("presets utf-8");
    let names: Vec<String> = if text.is_empty() {
        Vec::new()
    } else {
        text.split('|').map(str::to_string).collect()
    };
    assert!(names.iter().all(|name| !name.is_empty()), "{id} published an empty preset name: {names:?}");
    let (ty, params) = read_cmd(&mut session.command);
    assert_eq!(ty, PARAMETERS);
    let packed = command::read_length_prefixed(&params).unwrap_or(&[]);
    let count = decode_msgpack(packed).and_then(|value| value.as_array().map(|items| items.len())).unwrap_or(0);
    Some((names, count))
}

fn write_preset(session: &mut Session, index: i32, preset: i32, channel: i32) {
    let mut body = [0u8; 12];
    body[0..4].copy_from_slice(&index.to_le_bytes());
    body[4..8].copy_from_slice(&preset.to_le_bytes());
    body[8..12].copy_from_slice(&channel.to_le_bytes());
    write_cmd(&mut session.command, PRESET, &body);
    assert_no_reply(session);
}

fn param_snapshot(session: &mut Session, count: usize) -> Vec<f32> {
    (0..count.min(32)).map(|index| get_param(session, 0, index as i32)).collect()
}

fn param_distance(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max)
}

fn au_candidates(list: &Value) -> Vec<(String, bool)> {
    let mut found = Vec::new();
    for plugin in plugins_of(list) {
        let id = plugin.get("id").and_then(|value| value.as_str()).unwrap_or("");
        if !id.starts_with("au:") {
            continue;
        }
        let instrument = plugin.get("isInstrument").and_then(|value| value.as_bool()).unwrap_or(false);
        found.push((id.to_string(), instrument));
    }
    found.sort_by(|left, right| {
        let rank = |id: &str, instrument: bool| if id.contains("dls") { 0 } else if instrument { 2 } else { 1 };
        (rank(&left.0, left.1), left.0.as_str()).cmp(&(rank(&right.0, right.1), right.0.as_str()))
    });
    found
}

fn playhead_of(text: &str) -> (f64, bool, i64, f64) {
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap_or_else(|err| panic!("settings json {err}: {text}"));
    let play = &parsed["playhead"];
    assert!(play.is_object(), "settings have no playhead object: {text}");
    (
        play["bpm"].as_f64().unwrap_or_else(|| panic!("bpm missing in {text}")),
        play["playing"].as_bool().unwrap_or_else(|| panic!("playing missing in {text}")),
        play["samples"].as_i64().unwrap_or_else(|| panic!("samples missing in {text}")),
        play["ppq"].as_f64().unwrap_or_else(|| panic!("ppq missing in {text}")),
    )
}

fn read_cmd_result(socket: &mut TcpStream) -> std::io::Result<(i32, Vec<u8>)> {
    let mut header = [0u8; command::CMD_HEADER];
    socket.read_exact(&mut header)?;
    let message_type = i32::from_le_bytes(header[0..4].try_into().unwrap());
    let size = i32::from_le_bytes(header[4..8].try_into().unwrap());
    if size < 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "negative command size"));
    }
    let mut body = vec![0u8; size as usize];
    if size > 0 {
        socket.read_exact(&mut body)?;
    }
    Ok((message_type, body))
}

fn read_png_frame(session: &mut Session) -> (u32, u32, Vec<u8>) {
    session.screen.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    let (ty, body) = read_cmd(&mut session.screen);
    assert_eq!(ty, SCREEN_CAPTURE);
    assert!(body.len() >= 32, "screen header");
    let width = i32::from_le_bytes(body[0..4].try_into().unwrap());
    let height = i32::from_le_bytes(body[4..8].try_into().unwrap());
    let size = u64::from_le_bytes(body[24..32].try_into().unwrap()) as usize;
    let data = &body[32..32 + size];
    assert!(data.starts_with(b"\x89PNG\r\n\x1a\n"), "screen payload is not a PNG");
    let (png_w, png_h, pixels) = decode_png_rgba(data);
    assert_eq!(png_w as i32, width);
    assert_eq!(png_h as i32, height);
    (png_w, png_h, pixels)
}

fn expect_no_screen(session: &mut Session) {
    session.screen.set_read_timeout(Some(Duration::from_millis(250))).unwrap();
    let result = read_cmd_result(&mut session.screen);
    session.screen.set_read_timeout(Some(COMMAND_WAIT)).unwrap();
    assert!(result.is_err(), "expected no screen frame, got {result:?}");
}

fn opaque_pixels(pixels: &[u8]) -> usize {
    pixels.chunks_exact(4).filter(|pixel| pixel[3] == 255).count()
}

fn decode_png_rgba(data: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut at = 8;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut idat = Vec::new();
    while at + 12 <= data.len() {
        let len = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        let kind = &data[at + 4..at + 8];
        let chunk = &data[at + 8..at + 8 + len];
        if kind == b"IHDR" {
            width = u32::from_be_bytes(chunk[0..4].try_into().unwrap());
            height = u32::from_be_bytes(chunk[4..8].try_into().unwrap());
        } else if kind == b"IDAT" {
            idat.extend_from_slice(chunk);
        } else if kind == b"IEND" {
            break;
        }
        at += 12 + len;
    }
    let raw = inflate_stored(&idat);
    let stride = width as usize * 4;
    let mut pixels = Vec::with_capacity(stride * height as usize);
    let mut src = 0;
    for _ in 0..height {
        assert_eq!(raw[src], 0, "png row filter");
        src += 1;
        pixels.extend_from_slice(&raw[src..src + stride]);
        src += stride;
    }
    (width, height, pixels)
}

fn inflate_stored(zlib: &[u8]) -> Vec<u8> {
    assert!(zlib.len() > 6, "zlib block");
    assert_eq!(zlib[0], 0x78, "zlib header");
    let mut at = 2;
    let mut out = Vec::new();
    loop {
        let header = zlib[at];
        at += 1;
        let len = u16::from_le_bytes(zlib[at..at + 2].try_into().unwrap()) as usize;
        at += 2;
        let nlen = u16::from_le_bytes(zlib[at..at + 2].try_into().unwrap());
        at += 2;
        assert_eq!(nlen, !len as u16, "stored block length");
        out.extend_from_slice(&zlib[at..at + len]);
        at += len;
        if header & 1 == 1 {
            break;
        }
    }
    out
}

fn assert_decodable_image(body: &[u8]) {
    assert!(body.len() >= 32, "screen capture header, got {} bytes", body.len());
    let width = i32::from_le_bytes(body[0..4].try_into().unwrap());
    let height = i32::from_le_bytes(body[4..8].try_into().unwrap());
    let size = u64::from_le_bytes(body[24..32].try_into().unwrap()) as usize;
    assert!(width > 0 && height > 0, "image size {width}x{height}");
    assert!(body.len() >= 32 + size, "capture truncated");
    let data = &body[32..32 + size];
    assert!(!data.is_empty(), "image has no bytes");
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        assert!(data.len() >= 24, "png header");
        let png_w = u32::from_be_bytes(data[16..20].try_into().unwrap());
        let png_h = u32::from_be_bytes(data[20..24].try_into().unwrap());
        assert_eq!(png_w as i32, width);
        assert_eq!(png_h as i32, height);
        assert!(data.windows(4).any(|chunk| chunk == b"IDAT"), "png has no image data");
    } else {
        let pixels = width as usize * height as usize * 4;
        assert!(data.len() >= pixels, "rgba image is {} bytes, need {pixels}", data.len());
        let nonzero = data[..pixels].iter().filter(|byte| **byte != 0).count();
        eprintln!("editor image {width}x{height} rgba bytes {pixels} nonzero {nonzero}");
        assert!(nonzero > 0, "editor image pixels are empty");
    }
}

fn decode_msgpack(bytes: &[u8]) -> Option<Value> {
    let (value, used) = read_msgpack(bytes)?;
    if used != bytes.len() {
        return None;
    }
    Some(value)
}

fn read_msgpack(bytes: &[u8]) -> Option<(Value, usize)> {
    let tag = *bytes.first()?;
    match tag {
        0xc0 => Some((Value::Null, 1)),
        0xc2 => Some((Value::Bool(false), 1)),
        0xc3 => Some((Value::Bool(true), 1)),
        0x00..=0x7f => Some((Value::from(i64::from(tag)), 1)),
        0xe0..=0xff => Some((Value::from(tag as i8 as i64), 1)),
        0xa0..=0xbf => read_str(bytes, 1, (tag & 0x1f) as usize),
        0xd9 => read_str(bytes, 2, *bytes.get(1)? as usize),
        0xda => {
            let n = u16::from_be_bytes(bytes.get(1..3)?.try_into().ok()?) as usize;
            read_str(bytes, 3, n)
        }
        0xcb => {
            let value = f64::from_be_bytes(bytes.get(1..9)?.try_into().ok()?);
            Some((Value::from(value), 9))
        }
        0xd2 => {
            let value = i32::from_be_bytes(bytes.get(1..5)?.try_into().ok()?) as i64;
            Some((Value::from(value), 5))
        }
        0xd3 => {
            let value = i64::from_be_bytes(bytes.get(1..9)?.try_into().ok()?);
            Some((Value::from(value), 9))
        }
        0xce => {
            let value = u32::from_be_bytes(bytes.get(1..5)?.try_into().ok()?) as u64;
            Some((Value::from(value), 5))
        }
        0xcf => {
            let value = u64::from_be_bytes(bytes.get(1..9)?.try_into().ok()?);
            Some((Value::from(value), 9))
        }
        0x90..=0x9f => read_array(bytes, 1, (tag & 0x0f) as usize),
        0xdc => {
            let n = u16::from_be_bytes(bytes.get(1..3)?.try_into().ok()?) as usize;
            read_array(bytes, 3, n)
        }
        0x80..=0x8f => read_map(bytes, 1, (tag & 0x0f) as usize),
        0xde => {
            let n = u16::from_be_bytes(bytes.get(1..3)?.try_into().ok()?) as usize;
            read_map(bytes, 3, n)
        }
        _ => None,
    }
}

fn read_str(bytes: &[u8], at: usize, len: usize) -> Option<(Value, usize)> {
    let text = std::str::from_utf8(bytes.get(at..at + len)?).ok()?;
    Some((Value::String(text.to_string()), at + len))
}

fn read_array(bytes: &[u8], mut at: usize, len: usize) -> Option<(Value, usize)> {
    let mut items = Vec::with_capacity(len);
    for _ in 0..len {
        let (value, used) = read_msgpack(bytes.get(at..)?)?;
        items.push(value);
        at += used;
    }
    Some((Value::Array(items), at))
}

fn read_map(bytes: &[u8], mut at: usize, len: usize) -> Option<(Value, usize)> {
    let mut map = serde_json::Map::new();
    for _ in 0..len {
        let (key, used) = read_msgpack(bytes.get(at..)?)?;
        at += used;
        let (value, used) = read_msgpack(bytes.get(at..)?)?;
        at += used;
        map.insert(key.as_str()?.to_string(), value);
    }
    Some((Value::Object(map), at))
}

#[test]
fn msgpack_parser_reads_the_protocol_writer() {
    let value = serde_json::json!({
        "plugins": [{"id": "Builtin-gain", "isInstrument": false, "n": 1, "gain": 0.5}]
    });
    let body = command::msgpack_payload(&value);
    let packed = command::read_length_prefixed(&body).unwrap();
    assert_eq!(decode_msgpack(packed).unwrap(), value);
}

#[test]
fn h5_clap_effect_scan_and_host_in_catalog() {
    let scanned = crate::host::scan();
    let clap_plugins: Vec<_> = scanned.iter().filter(|p| p.format == "CLAP").collect();
    assert!(!clap_plugins.is_empty(), "CLAP plugins should be discovered on system: {:?}", scanned);
    let first = &clap_plugins[0];
    assert!(first.id.starts_with("clap:"));
    assert!(!first.name.is_empty());
}
