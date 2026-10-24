//! Full AudioGridder network client (`Client` in `Plugin/Source/Client.hpp` & `Client.cpp`).
//!
//! Implements the 4-socket architecture (`cmdOut`, `cmdIn`, `audio`, `screen`),
//! binary protocol version 13 handshake, and all 26 remote plugin RPC commands
//! including `set_plugin_oversampling` (Message Type `75`, commit `559863f`).

use crate::audio_streamer::{send_and_receive_f32, send_and_receive_f64};
use og_common::defaults::DEFAULT_SERVER_PORT;
use og_common::key_and_mouse::MouseEvType;
use og_common::message::{
    read_value, send_json_message, write_value, AddPluginResult, CpuLoadPayload,
    ExchangePayload, HandshakeRequest, HandshakeResponse, KeyPayload, MessageFactory,
    MessageType, MidiEvent, MousePayload, OversamplingPayload, ParameterValuePayload, PositionInfo,
    PresetPayload, RawMessage, ScreenBoundsPayload, AG_PROTOCOL_VERSION, HANDSHAKE_FLAGS_LOCAL_MODE,
    HANDSHAKE_FLAGS_SANDBOX_ENABLED, HANDSHAKE_FLAG_STATE_ACK, HANDSHAKE_RESP_FLAG_STATE_ACK,
};
use og_common::server_plugin::{PluginLayout, PluginParameter, ServerPlugin};
use parking_lot::Mutex;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::time::Duration;

pub type ScreenUpdateCallback = Box<dyn Fn(u32, u32, f64, &[u8]) + Send + Sync>;
pub type ParamChangeCallback = Box<dyn Fn(i32, i32, f32) + Send + Sync>;

/// Client connection configuration matching `Client::init` parameters in `Plugin/Source/Client.hpp`.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub host: String,
    pub port: u16,
    pub id: i32,
    pub sample_rate: f64,
    pub samples_per_block: i32,
    pub channels_in: i32,
    pub channels_out: i32,
    pub channels_sc: i32,
    pub double_precision: bool,
    pub num_of_buffers: i32,
    pub active_channels: u64,
    pub no_plugins_list_filter: bool,
    pub local_mode: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: DEFAULT_SERVER_PORT,
            id: 0,
            sample_rate: 48000.0,
            samples_per_block: 512,
            channels_in: 2,
            channels_out: 2,
            channels_sc: 0,
            double_precision: false,
            num_of_buffers: 4,
            active_channels: 0b11,
            no_plugins_list_filter: false,
            local_mode: false,
        }
    }
}

/// Complete AudioGridder `Client` implementation matching `Plugin/Source/Client.cpp`.
pub struct Client {
    config: Mutex<ClientConfig>,
    cmd_out: Mutex<Option<TcpStream>>,
    cmd_in: Mutex<Option<TcpStream>>,
    audio_stream: Mutex<Option<TcpStream>>,
    screen_stream: Mutex<Option<TcpStream>>,
    plugins: Mutex<Vec<ServerPlugin>>,
    ready: AtomicBool,
    sandbox_enabled: AtomicBool,
    state_ack: AtomicBool,
    latency_samples: AtomicI32,
    server_cpu_load: AtomicU32,
}

impl Default for Client {
    fn default() -> Self {
        Self::new(ClientConfig::default())
    }
}

