//! Command channel framing from AudioGridder `Message.hpp`.
//!
//! A message is a little-endian header (`type`, `size`) plus `size` body
//! bytes. Sizes below were checked with clang++ against the same field
//! layout as the JUCE headers (`/tmp/ag-sizes`, 2026-11-07): command header
//! 8, result header 8, edit/screen-bounds 16, mono-channels 16, mouse 28,
//! screen-capture header 32, parameter value 16, gesture 16, preset 12.

use serde_json::Value;

pub const CMD_HEADER: usize = 8;
pub const MAX_BODY: usize = 1024 * 1024 * 60;

pub const ANY: i32 = 0;
pub const QUIT: i32 = 1;
pub const RESULT: i32 = 2;
pub const PLUGIN_LIST: i32 = 10;
pub const ADD_PLUGIN: i32 = 20;
pub const ADD_PLUGIN_RESULT: i32 = 21;
pub const DEL_PLUGIN: i32 = 30;
pub const PLUGIN_STATUS: i32 = 40;
pub const SET_MONO_CHANNELS: i32 = 50;
pub const EDIT_PLUGIN: i32 = 60;
pub const HIDE_PLUGIN: i32 = 61;
pub const GET_SCREEN_BOUNDS: i32 = 62;
pub const SCREEN_BOUNDS: i32 = 63;
pub const SCREEN_CAPTURE: i32 = 64;
pub const UPDATE_SCREEN_CAPTURE_AREA: i32 = 65;
pub const MOUSE: i32 = 66;
pub const KEY: i32 = 67;
pub const CLIPBOARD: i32 = 68;
pub const GET_PLUGIN_SETTINGS: i32 = 70;
pub const SET_PLUGIN_SETTINGS: i32 = 71;
pub const PLUGIN_SETTINGS: i32 = 72;
pub const BYPASS_PLUGIN: i32 = 73;
pub const UNBYPASS_PLUGIN: i32 = 74;
pub const SET_PLUGIN_OVERSAMPLING: i32 = 75;
pub const EXCHANGE_PLUGINS: i32 = 80;
pub const RECENTS_LIST: i32 = 90;
pub const PARAMETERS: i32 = 100;
pub const PARAMETER_VALUE: i32 = 101;
pub const GET_PARAMETER_VALUE: i32 = 102;
pub const GET_ALL_PARAMETER_VALUES: i32 = 103;
pub const PARAMETER_GESTURE: i32 = 104;
pub const PRESETS: i32 = 110;
pub const PRESET: i32 = 111;
pub const RESCAN: i32 = 120;
pub const RESTART: i32 = 121;
pub const CPU_LOAD: i32 = 130;
pub const SERVER_ERROR: i32 = 200;

// OpenGridder extensions (P80-P82)
pub const OG_LATENCY: i32 = 140;
pub const OG_PLUGIN_LAYOUTS: i32 = 141;
pub const OG_ALL_PARAMETER_VALUES: i32 = 142;

pub const FLAG_NO_PLUGINLIST_FILTER: u8 = 1;
pub const RESP_SANDBOX: u32 = 1;
pub const RESP_LOCAL_MODE: u32 = 2;

// OpenGridder flag extensions (P83-P85)
pub const REQ_FLAG_LOCAL_MODE: u8 = 2;
pub const REQ_FLAG_STATE_ACK: u8 = 4;
pub const RESP_FLAG_STATE_ACK: u32 = 1 << 8;

/// AudioGridder JsonMessage payload wrapper (used by PluginTrayMessage and SandboxMessage)
#[derive(Clone, Debug, PartialEq)]
pub struct JsonMessage {
    pub msg_type: u16,
    pub uuid: String,
    pub data: serde_json::Value,
}