impl Client {
    pub fn new(config: ClientConfig) -> Self {
        Self {
            config: Mutex::new(config),
            cmd_out: Mutex::new(None),
            cmd_in: Mutex::new(None),
            audio_stream: Mutex::new(None),
            screen_stream: Mutex::new(None),
            plugins: Mutex::new(Vec::new()),
            ready: AtomicBool::new(false),
            sandbox_enabled: AtomicBool::new(false),
            state_ack: AtomicBool::new(false),
            latency_samples: AtomicI32::new(0),
            server_cpu_load: AtomicU32::new(0),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    pub fn is_sandbox_enabled(&self) -> bool {
        self.sandbox_enabled.load(Ordering::Relaxed)
    }

    pub fn latency_samples(&self) -> i32 {
        self.latency_samples.load(Ordering::Relaxed)
    }

    pub fn server_cpu_load(&self) -> f32 {
        f32::from_bits(self.server_cpu_load.load(Ordering::Relaxed))
    }

    pub fn available_plugins(&self) -> Vec<ServerPlugin> {
        self.plugins.lock().clone()
    }

    pub fn set_config(&self, cfg: ClientConfig) {
        *self.config.lock() = cfg;
    }

    pub fn config(&self) -> ClientConfig {
        self.config.lock().clone()
    }

    /// Connect to the AudioGridder server, perform the `AG_PROTOCOL_VERSION = 13` handshake,
    /// open the 4 worker sockets (`cmdOut`, `cmdIn`, `audio`, `screen`), and receive `PluginList`.
    pub fn connect(&self) -> Result<(), String> {
        self.disconnect();
        let cfg = self.config.lock().clone();
        let addr_str = format!("{}:{}", cfg.host, cfg.port);
        let addr: SocketAddr = addr_str
            .to_socket_addrs()
            .map_err(|e| format!("resolve {} failed: {}", addr_str, e))?
            .next()
            .ok_or_else(|| format!("no socket address for {}", addr_str))?;

        let timeout = Duration::from_millis(5000);
        let mut master = TcpStream::connect_timeout(&addr, timeout)
            .map_err(|e| format!("master connect failed: {}", e))?;
        let _ = master.set_nodelay(true);
        let _ = master.set_read_timeout(Some(timeout));
        let _ = master.set_write_timeout(Some(timeout));

        let mut flags = HANDSHAKE_FLAG_STATE_ACK;
        if cfg.local_mode {
            flags |= HANDSHAKE_FLAGS_LOCAL_MODE;
        }
        let req = HandshakeRequest {
            version: AG_PROTOCOL_VERSION,
            channels_in: cfg.channels_in,
            channels_out: cfg.channels_out,
            channels_sc: cfg.channels_sc,
            sample_rate: cfg.sample_rate,
            samples_per_block: cfg.samples_per_block,
            double_precision: cfg.double_precision,
            client_id: 0x4F47_5253_5431_3231, // "OGRST121"
            flags,
            active_channels: cfg.active_channels,
        };
        write_value(&mut master, &req.to_bytes())
            .map_err(|e| format!("write HandshakeRequest failed: {}", e))?;

        let resp_bytes: [u8; HandshakeResponse::SIZE] = read_value(&mut master)
            .map_err(|e| format!("read HandshakeResponse failed: {}", e))?;
        let resp = HandshakeResponse::from_bytes(&resp_bytes)
            .map_err(|e| format!("invalid HandshakeResponse: {}", e))?;

        if resp.version != AG_PROTOCOL_VERSION {
            return Err(format!(
                "protocol version mismatch: server={}, client={}",
                resp.version, AG_PROTOCOL_VERSION
            ));
        }

        self.state_ack.store(resp.is_flag(HANDSHAKE_RESP_FLAG_STATE_ACK), Ordering::Relaxed);
        self.sandbox_enabled.store(
            resp.is_flag(HANDSHAKE_FLAGS_SANDBOX_ENABLED),
            Ordering::Relaxed,
        );

        let worker_addr = SocketAddr::new(addr.ip(), resp.port as u16);

        // If sandbox is disabled, the master connection becomes cmdOut directly,
        // and we open 3 additional connections (`cmdIn`, `audio`, `screen`).
        // If sandbox or dedicated worker port is used, we connect to `worker_addr`.
        let mut cmd_out = if resp.port as u16 == cfg.port {
            master
        } else {
            let s = TcpStream::connect_timeout(&worker_addr, timeout)
                .map_err(|e| format!("worker cmdOut connect failed: {}", e))?;
            let _ = s.set_nodelay(true);
            s
        };

        let cmd_in = TcpStream::connect_timeout(&worker_addr, timeout)
            .map_err(|e| format!("worker cmdIn connect failed: {}", e))?;
        let _ = cmd_in.set_nodelay(true);

        let audio_sock = TcpStream::connect_timeout(&worker_addr, timeout)
            .map_err(|e| format!("worker audio connect failed: {}", e))?;
        let _ = audio_sock.set_nodelay(true);

        let screen_sock = TcpStream::connect_timeout(&worker_addr, timeout)
            .map_err(|e| format!("worker screen connect failed: {}", e))?;
        let _ = screen_sock.set_nodelay(true);

        // Native adapters perform IO on a worker which must be able to stop.
        // Bound stalled peer reads/writes on every stream, including worker ports.
        for socket in [&cmd_out, &cmd_in, &audio_sock, &screen_sock] {
            socket.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
            socket.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;
        }

        // Receive initial PluginList message on cmdOut
        let msg = MessageFactory::get_next_message(&mut cmd_out, None)
            .map_err(|e| format!("failed to read initial PluginList: {}", e))?;
        if msg.msg_type_enum() != MessageType::PluginList {
            return Err(format!(
                "expected PluginList ({}), got {:?}",
                MessageType::PluginList as i32,
                msg.msg_type
            ));
        }
        let parsed_plugins = Self::parse_plugin_list(msg.as_str());
        *self.plugins.lock() = parsed_plugins;

        *self.cmd_out.lock() = Some(cmd_out);
        *self.cmd_in.lock() = Some(cmd_in);
        *self.audio_stream.lock() = Some(audio_sock);
        *self.screen_stream.lock() = Some(screen_sock);
        self.ready.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn disconnect(&self) {
        self.ready.store(false, Ordering::SeqCst);
        if let Some(mut s) = self.cmd_out.lock().take() {
            let quit = RawMessage::new(MessageType::Quit, Vec::new());
            let _ = quit.send(&mut s);
        }
        *self.cmd_in.lock() = None;
        *self.audio_stream.lock() = None;
        *self.screen_stream.lock() = None;
    }

    fn parse_plugin_list(text: &str) -> Vec<ServerPlugin> {
        // Supports both JSON array of ServerPlugin objects and newline-delimited ServerPlugin::from_string
        let trimmed = text.trim();
        if trimmed.starts_with('[') {
            if let Ok(list) = serde_json::from_str::<Vec<ServerPlugin>>(trimmed) {
                return list;
            }
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(arr) = val.get("plugins").and_then(|v| v.as_array()) {
                let mut out = Vec::new();
                for item in arr {
                    if let Some(s) = item.as_str() {
                        if let Some(p) = ServerPlugin::from_string(s) {
                            out.push(p);
                        }
                    } else if let Ok(p) = serde_json::from_value::<ServerPlugin>(item.clone()) {
                        out.push(p);
                    }
                }
                return out;
            }
        }
        text.lines()
            .filter_map(|l| ServerPlugin::from_string(l.trim()))
            .collect()
    }

    /// Add a plugin to the server's processor chain (`MessageType::AddPlugin`).
    /// Returns `(ok, latency_samples, parameters, layouts, presets, err)`.
    pub fn add_plugin(
        &self,
        id: &str,
        channel_layout: &str,
        mono_channels: u64,
    ) -> Result<(AddPluginResult, Vec<PluginParameter>, Vec<PluginLayout>, Vec<String>), String>
    {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;

        let payload = serde_json::json!({
            "id": id,
            "layout": channel_layout,
            "monoChannels": mono_channels,
        });
        send_json_message(sock, MessageType::AddPlugin, &payload)
            .map_err(|e| format!("send AddPlugin failed: {}", e))?;

        let res_msg = MessageFactory::get_next_message(sock, None)
            .map_err(|e| format!("read AddPluginResult failed: {}", e))?;
        let res: AddPluginResult = serde_json::from_slice(&res_msg.payload)
            .map_err(|e| format!("failed to decode AddPluginResult: {}", e))?;

        if !res.ok {
            return Err(res.err);
        }

        // Read Presets message
        let presets_msg = MessageFactory::get_next_message(sock, None)
            .map_err(|e| format!("read Presets failed: {}", e))?;
        let presets: Vec<String> = presets_msg
            .as_str()
            .split('|')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        // Read ParameterList message
        let params_msg = MessageFactory::get_next_message(sock, None)
            .map_err(|e| format!("read ParameterList failed: {}", e))?;
        let params: Vec<PluginParameter> =
            serde_json::from_str(params_msg.as_str()).unwrap_or_default();

        // Read PluginLayouts message
        let layouts_msg = MessageFactory::get_next_message(sock, None)
            .map_err(|e| format!("read PluginLayouts failed: {}", e))?;
        let layouts: Vec<PluginLayout> =
            serde_json::from_str(layouts_msg.as_str()).unwrap_or_default();

        // Read Latency message
        let lat_msg = MessageFactory::get_next_message(sock, None)
            .map_err(|e| format!("read Latency failed: {}", e))?;
        if lat_msg.payload.len() >= 4 {
            let lat = i32::from_le_bytes(lat_msg.payload[0..4].try_into().unwrap());
            self.latency_samples.store(lat, Ordering::Relaxed);
        } else {
            self.latency_samples
                .store(res.latency_samples, Ordering::Relaxed);
        }

        Ok((res, params, layouts, presets))
    }

    /// Delete a plugin at `idx` from the server's processor chain (`MessageType::DelPlugin`).
    pub fn del_plugin(&self, idx: i32) -> Result<i32, String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        let msg = RawMessage::new(MessageType::DelPlugin, idx.to_le_bytes().to_vec());
        msg.send(sock).map_err(|e| e.to_string())?;
        let resp = MessageFactory::get_next_message(sock, None).map_err(|e| e.to_string())?;
        if resp.payload.len() >= 4 {
            let lat = i32::from_le_bytes(resp.payload[0..4].try_into().unwrap());
            self.latency_samples.store(lat, Ordering::Relaxed);
            Ok(lat)
        } else {
            Ok(self.latency_samples())
        }
    }

    /// Open a plugin's UI editor on the server (`MessageType::EditPlugin`).
    pub fn edit_plugin(&self, idx: i32, x: i32, y: i32) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        let mut payload = Vec::with_capacity(12);
        payload.extend_from_slice(&idx.to_le_bytes());
        payload.extend_from_slice(&x.to_le_bytes());
        payload.extend_from_slice(&y.to_le_bytes());
        RawMessage::new(MessageType::EditPlugin, payload)
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Hide the active plugin editor on the server (`MessageType::HidePlugin`).
    pub fn hide_plugin(&self) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        RawMessage::new(MessageType::HidePlugin, Vec::new())
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Fetch raw binary state of plugin `idx` (`MessageType::GetPluginSettings`).
    pub fn get_plugin_settings(&self, idx: i32) -> Result<Vec<u8>, String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        RawMessage::new(MessageType::GetPluginSettings, idx.to_le_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())?;
        let resp = MessageFactory::get_next_message(sock, None).map_err(|e| e.to_string())?;
        Ok(resp.payload)
    }

    /// Restore raw binary state of plugin `idx` (`MessageType::SetPluginSettings`).
    pub fn set_plugin_settings(&self, idx: i32, settings: &[u8]) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        RawMessage::new(MessageType::SetPluginSettings, idx.to_le_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())?;
        RawMessage::new(MessageType::PluginSettings, settings.to_vec())
            .send(sock)
            .map_err(|e| e.to_string())?;
        if self.state_ack.load(Ordering::Relaxed) {
            let reply = MessageFactory::get_next_message(sock, None).map_err(|e| e.to_string())?;
            if reply.msg_type_enum() != MessageType::Result { return Err("Unexpected state restore acknowledgement".into()); }
            let result: serde_json::Value = serde_json::from_slice(&reply.payload).map_err(|e| e.to_string())?;
            if result.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                return Err(result.get("error").and_then(|v| v.as_str()).unwrap_or("State restore failed").into());
            }
        }
        Ok(())
    }

    /// Bypass plugin `idx` (`MessageType::BypassPlugin`).
    pub fn bypass_plugin(&self, idx: i32) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        RawMessage::new(MessageType::BypassPlugin, idx.to_le_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Unbypass plugin `idx` (`MessageType::UnbypassPlugin`).
    pub fn unbypass_plugin(&self, idx: i32) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        RawMessage::new(MessageType::UnbypassPlugin, idx.to_le_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Set per-plugin oversampling factor (`1`, `2`, or `4`) on the server
    /// (`MessageType::SetPluginOversampling = 75`, commit `559863f`).
    pub fn set_plugin_oversampling(&self, idx: i32, factor: i32) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        let payload = OversamplingPayload { idx, factor };
        RawMessage::new(MessageType::SetPluginOversampling, payload.to_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Swap plugins `idx_a` and `idx_b` in the chain (`MessageType::ExchangePlugins`).
    pub fn exchange_plugins(&self, idx_a: i32, idx_b: i32) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        let payload = ExchangePayload { idx_a, idx_b };
        RawMessage::new(MessageType::ExchangePlugins, payload.to_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Fetch recently used plugins from the server (`MessageType::RecentsList`).
    pub fn get_recents(&self) -> Result<Vec<ServerPlugin>, String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        RawMessage::new(MessageType::RecentsList, Vec::new())
            .send(sock)
            .map_err(|e| e.to_string())?;
        let resp = MessageFactory::get_next_message(sock, None).map_err(|e| e.to_string())?;
        Ok(Self::parse_plugin_list(resp.as_str()))
    }

    /// Select preset `preset` on plugin `idx` (`MessageType::Preset`).
    pub fn set_preset(&self, idx: i32, preset: i32) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        let payload = PresetPayload { idx, preset };
        RawMessage::new(MessageType::Preset, payload.to_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Query current normalized value of `param_idx` on plugin `idx` (`MessageType::GetParameterValue`).
    pub fn get_parameter_value(&self, idx: i32, param_idx: i32) -> Result<f32, String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        let req = ParameterValuePayload {
            idx,
            param_idx,
            value: 0.0,
        };
        RawMessage::new(MessageType::GetParameterValue, req.to_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())?;
        let resp = MessageFactory::get_next_message(sock, None).map_err(|e| e.to_string())?;
        let parsed = ParameterValuePayload::from_bytes(&resp.payload)
            .ok_or_else(|| "invalid ParameterValue response".to_string())?;
        Ok(parsed.value)
    }

    /// Set normalized value of `param_idx` on plugin `idx` (`MessageType::ParameterValue`).
    pub fn set_parameter_value(&self, idx: i32, param_idx: i32, value: f32) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        let req = ParameterValuePayload {
            idx,
            param_idx,
            value,
        };
        RawMessage::new(MessageType::ParameterValue, req.to_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Fetch all parameter values for plugin `idx` (`MessageType::GetAllParameterValues`).
    pub fn get_all_parameter_values(&self, idx: i32) -> Result<Vec<(i32, f32)>, String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        RawMessage::new(MessageType::GetAllParameterValues, idx.to_le_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())?;
        let resp = MessageFactory::get_next_message(sock, None).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for chunk in resp.payload.chunks_exact(ParameterValuePayload::SIZE) {
            if let Some(p) = ParameterValuePayload::from_bytes(chunk) {
                out.push((p.param_idx, p.value));
            }
        }
        Ok(out)
    }

    /// Update screen capture bounds (`MessageType::ScreenCaptureArea`).
    pub fn update_screen_capture_area(&self, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        let payload = ScreenBoundsPayload { x, y, w, h };
        RawMessage::new(MessageType::ScreenCaptureArea, payload.to_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Request plugin rescan on server (`MessageType::Rescan`).
    pub fn rescan(&self, wipe: bool) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        let val: i32 = if wipe { 1 } else { 0 };
        RawMessage::new(MessageType::Rescan, val.to_le_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Request server restart (`MessageType::Restart`).
    pub fn restart(&self) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        RawMessage::new(MessageType::Restart, Vec::new())
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Poll server CPU load (`MessageType::CpuLoad`).
    pub fn update_cpu_load(&self) -> Result<f32, String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        RawMessage::new(MessageType::CpuLoad, Vec::new())
            .send(sock)
            .map_err(|e| e.to_string())?;
        let resp = MessageFactory::get_next_message(sock, None).map_err(|e| e.to_string())?;
        if let Some(cpu) = CpuLoadPayload::from_bytes(&resp.payload) {
            self.server_cpu_load
                .store(cpu.val.to_bits(), Ordering::Relaxed);
            Ok(cpu.val)
        } else {
            Err("invalid CpuLoad response".to_string())
        }
    }

    /// Forward mouse event to remote plugin UI (`MessageType::Mouse`).
    pub fn send_mouse_event(
        &self,
        ev_type: MouseEvType,
        x: f32,
        y: f32,
        is_shift_down: bool,
        is_ctrl_down: bool,
        is_alt_down: bool,
        delta_x: f32,
        delta_y: f32,
        is_smooth: bool,
    ) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        let payload = MousePayload {
            ev_type,
            x,
            y,
            is_shift_down,
            is_ctrl_down,
            is_alt_down,
            delta_x,
            delta_y,
            is_smooth,
        };
        RawMessage::new(MessageType::Mouse, payload.to_bytes().to_vec())
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Forward keyboard event to remote plugin UI (`MessageType::Key`).
    pub fn send_key_event(&self, key_codes: &[u16]) -> Result<(), String> {
        let mut guard = self.cmd_out.lock();
        let sock = guard.as_mut().ok_or("not connected")?;
        let payload = KeyPayload {
            key_codes: key_codes.to_vec(),
        };
        RawMessage::new(MessageType::Key, payload.to_bytes())
            .send(sock)
            .map_err(|e| e.to_string())
    }

    /// Stream an `f32` audio + MIDI block to the server and receive processed output + updated latency.
    pub fn process_audio_f32(
        &self,
        audio: &mut [Vec<f32>],
        midi: &mut Vec<MidiEvent>,
        pos: &PositionInfo,
    ) -> Result<i32, String> {
        let mut guard = self.audio_stream.lock();
        let sock = guard.as_mut().ok_or("audio socket not connected")?;
        let ch_req = audio.len() as i32;
        let samples_req = audio.first().map(|c| c.len() as i32).unwrap_or(0);
        let lat = send_and_receive_f32(sock, audio, midi, pos, ch_req, samples_req)
            .map_err(|e| format!("audio f32 stream error: {}", e))?;
        self.latency_samples.store(lat, Ordering::Relaxed);
        Ok(lat)
    }

    /// Stream an `f64` audio + MIDI block to the server and receive processed output + updated latency.
    pub fn process_audio_f64(
        &self,
        audio: &mut [Vec<f64>],
        midi: &mut Vec<MidiEvent>,
        pos: &PositionInfo,
    ) -> Result<i32, String> {
        let mut guard = self.audio_stream.lock();
        let sock = guard.as_mut().ok_or("audio socket not connected")?;
        let ch_req = audio.len() as i32;
        let samples_req = audio.first().map(|c| c.len() as i32).unwrap_or(0);
        let lat = send_and_receive_f64(sock, audio, midi, pos, ch_req, samples_req)
            .map_err(|e| format!("audio f64 stream error: {}", e))?;
        self.latency_samples.store(lat, Ordering::Relaxed);
        Ok(lat)
    }

    /// Poll a single message from `cmdIn` (e.g., `Latency`, `ParameterValue`, `Alive`).
    pub fn poll_cmd_in(&self) -> Option<RawMessage> {
        let mut guard = self.cmd_in.lock();
        let sock = guard.as_mut()?;
        let _ = sock.set_read_timeout(Some(Duration::from_millis(5)));
        let msg = MessageFactory::get_next_message(sock, None).ok()?;
        if msg.msg_type_enum() == MessageType::Latency && msg.payload.len() >= 4 {
            let lat = i32::from_le_bytes(msg.payload[0..4].try_into().unwrap());
            self.latency_samples.store(lat, Ordering::Relaxed);
        }
        Some(msg)
    }
}