impl JsonMessage {
    pub fn new(msg_type: u16, uuid: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            msg_type,
            uuid: uuid.into(),
            data,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let val = serde_json::json!({
            "type": self.msg_type,
            "uuid": self.uuid,
            "data": self.data,
        });
        serde_json::to_vec(&val).unwrap_or_default()
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let val: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        let obj = val.as_object()?;
        let msg_type = obj.get("type")?.as_u64()? as u16;
        let uuid = obj.get("uuid")?.as_str()?.to_string();
        let data = obj.get("data")?.clone();
        Some(Self {
            msg_type,
            uuid,
            data,
        })
    }
}

pub mod plugin_tray {
    pub const STATUS: u16 = 0;
    pub const CHANGE_SERVER: u16 = 1;
    pub const RELOAD: u16 = 2;
    pub const GET_RECENTS: u16 = 3;
    pub const UPDATE_RECENTS: u16 = 4;
    pub const SHOW_MONITOR: u16 = 5;
    pub const STOP: u16 = 6;
}

pub mod sandbox_msg {
    pub const CONFIG: u16 = 0;
    pub const SANDBOX_PORT: u16 = 1;
    pub const SHOW_EDITOR: u16 = 2;
    pub const HIDE_EDITOR: u16 = 3;
    pub const METRICS: u16 = 4;
}

pub fn encode(message_type: i32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(CMD_HEADER + body.len());
    out.extend_from_slice(&message_type.to_le_bytes());
    out.extend_from_slice(&(body.len() as i32).to_le_bytes());
    out.extend_from_slice(body);
    out
}

pub fn decode(bytes: &[u8]) -> Option<(i32, Vec<u8>)> {
    if bytes.len() < CMD_HEADER {
        return None;
    }
    let message_type = i32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let size = i32::from_le_bytes(bytes[4..8].try_into().ok()?);
    if size < 0 || size as usize > MAX_BODY {
        return None;
    }
    let end = CMD_HEADER + size as usize;
    if bytes.len() < end {
        return None;
    }
    Some((message_type, bytes[CMD_HEADER..end].to_vec()))
}

pub fn number(value: i32) -> [u8; 4] {
    value.to_le_bytes()
}

pub fn read_number(body: &[u8]) -> Option<i32> {
    let bytes: [u8; 4] = body.get(..4)?.try_into().ok()?;
    Some(i32::from_le_bytes(bytes))
}

pub fn float_bits(value: f32) -> [u8; 4] {
    value.to_le_bytes()
}

pub fn read_float(body: &[u8]) -> Option<f32> {
    let bytes: [u8; 4] = body.get(..4)?.try_into().ok()?;
    Some(f32::from_le_bytes(bytes))
}

/// `StringPayload` / `BinaryPayload`: int32 length, then that many bytes.
pub fn length_prefixed(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + data.len());
    out.extend_from_slice(&(data.len() as i32).to_le_bytes());
    out.extend_from_slice(data);
    out
}

pub fn read_length_prefixed(body: &[u8]) -> Option<&[u8]> {
    let size = read_number(body)? as usize;
    body.get(4..4 + size)
}

pub fn result(rc: i32, text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + text.len());
    out.extend_from_slice(&rc.to_le_bytes());
    out.extend_from_slice(&(text.len() as i32).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
    out
}

pub fn read_result(body: &[u8]) -> Option<(i32, &str)> {
    if body.len() < 8 {
        return None;
    }
    let rc = i32::from_le_bytes(body[0..4].try_into().ok()?);
    let size = i32::from_le_bytes(body[4..8].try_into().ok()?) as usize;
    let text = std::str::from_utf8(body.get(8..8 + size)?).ok()?;
    Some((rc, text))
}

pub fn json_payload(value: &Value) -> Vec<u8> {
    length_prefixed(value.to_string().as_bytes())
}

pub fn read_json_payload(body: &[u8]) -> Option<Value> {
    let bytes = read_length_prefixed(body)?;
    serde_json::from_slice(bytes).ok()
}

pub fn msgpack_payload(value: &Value) -> Vec<u8> {
    let mut packed = Vec::new();
    write_msgpack(&mut packed, value);
    length_prefixed(&packed)
}

/// Decode one msgpack value written by `msgpack_payload`'s inner bytes.
pub fn decode_msgpack(bytes: &[u8]) -> Option<Value> {
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
        0xa0..=0xbf => read_msgpack_str(bytes, 1, (tag & 0x1f) as usize),
        0xd9 => read_msgpack_str(bytes, 2, *bytes.get(1)? as usize),
        0xda => {
            let len = u16::from_be_bytes(bytes.get(1..3)?.try_into().ok()?) as usize;
            read_msgpack_str(bytes, 3, len)
        }
        0xca => {
            let value = f32::from_be_bytes(bytes.get(1..5)?.try_into().ok()?) as f64;
            Some((Value::from(value), 5))
        }
        0xcb => {
            let value = f64::from_be_bytes(bytes.get(1..9)?.try_into().ok()?);
            Some((Value::from(value), 9))
        }
        0xcc => {
            let value = *bytes.get(1)? as u64;
            Some((Value::from(value), 2))
        }
        0xcd => {
            let value = u16::from_be_bytes(bytes.get(1..3)?.try_into().ok()?) as u64;
            Some((Value::from(value), 3))
        }
        0xce => {
            let value = u32::from_be_bytes(bytes.get(1..5)?.try_into().ok()?) as u64;
            Some((Value::from(value), 5))
        }
        0xcf => {
            let value = u64::from_be_bytes(bytes.get(1..9)?.try_into().ok()?);
            Some((Value::from(value), 9))
        }
        0xd0 => {
            let value = *bytes.get(1)? as i8 as i64;
            Some((Value::from(value), 2))
        }
        0xd1 => {
            let value = i16::from_be_bytes(bytes.get(1..3)?.try_into().ok()?) as i64;
            Some((Value::from(value), 3))
        }
        0xd2 => {
            let value = i32::from_be_bytes(bytes.get(1..5)?.try_into().ok()?) as i64;
            Some((Value::from(value), 5))
        }
        0xd3 => {
            let value = i64::from_be_bytes(bytes.get(1..9)?.try_into().ok()?);
            Some((Value::from(value), 9))
        }
        0xdb => {
            let len = u32::from_be_bytes(bytes.get(1..5)?.try_into().ok()?) as usize;
            read_msgpack_str(bytes, 5, len)
        }
        0x90..=0x9f => read_msgpack_array(bytes, 1, (tag & 0x0f) as usize),
        0xdc => {
            let len = u16::from_be_bytes(bytes.get(1..3)?.try_into().ok()?) as usize;
            read_msgpack_array(bytes, 3, len)
        }
        0xdd => {
            let len = u32::from_be_bytes(bytes.get(1..5)?.try_into().ok()?) as usize;
            read_msgpack_array(bytes, 5, len)
        }
        0x80..=0x8f => read_msgpack_map(bytes, 1, (tag & 0x0f) as usize),
        0xde => {
            let len = u16::from_be_bytes(bytes.get(1..3)?.try_into().ok()?) as usize;
            read_msgpack_map(bytes, 3, len)
        }
        0xdf => {
            let len = u32::from_be_bytes(bytes.get(1..5)?.try_into().ok()?) as usize;
            read_msgpack_map(bytes, 5, len)
        }
        _ => None,
    }
}

fn read_msgpack_str(bytes: &[u8], at: usize, len: usize) -> Option<(Value, usize)> {
    let text = std::str::from_utf8(bytes.get(at..at + len)?).ok()?;
    Some((Value::String(text.to_string()), at + len))
}

fn read_msgpack_array(bytes: &[u8], mut at: usize, len: usize) -> Option<(Value, usize)> {
    let mut items = Vec::with_capacity(len);
    for _ in 0..len {
        let (value, used) = read_msgpack(bytes.get(at..)?)?;
        items.push(value);
        at += used;
    }
    Some((Value::Array(items), at))
}

fn read_msgpack_map(bytes: &[u8], mut at: usize, len: usize) -> Option<(Value, usize)> {
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

fn write_msgpack(out: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Null => out.push(0xc0),
        Value::Bool(true) => out.push(0xc3),
        Value::Bool(false) => out.push(0xc2),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                write_int(out, integer);
            } else if let Some(unsigned) = number.as_u64() {
                write_uint(out, unsigned);
            } else {
                write_f64(out, number.as_f64().unwrap_or(0.0));
            }
        }
        Value::String(text) => write_str(out, text),
        Value::Array(items) => {
            write_len(out, items.len(), 0x90, 0xdc);
            for item in items {
                write_msgpack(out, item);
            }
        }
        Value::Object(map) => {
            write_len(out, map.len(), 0x80, 0xde);
            for (key, item) in map {
                write_str(out, key);
                write_msgpack(out, item);
            }
        }
    }
}

fn write_len(out: &mut Vec<u8>, len: usize, fix: u8, wide: u8) {
    if len < 16 {
        out.push(fix | len as u8);
    } else {
        out.push(wide);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    }
}

fn write_str(out: &mut Vec<u8>, text: &str) {
    let bytes = text.as_bytes();
    if bytes.len() < 32 {
        out.push(0xa0 | bytes.len() as u8);
    } else if bytes.len() < 256 {
        out.push(0xd9);
        out.push(bytes.len() as u8);
    } else {
        out.push(0xda);
        out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    }
    out.extend_from_slice(bytes);
}

fn write_int(out: &mut Vec<u8>, value: i64) {
    if (0..128).contains(&value) {
        out.push(value as u8);
    } else if (-32..0).contains(&value) {
        out.push(value as u8);
    } else if (i32::MIN as i64..=i32::MAX as i64).contains(&value) {
        out.push(0xd2);
        out.extend_from_slice(&(value as i32).to_be_bytes());
    } else {
        out.push(0xd3);
        out.extend_from_slice(&value.to_be_bytes());
    }
}

fn write_uint(out: &mut Vec<u8>, value: u64) {
    if value < 128 {
        out.push(value as u8);
    } else if value <= u32::MAX as u64 {
        out.push(0xce);
        out.extend_from_slice(&(value as u32).to_be_bytes());
    } else {
        out.push(0xcf);
        out.extend_from_slice(&value.to_be_bytes());
    }
}

fn write_f64(out: &mut Vec<u8>, value: f64) {
    out.push(0xcb);
    out.extend_from_slice(&value.to_be_bytes());
}

pub fn edit_plugin(index: i32, channel: i32, x: i32, y: i32) -> [u8; 16] {
    let mut body = [0u8; 16];
    body[0..4].copy_from_slice(&index.to_le_bytes());
    body[4..8].copy_from_slice(&channel.to_le_bytes());
    body[8..12].copy_from_slice(&x.to_le_bytes());
    body[12..16].copy_from_slice(&y.to_le_bytes());
    body
}

pub fn read_edit_plugin(body: &[u8]) -> Option<(i32, i32, i32, i32)> {
    if body.len() < 16 {
        return None;
    }
    let word = |at: usize| -> Option<i32> { Some(i32::from_le_bytes(body.get(at..at + 4)?.try_into().ok()?)) };
    Some((word(0)?, word(4)?, word(8)?, word(12)?))
}

pub fn parameter_value(idx: i32, param_idx: i32, value: f32, channel: i32) -> [u8; 16] {
    let mut body = [0u8; 16];
    body[0..4].copy_from_slice(&idx.to_le_bytes());
    body[4..8].copy_from_slice(&param_idx.to_le_bytes());
    body[8..12].copy_from_slice(&value.to_le_bytes());
    body[12..16].copy_from_slice(&channel.to_le_bytes());
    body
}

/// `parametergesture_t`: two ints, a bool, three pad bytes, then channel. 16 bytes.
pub fn parameter_gesture(idx: i32, param_idx: i32, starting: bool, channel: i32) -> [u8; 16] {
    let mut body = [0u8; 16];
    body[0..4].copy_from_slice(&idx.to_le_bytes());
    body[4..8].copy_from_slice(&param_idx.to_le_bytes());
    body[8] = u8::from(starting);
    body[12..16].copy_from_slice(&channel.to_le_bytes());
    body
}

pub fn read_parameter_value(body: &[u8]) -> Option<(i32, i32, f32, i32)> {
    if body.len() < 16 {
        return None;
    }
    let word = |at: usize| -> Option<i32> { Some(i32::from_le_bytes(body.get(at..at + 4)?.try_into().ok()?)) };
    let value = f32::from_le_bytes(body.get(8..12)?.try_into().ok()?);
    Some((word(0)?, word(4)?, value, word(12)?))
}

pub fn screen_bounds(x: i32, y: i32, w: i32, h: i32) -> [u8; 16] {
    edit_plugin(x, y, w, h)
}

/// `setmonochannels_t`: int, 4 bytes of padding, uint64. clang size 16.
pub fn mono_channels(idx: i32, channels: u64) -> [u8; 16] {
    let mut body = [0u8; 16];
    body[0..4].copy_from_slice(&idx.to_le_bytes());
    body[8..16].copy_from_slice(&channels.to_le_bytes());
    body
}

pub fn read_mono_channels(body: &[u8]) -> Option<(i32, u64)> {
    if body.len() < 16 {
        return None;
    }
    let idx = i32::from_le_bytes(body[0..4].try_into().ok()?);
    let channels = u64::from_le_bytes(body[8..16].try_into().ok()?);
    Some((idx, channels))
}

pub fn pair_i32(a: i32, b: i32) -> [u8; 8] {
    let mut body = [0u8; 8];
    body[0..4].copy_from_slice(&a.to_le_bytes());
    body[4..8].copy_from_slice(&b.to_le_bytes());
    body
}

pub fn read_pair_i32(body: &[u8]) -> Option<(i32, i32)> {
    if body.len() < 8 {
        return None;
    }
    Some((
        i32::from_le_bytes(body[0..4].try_into().ok()?),
        i32::from_le_bytes(body[4..8].try_into().ok()?),
    ))
}

/// `preset_t` is three ints: slot, program, channel. An 8-byte pair is not a preset.
pub fn read_preset(body: &[u8]) -> Option<(i32, i32, i32)> {
    if body.len() < 12 {
        return None;
    }
    Some((
        i32::from_le_bytes(body[0..4].try_into().ok()?),
        i32::from_le_bytes(body[4..8].try_into().ok()?),
        i32::from_le_bytes(body[8..12].try_into().ok()?),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_numbers_match_message_hpp() {
        let pairs = [
            (ANY, 0), (QUIT, 1), (RESULT, 2), (PLUGIN_LIST, 10), (ADD_PLUGIN, 20),
            (ADD_PLUGIN_RESULT, 21), (DEL_PLUGIN, 30), (PLUGIN_STATUS, 40), (SET_MONO_CHANNELS, 50),
            (EDIT_PLUGIN, 60), (HIDE_PLUGIN, 61), (GET_SCREEN_BOUNDS, 62), (SCREEN_BOUNDS, 63),
            (SCREEN_CAPTURE, 64), (UPDATE_SCREEN_CAPTURE_AREA, 65), (MOUSE, 66), (KEY, 67),
            (CLIPBOARD, 68), (GET_PLUGIN_SETTINGS, 70), (SET_PLUGIN_SETTINGS, 71), (PLUGIN_SETTINGS, 72),
            (BYPASS_PLUGIN, 73), (UNBYPASS_PLUGIN, 74), (SET_PLUGIN_OVERSAMPLING, 75),
            (EXCHANGE_PLUGINS, 80), (RECENTS_LIST, 90), (PARAMETERS, 100), (PARAMETER_VALUE, 101),
            (GET_PARAMETER_VALUE, 102), (GET_ALL_PARAMETER_VALUES, 103), (PARAMETER_GESTURE, 104),
            (PRESETS, 110), (PRESET, 111), (RESCAN, 120), (RESTART, 121), (CPU_LOAD, 130),
            (SERVER_ERROR, 200),
        ];
        for (value, expect) in pairs {
            assert_eq!(value, expect);
        }
    }

    #[test]
    fn header_and_payloads_roundtrip() {
        let raw = encode(ADD_PLUGIN, &json_payload(&serde_json::json!({"id": "Builtin-gain"})));
        let (ty, body) = decode(&raw).unwrap();
        assert_eq!(ty, 20);
        assert_eq!(read_json_payload(&body).unwrap()["id"], "Builtin-gain");

        let body = result(256, "ok");
        let (rc, text) = read_result(&body).unwrap();
        assert_eq!(rc, 256);
        assert_eq!(text, "ok");
        assert_eq!(read_number(&number(-3)), Some(-3));
        assert_eq!(read_mono_channels(&mono_channels(2, 0x11)).unwrap(), (2, 0x11));
        assert_eq!(mono_channels(2, 1)[4..8], [0, 0, 0, 0]);
        let (idx, param, value, channel) = read_parameter_value(&parameter_value(1, 0, 0.5, 0)).unwrap();
        assert_eq!((idx, param, channel), (1, 0, 0));
        assert!((value - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn msgpack_map_matches_the_fixmap_encoding() {
        let packed = msgpack_payload(&serde_json::json!({"a": 1}));
        assert_eq!(read_number(&packed), Some(4));
        assert_eq!(&packed[4..], &[0x81, 0xa1, b'a', 0x01]);
    }

    #[test]
    fn msgpack_extended_types_decode_properly() {
        // 0xcc (u8): 200
        assert_eq!(decode_msgpack(&[0xcc, 200]), Some(Value::from(200)));
        // 0xcd (u16): 3000
        assert_eq!(decode_msgpack(&[0xcd, 0x0b, 0xb8]), Some(Value::from(3000)));
        // 0xd0 (i8): -50
        assert_eq!(decode_msgpack(&[0xd0, (-50i8) as u8]), Some(Value::from(-50)));
        // 0xd1 (i16): -500
        let val_d1 = (-500i16).to_be_bytes();
        assert_eq!(decode_msgpack(&[0xd1, val_d1[0], val_d1[1]]), Some(Value::from(-500)));
        // 0xca (f32): 1.5
        let val_ca = 1.5f32.to_be_bytes();
        assert_eq!(decode_msgpack(&[0xca, val_ca[0], val_ca[1], val_ca[2], val_ca[3]]), Some(Value::from(1.5f64)));
    }

    #[test]
    fn opengridder_and_json_messages_roundtrip() {
        assert_eq!(OG_LATENCY, 140);
        assert_eq!(OG_PLUGIN_LAYOUTS, 141);
        assert_eq!(OG_ALL_PARAMETER_VALUES, 142);
        assert_eq!(REQ_FLAG_LOCAL_MODE, 2);
        assert_eq!(REQ_FLAG_STATE_ACK, 4);
        assert_eq!(RESP_FLAG_STATE_ACK, 256);

        let tray = JsonMessage::new(plugin_tray::STATUS, "test-uuid-1", serde_json::json!({"active": true}));
        let tray_bytes = tray.to_bytes();
        let parsed_tray = JsonMessage::from_bytes(&tray_bytes).unwrap();
        assert_eq!(parsed_tray, tray);
        assert_eq!(parsed_tray.msg_type, plugin_tray::STATUS);

        let sandbox = JsonMessage::new(sandbox_msg::CONFIG, "test-uuid-2", serde_json::json!({"port": 55100}));
        let sandbox_bytes = sandbox.to_bytes();
        let parsed_sandbox = JsonMessage::from_bytes(&sandbox_bytes).unwrap();
        assert_eq!(parsed_sandbox, sandbox);
        assert_eq!(parsed_sandbox.msg_type, sandbox_msg::CONFIG);
    }
}
