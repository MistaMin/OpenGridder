//! Client side of AudioGridder's four-socket worker.
//!
//! The real client connects command-out, command-in, audio, then screen.
//! Request replies come back on command-out, the socket this session writes.
//! The worker pushes the plugin list on that socket as soon as the four
//! sockets are up. Command-in is read for plugin status, gestures, parameter
//! values, hide, and keys. The screen socket is read for editor frames.
//! Mouse and key messages are written on command-out while that editor is open.
//! The last audio-response latency is kept for the insert. `Plugin::process`
//! does not use this type.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use og_protocol::command::{
    self, ADD_PLUGIN, ADD_PLUGIN_RESULT, BYPASS_PLUGIN, CPU_LOAD, DEL_PLUGIN, EDIT_PLUGIN, EXCHANGE_PLUGINS, GET_PARAMETER_VALUE,
    GET_PLUGIN_SETTINGS, KEY, SET_PLUGIN_SETTINGS, MOUSE, PARAMETER_VALUE, PARAMETERS, PLUGIN_LIST, PLUGIN_SETTINGS, PRESET, PRESETS, RESULT,
    SCREEN_CAPTURE, SET_MONO_CHANNELS, SET_PLUGIN_OVERSAMPLING, UNBYPASS_PLUGIN,
};
use og_protocol::{
    read_audio_response, AudioRequestHeader, AudioResponseHeader, HandshakeRequest, HandshakeResponse, PositionInfo,
    PROTOCOL_VERSION,
};
use openplugin::prelude::Plugin;

/// A new socket starts here. Command, command-in, and screen wait up to 45 s for heavy plug-in cold loads (e.g. Acustica Audio / Kontakt).
/// Audio waits 500 ms so a stalled block cannot hold the host.
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
const AUDIO_TIMEOUT: Duration = Duration::from_millis(500);

/// Id of the alpha host's builtin gain. The server rejects every other id.
pub const BUILTIN_GAIN_ID: &str = "Builtin-gain";

pub struct ScreenFrame {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
    /// Where `pixels` (an opaque PNG or JPEG of that rectangle) goes in the full picture.
    /// `None`: a whole-picture PNG whose transparent pixels keep the previous picture.
    pub rect: Option<(i32, i32, i32, i32)>,
}

/// One hosted parameter from the worker's Parameters message.
#[derive(Clone, Debug)]
pub struct RemoteParam {
    pub idx: i32,
    pub name: String,
    pub default_value: f32,
    pub current: f32,
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CatalogItem {
    pub name: String,
    pub company: String,
    pub id: String,
    pub format: String,
    pub category: String,
    pub is_instrument: bool,
}

/// One slot in the insert's chain. AudioGridder keeps this list across reconnect
/// and sends bypass again after each plugin is re-added.
#[derive(Clone, Debug, Default)]
pub struct LoadedSlot {
    pub name: String,
    pub id: String,
    pub settings: String,
    pub layout: String,
    pub mono: u64,
    pub bypassed: bool,
    pub oversampling: u32,
    /// Parameters read from the add reply. Empty until that body is kept.
    pub params: Vec<RemoteParam>,
    /// Factory preset names from the add reply, split on a vertical bar.
    pub presets: Vec<String>,
}

pub struct RemoteSession {
    command: TcpStream,
    audio: TcpStream,
    inbound: TcpStream,
    screen: TcpStream,
    latency_samples: i32,
    /// MIDI `sendToClient` wrote after the last `exchange_midi` block.
    response_midi: Vec<crate::QueuedMidi>,
    parameters: Vec<(i32, i32, f32, i32)>,
    /// Last successful AddPlugin result, including layout and monoChannels.
    last_add: serde_json::Value,
    slots: Vec<LoadedSlot>,
    catalog: Vec<String>,
    catalog_items: Vec<CatalogItem>,
    applied_selection: i32,
    pos_wire_size: usize,
}

impl RemoteSession {
    /// Cancellation handles let the control thread interrupt a stalled worker at teardown.
    pub fn shutdown_handles(&self) -> std::io::Result<Vec<TcpStream>> {
        [&self.command, &self.audio, &self.inbound, &self.screen].into_iter().map(TcpStream::try_clone).collect()
    }

    /// Clones of command-in and screen for reader threads that block on them, so pushes
    /// and editor frames are drained as they arrive (an unread socket stalls the server).
    pub fn reader_sockets(&self) -> std::io::Result<(TcpStream, TcpStream)> {
        Ok((self.inbound.try_clone()?, self.screen.try_clone()?))
    }
    /// Handshake on `127.0.0.1:port`, then open the four worker sockets from the answer.
    pub fn connect(port: u16, request: HandshakeRequest) -> std::io::Result<Self> {
        Self::connect_host("127.0.0.1", port, request)
    }

    /// Handshake on `host:port`, then open the four worker sockets on that same host.
    pub fn connect_host(host: &str, port: u16, request: HandshakeRequest) -> std::io::Result<Self> {
        // Resolve once. The server opens the worker port on the address the handshake
        // arrived on, so the worker sockets must use that same address: a .local name
        // resolves to several (IPv6, link-local, every interface) and a fresh lookup
        // could pick one the worker is not listening on ("Connection refused").
        let (mut handshake, server) = connect_with_timeout(host, port)?;
        handshake.set_read_timeout(Some(IO_TIMEOUT))?;
        handshake.set_write_timeout(Some(IO_TIMEOUT))?;
        handshake.write_all(&request.to_bytes())?;
        let mut raw = [0u8; HandshakeResponse::WIRE_SIZE];
        handshake.read_exact(&mut raw)?;
        let answer = HandshakeResponse::from_bytes(&raw).ok_or_else(|| invalid("handshake"))?;
        if answer.version != PROTOCOL_VERSION {
            return Err(invalid("protocol"));
        }
        let worker = answer.port as u16;
        drop(handshake);

        let mut sockets = Vec::with_capacity(4);
        for _ in 0..4 {
            let sock = TcpStream::connect_timeout(&std::net::SocketAddr::new(server.ip(), worker), Duration::from_secs(3))?;
            sock.set_read_timeout(Some(IO_TIMEOUT))?;
            sock.set_write_timeout(Some(IO_TIMEOUT))?;
            sockets.push(sock);
        }
        let command = sockets.remove(0);
        let inbound = sockets.remove(0);
        let audio = sockets.remove(0);
        let screen = sockets.remove(0);
        command.set_read_timeout(Some(COMMAND_TIMEOUT))?;
        command.set_write_timeout(Some(COMMAND_TIMEOUT))?;
        inbound.set_read_timeout(Some(COMMAND_TIMEOUT))?;
        inbound.set_write_timeout(Some(COMMAND_TIMEOUT))?;
        screen.set_read_timeout(Some(COMMAND_TIMEOUT))?;
        screen.set_write_timeout(Some(COMMAND_TIMEOUT))?;
        audio.set_read_timeout(Some(AUDIO_TIMEOUT))?;
        audio.set_write_timeout(Some(AUDIO_TIMEOUT))?;
        let mut session = Self {
            command,
            audio,
            inbound,
            screen,
            latency_samples: 0,
            response_midi: Vec::new(),
            parameters: Vec::new(),
            last_add: serde_json::Value::Null,
            slots: Vec::new(),
            catalog: Vec::new(),
            catalog_items: Vec::new(),
            applied_selection: 0,
            pos_wire_size: if port == 55057 { 104 } else { PositionInfo::WIRE_SIZE },
        };
        let (message_type, body) = session.read_cmd()?;
        if message_type != PLUGIN_LIST {
            return Err(invalid("plugin list"));
        }
        let packed = command::read_length_prefixed(&body).ok_or_else(|| invalid("plugin list"))?;
        if packed.is_empty() {
            return Err(invalid("plugin list"));
        }
        session.catalog = plugin_ids(packed);
        session.catalog_items = parse_catalog(packed);
        Ok(session)
    }

    /// Ask for the plugin list again (the server's background scan may have grown it).
    pub fn refresh_catalog(&mut self) -> std::io::Result<()> {
        self.write_cmd(PLUGIN_LIST, &[])?;
        let (message_type, body) = self.read_cmd()?;
        if message_type != PLUGIN_LIST {
            return Err(invalid("plugin list"));
        }
        let packed = command::read_length_prefixed(&body).ok_or_else(|| invalid("plugin list"))?;
        self.catalog = plugin_ids(packed);
        self.catalog_items = parse_catalog(packed);
        Ok(())
    }

    /// Full catalog items from the connect-time list.
    pub fn catalog_items(&self) -> &[CatalogItem] {
        &self.catalog_items
    }

    /// Plugin ids from the connect-time list, in catalog order.
    pub fn catalog_ids(&self) -> &[String] {
        &self.catalog
    }

    /// Use the 104-byte position record instead of the 48-byte record.
    pub fn set_legacy_wire(&mut self, legacy: bool) {
        self.pos_wire_size = if legacy { 104 } else { PositionInfo::WIRE_SIZE };
    }

    /// When selection changes to n > 0, load catalog plugin n-1. Selection 0 does not unload.
    pub fn apply_selection(&mut self, plugin: &crate::AudioGridder) -> std::io::Result<Option<String>> {
        let selected = plugin.param("selection").unwrap_or(0.0).round() as i32;
        if selected == self.applied_selection || selected <= 0 {
            return Ok(None);
        }
        let index = (selected as usize).saturating_sub(1);
        let Some(id) = self.catalog.get(index).cloned() else {
            self.applied_selection = selected;
            return Ok(None);
        };
        self.add_plugin(&id, "")?;
        self.applied_selection = selected;
        Ok(Some(id))
    }

    /// Ask the worker for its host CPU percent. This is the command path, not mDNS.
    /// The audio callback does not call this.
    pub fn cpu_load(&mut self) -> std::io::Result<f32> {
        self.write_cmd(CPU_LOAD, &[])?;
        let (message_type, body) = self.read_cmd()?;
        if message_type != CPU_LOAD {
            return Err(invalid("cpu"));
        }
        command::read_float(&body).filter(|load| load.is_finite()).ok_or_else(|| invalid("cpu"))
    }

    /// Load one plugin by server id. Layout is Stereo and the channel mask is 0.
    /// A failed add does not read past the result.
    pub fn add_plugin(&mut self, id: &str, settings: &str) -> std::io::Result<()> {
        self.add_plugin_with(id, settings, "Stereo", 0)
    }

    /// AddPlugin JSON: id, settings, layout, and monoChannels.
    pub fn add_plugin_with(&mut self, id: &str, settings: &str, layout: &str, mono_channels: u64) -> std::io::Result<()> {
        let body = command::json_payload(&serde_json::json!({
            "id": id,
            "settings": settings,
            "layout": layout,
            "monoChannels": mono_channels
        }));
        self.write_cmd(ADD_PLUGIN, &body)?;
        let (message_type, body) = self.read_cmd()?;
        if message_type != ADD_PLUGIN_RESULT {
            return Err(invalid("add result"));
        }
        let result = command::read_json_payload(&body).ok_or_else(|| invalid("add json"))?;
        if result.get("success").and_then(|value| value.as_bool()) != Some(true) {
            let err = result.get("err").and_then(|value| value.as_str()).unwrap_or("add failed");
            // A refusal is a complete reply (ErrorKind::Other): the session stays usable.
            return Err(refused(err));
        }
        if let Some(latency) = result.get("latency").and_then(|value| value.as_i64()) {
            self.latency_samples = latency as i32;
        }
        self.last_add = result;
        let (message_type, preset_body) = self.read_cmd()?;
        if message_type != PRESETS {
            return Err(invalid("presets"));
        }
        let presets = preset_names(&preset_body).ok_or_else(|| invalid("presets"))?;
        let (message_type, param_body) = self.read_cmd()?;
        if message_type != PARAMETERS {
            return Err(invalid("parameters"));
        }
        let params = parameter_list(&param_body).ok_or_else(|| invalid("parameters"))?;
        let name = self.last_add.get("name").and_then(|value| value.as_str()).unwrap_or(id).to_string();
        self.slots.push(LoadedSlot {
            name,
            id: id.to_string(),
            settings: settings.to_string(),
            layout: layout.to_string(),
            mono: mono_channels,
            bypassed: false,
            oversampling: 1,
            params,
            presets,
        });
        Ok(())
    }

    /// Parameters kept from the add reply for this slot. Channel 0.
    pub fn slot_parameters(&self, index: usize) -> &[RemoteParam] {
        self.slots.get(index).map(|slot| slot.params.as_slice()).unwrap_or(&[])
    }

    /// Factory preset names kept from the add reply for this slot.
    pub fn slot_presets(&self, index: usize) -> &[String] {
        self.slots.get(index).map(|slot| slot.presets.as_slice()).unwrap_or(&[])
    }

    /// Preset on the request socket. AudioGridder does not wait for a reply.
    /// Channel 0 selects the program. Any other channel is left alone by the worker.
    pub fn set_preset(&mut self, index: i32, preset: i32, channel: i32) -> std::io::Result<()> {
        let mut body = [0u8; 12];
        body[0..4].copy_from_slice(&index.to_le_bytes());
        body[4..8].copy_from_slice(&preset.to_le_bytes());
        body[8..12].copy_from_slice(&channel.to_le_bytes());
        self.write_cmd(PRESET, &body)
    }

    /// Slots loaded on this connection, in chain order.
    pub fn loaded(&self) -> &[LoadedSlot] {
        &self.slots
    }

    /// Delete one slot. The result's return code is the chain latency.
    pub fn unload_plugin(&mut self, index: i32) -> std::io::Result<()> {
        if index < 0 || index as usize >= self.slots.len() {
            return Err(invalid("unload"));
        }
        self.write_cmd(DEL_PLUGIN, &command::number(index))?;
        let (message_type, body) = self.read_cmd()?;
        if message_type != RESULT {
            return Err(invalid("delete result"));
        }
        let (latency, _) = command::read_result(&body).ok_or_else(|| invalid("delete result"))?;
        self.latency_samples = latency;
        self.slots.remove(index as usize);
        Ok(())
    }

    /// Bypass one slot. AudioGridder does not wait for a reply.
    pub fn bypass_plugin(&mut self, index: i32) -> std::io::Result<()> {
        self.set_slot_bypass(index, true)
    }

    /// Clear one slot's bypass. AudioGridder does not wait for a reply.
    pub fn unbypass_plugin(&mut self, index: i32) -> std::io::Result<()> {
        self.set_slot_bypass(index, false)
    }

    /// Swap this slot with the one above it.
    pub fn move_up(&mut self, index: i32) -> std::io::Result<()> {
        self.exchange_plugins(index, index - 1)
    }

    /// Swap this slot with the one below it.
    pub fn move_down(&mut self, index: i32) -> std::io::Result<()> {
        self.exchange_plugins(index, index + 1)
    }

    /// Re-add saved slots and resend bypass. Used when the worker connects again.
    /// Rebuild a saved chain. A plugin this server refuses (not installed here, failed to
    /// load) is skipped and named in the result; it must not keep the insert from connecting.
    pub fn replay(&mut self, saved: &[LoadedSlot]) -> std::io::Result<Vec<String>> {
        let mut skipped = Vec::new();
        for slot in saved {
            match self.add_plugin_with(&slot.id, &slot.settings, &slot.layout, slot.mono) {
                Ok(()) => {}
                Err(err) if is_refusal(&err) => {
                    skipped.push(if slot.name.is_empty() { slot.id.clone() } else { slot.name.clone() });
                    continue;
                }
                Err(err) => return Err(err),
            }
            if slot.bypassed {
                let index = self.slots.len() as i32 - 1;
                self.bypass_plugin(index)?;
            }
        }
        Ok(skipped)
    }

    fn set_slot_bypass(&mut self, index: i32, bypassed: bool) -> std::io::Result<()> {
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return Err(invalid("bypass"));
        };
        slot.bypassed = bypassed;
        let message = if bypassed { BYPASS_PLUGIN } else { UNBYPASS_PLUGIN };
        self.write_cmd(message, &command::number(index))
    }

    fn exchange_plugins(&mut self, index: i32, other: i32) -> std::io::Result<()> {
        let count = self.slots.len() as i32;
        if index < 0 || other < 0 || index >= count || other >= count || index == other {
            return Err(invalid("exchange"));
        }
        self.write_cmd(EXCHANGE_PLUGINS, &command::pair_i32(index, other))?;
        self.slots.swap(index as usize, other as usize);
        Ok(())
    }

    /// Load the builtin gain. `gain_db` is the server's starting gain.
    pub fn add_builtin_gain(&mut self, gain_db: f32) -> std::io::Result<()> {
        self.add_plugin(BUILTIN_GAIN_ID, &format!("{gain_db}"))
    }

    /// Samples reported by the last add result or audio response.
    pub fn latency_samples(&self) -> i32 {
        self.latency_samples
    }

    /// AddPlugin result from the last successful add.
    pub fn add_result(&self) -> &serde_json::Value {
        &self.last_add
    }

    /// SetMonoChannels. The next audio block uses this mask.
    pub fn set_mono_channels(&mut self, index: i32, channels: u64) -> std::io::Result<()> {
        self.write_cmd(SET_MONO_CHANNELS, &command::mono_channels(index, channels))
    }

    /// GetPluginSettings. The chunk includes the playhead from the last audio block.
    /// Re-read every slot's settings from the server so a reconnect replay and the DAW state
    /// carry the plugin's current values, not the ones it was added with.
    pub fn refresh_settings(&mut self) -> std::io::Result<()> {
        for index in 0..self.slots.len() {
            let mut value = self.plugin_settings(index as i32)?;
            if let Some(object) = value.as_object_mut() {
                object.remove("playhead");
            }
            if let Some(slot) = self.slots.get_mut(index) {
                // Keep the parameter values the client shows in step with the server's.
                if let Some(values) = value.get("params").and_then(|v| v.as_array()) {
                    for (i, current) in values.iter().enumerate() {
                        if let (Some(current), Some(param)) = (current.as_f64(), slot.params.iter_mut().find(|p| p.idx == i as i32)) {
                            param.current = current as f32;
                        }
                    }
                }
                slot.settings = value.to_string();
            }
        }
        Ok(())
    }

    pub fn plugin_settings(&mut self, index: i32) -> std::io::Result<serde_json::Value> {
        self.write_cmd(GET_PLUGIN_SETTINGS, &command::number(index))?;
        let (message_type, body) = self.read_cmd()?;
        if message_type != PLUGIN_SETTINGS {
            return Err(invalid("settings"));
        }
        let bytes = command::read_length_prefixed(&body).ok_or_else(|| invalid("settings"))?;
        let text = std::str::from_utf8(bytes).map_err(|_| invalid("settings"))?;
        serde_json::from_str(text).map_err(|_| invalid("settings"))
    }

    /// GetParameterValue for one channel of a slot. Multi-mono instances differ.
    pub fn parameter_on(&mut self, index: i32, param: i32, channel: i32) -> std::io::Result<f32> {
        let mut body = [0u8; 12];
        body[..4].copy_from_slice(&index.to_le_bytes());
        body[4..8].copy_from_slice(&param.to_le_bytes());
        body[8..12].copy_from_slice(&channel.to_le_bytes());
        self.write_cmd(GET_PARAMETER_VALUE, &body)?;
        let (message_type, body) = self.read_cmd()?;
        if message_type != PARAMETER_VALUE {
            return Err(invalid("parameter"));
        }
        let (_, _, value, got) = command::read_parameter_value(&body).ok_or_else(|| invalid("parameter"))?;
        if got != channel {
            return Err(invalid("parameter channel"));
        }
        Ok(value)
    }

    /// Last parameter value read from command-in for this slot and parameter.
    pub fn parameter_push(&self, index: i32, param: i32) -> Option<f32> {
        self.parameters.iter().rev().find(|item| item.0 == index && item.1 == param).map(|item| item.2)
    }

    /// One command-in message, or `None` when the socket is idle.
    pub fn poll_inbound(&mut self) -> std::io::Result<Option<(i32, Vec<u8>)>> {
        let Some(message) = try_read_message(&mut self.inbound)? else {
            return Ok(None);
        };
        if message.0 == PARAMETER_VALUE {
            self.remember_parameter(&message.1);
        }
        Ok(Some(message))
    }

    /// One screen-socket frame, or `None` when the socket is idle.
    pub fn poll_screen(&mut self) -> std::io::Result<Option<ScreenFrame>> {
        let Some((message_type, body)) = try_read_message(&mut self.screen)? else {
            return Ok(None);
        };
        if message_type != SCREEN_CAPTURE {
            return Err(invalid("screen capture"));
        }
        Ok(parse_screen(&body))
    }

    /// Open the remote editor. The next screen-socket message is its frame.
    pub fn edit_plugin(&mut self, index: i32) -> std::io::Result<()> {
        self.write_cmd(EDIT_PLUGIN, &command::edit_plugin(index, 0, 0, 0))
    }

    /// Open the editor with its window's top-left at (x, y) in top-left-origin screen
    /// coordinates (`ProcessorWindow::move`); 0, 0 lets the server place it.
    pub fn edit_plugin_at(&mut self, index: i32, x: i32, y: i32) -> std::io::Result<()> {
        self.write_cmd(EDIT_PLUGIN, &command::edit_plugin(index, 0, x, y))
    }

    pub fn hide_plugin(&mut self) -> std::io::Result<()> {
        self.write_cmd(command::HIDE_PLUGIN, &[])
    }

    /// `ParameterValue` on command-out. The server does not echo it.
    /// The stored parameter list keeps the value that was sent.
    pub fn set_parameter(&mut self, index: i32, param: i32, value: f32) -> std::io::Result<()> {
        self.write_cmd(PARAMETER_VALUE, &command::parameter_value(index, param, value, 0))?;
        if let Some(slot) = self.slots.get_mut(index as usize) {
            if let Some(item) = slot.params.iter_mut().find(|item| item.idx == param) {
                item.current = value;
            }
        }
        Ok(())
    }

    /// Send the insert's queued automation-slot writes. Returns how many were sent.
    pub fn apply_slot_writes(&mut self, plugin: &mut crate::AudioGridder) -> std::io::Result<usize> {
        let writes = plugin.take_slot_writes();
        let count = writes.len();
        for write in writes {
            self.write_cmd(PARAMETER_VALUE, &command::parameter_value(write.plugin, write.param, write.value, write.channel))?;
        }
        Ok(count)
    }

    /// `Mouse` on command-out. Kind 1 is left-down.
    pub fn send_mouse(&mut self, kind: u8, x: f32, y: f32) -> std::io::Result<()> {
        let mut body = [0u8; 28];
        body[0] = kind;
        body[4..8].copy_from_slice(&x.to_le_bytes());
        body[8..12].copy_from_slice(&y.to_le_bytes());
        self.write_cmd(MOUSE, &body)
    }

    /// `Mouse` with modifiers and wheel deltas (`mouseevent_t`): type, x, y, shift, ctrl,
    /// alt, then deltaX at byte 16 and deltaY at byte 20.
    #[allow(clippy::too_many_arguments)]
    pub fn send_mouse_event(&mut self, kind: u8, x: f32, y: f32, shift: bool, ctrl: bool, alt: bool, dx: f32, dy: f32) -> std::io::Result<()> {
        let mut body = [0u8; 28];
        body[0] = kind;
        body[4..8].copy_from_slice(&x.to_le_bytes());
        body[8..12].copy_from_slice(&y.to_le_bytes());
        body[12] = shift as u8;
        body[13] = ctrl as u8;
        body[14] = alt as u8;
        body[16..20].copy_from_slice(&dx.to_le_bytes());
        body[20..24].copy_from_slice(&dy.to_le_bytes());
        self.write_cmd(MOUSE, &body)
    }

    /// `Key` on command-out. Codes are little-endian uint16 values.
    pub fn send_key(&mut self, codes: &[u16]) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity(codes.len() * 2);
        for code in codes {
            bytes.extend_from_slice(&code.to_le_bytes());
        }
        self.write_cmd(KEY, &command::length_prefixed(&bytes))
    }

    /// `SetPluginOversampling`: two little-endian i32s, index then factor (1, 2, or 4).
    /// SetPluginSettings then PluginSettings (AudioGridder's pair): the server applies these
    /// settings, as GetPluginSettings returned them, to the running slot. No reply.
    pub fn apply_settings(&mut self, index: i32, settings: &str) -> std::io::Result<()> {
        self.write_cmd(SET_PLUGIN_SETTINGS, &command::number(index))?;
        self.write_cmd(PLUGIN_SETTINGS, &command::length_prefixed(settings.as_bytes()))?;
        if let Some(slot) = self.slots.get_mut(index as usize) {
            slot.settings = settings.to_string();
        }
        Ok(())
    }

    pub fn set_oversampling(&mut self, index: i32, factor: i32) -> std::io::Result<()> {
        let mut body = [0u8; 8];
        body[..4].copy_from_slice(&index.to_le_bytes());
        body[4..].copy_from_slice(&factor.to_le_bytes());
        self.write_cmd(SET_PLUGIN_OVERSAMPLING, &body)?;
        if let Some(slot) = self.slots.get_mut(index as usize) {
            slot.oversampling = factor.max(1) as u32;
        }
        Ok(())
    }

    /// Send one planar block and return the server's block. Call this off the audio thread.
    /// The playhead is [`PositionInfo::default`].
    pub fn exchange(&mut self, channels: &[Vec<f32>]) -> std::io::Result<Vec<Vec<f32>>> {
        Ok(self.exchange_measured(channels)?.0)
    }

    /// Same path as [`Self::exchange`], with the host playhead on the audio request.
    pub fn exchange_at(&mut self, channels: &[Vec<f32>], position: PositionInfo) -> std::io::Result<Vec<Vec<f32>>> {
        Ok(self.exchange_measured_at(channels, position)?.0)
    }

    /// Same path as [`Self::exchange`], keeping the latency the server reported.
    pub fn exchange_measured(&mut self, channels: &[Vec<f32>]) -> std::io::Result<(Vec<Vec<f32>>, i32)> {
        self.exchange_measured_at(channels, PositionInfo::default())
    }

    /// Audio request with the insert's queued MIDI after the samples.
    /// Each event is a little-endian sample offset, a size of 3, and 3 bytes.
    /// The returned count is the number of events written on this request.
    pub fn exchange_midi(&mut self, channels: &[Vec<f32>], midi: &[crate::QueuedMidi]) -> std::io::Result<(Vec<Vec<f32>>, i32)> {
        self.exchange_midi_at(channels, midi, PositionInfo::default())
    }

    /// MIDI and the block's transport travel together on the audio socket.
    pub fn exchange_midi_at(&mut self, channels: &[Vec<f32>], midi: &[crate::QueuedMidi], position: PositionInfo) -> std::io::Result<(Vec<Vec<f32>>, i32)> {
        let (audio, latency, response_midi) = exchange_audio(&mut self.audio, self.pos_wire_size, channels, midi, position)?;
        self.latency_samples = latency;
        self.response_midi = response_midi;
        Ok((audio, midi.len() as i32))
    }

    /// The audio socket on its own, for a dedicated audio thread: audio keeps flowing while
    /// this session waits on a slow command (AudioGridder's separate AudioStreamer).
    pub fn audio_link(&self) -> std::io::Result<AudioLink> {
        Ok(AudioLink {
            audio: self.audio.try_clone()?,
            pos_wire_size: self.pos_wire_size,
            latency_samples: self.latency_samples,
            inbox: Vec::new(),
            late_since: None,
        })
    }

    /// MIDI events the server returned on the last [`Self::exchange_midi`] call.
    pub fn response_midi(&self) -> &[crate::QueuedMidi] {
        &self.response_midi
    }

    /// Audio request plus the playhead AudioGridder sends with every block.
    pub fn exchange_measured_at(
        &mut self,
        channels: &[Vec<f32>],
        position: PositionInfo,
    ) -> std::io::Result<(Vec<Vec<f32>>, i32)> {
        let (audio, _) = self.exchange_midi_at(channels, &[], position)?;
        Ok((audio, self.latency_samples))
    }

    fn remember_parameter(&mut self, body: &[u8]) {
        let Some((index, param, value, channel)) = command::read_parameter_value(body) else {
            return;
        };
        if let Some(slot) = self.parameters.iter_mut().find(|item| item.0 == index && item.1 == param && item.3 == channel) {
            slot.2 = value;
        } else {
            self.parameters.push((index, param, value, channel));
        }
    }

    fn write_cmd(&mut self, message_type: i32, body: &[u8]) -> std::io::Result<()> {
        self.command.write_all(&command::encode(message_type, body))
    }

    fn read_cmd(&mut self) -> std::io::Result<(i32, Vec<u8>)> {
        read_message(&mut self.command)
    }
}

fn read_response_midi(socket: &mut TcpStream, count: i32) -> std::io::Result<Vec<crate::QueuedMidi>> {
    if count < 0 {
        return Err(invalid("midi size"));
    }
    let mut events = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut header = [0u8; 8];
        socket.read_exact(&mut header)?;
        let sample_offset = i32::from_le_bytes(header[0..4].try_into().unwrap());
        let size = i32::from_le_bytes(header[4..8].try_into().unwrap());
        if !(0..=65_536).contains(&size) {
            return Err(invalid("midi size"));
        }
        let mut data = vec![0u8; size as usize];
        if size > 0 {
            socket.read_exact(&mut data)?;
        }
        if data.len() >= 3 {
            let mut bytes = [0u8; 3];
            bytes.copy_from_slice(&data[..3]);
            events.push(crate::QueuedMidi { sample_offset, data: bytes });
        }
    }
    Ok(events)
}

fn read_message(socket: &mut TcpStream) -> std::io::Result<(i32, Vec<u8>)> {
    match try_read_message(socket)? {
        Some(message) => Ok(message),
        None => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "command")),
    }
}

/// The audio socket for the audio thread. One block is in flight at a time.
///
/// A late block is not a lost connection: when the server misses the deadline (a heavy chain
/// on a busy machine), that block stays dry, the reply is still read when it arrives (bytes
/// that came in are kept, so the stream stays in step) and only then is the next block sent.
/// A server that stays silent for `LATE_LIMIT` is gone.
pub struct AudioLink {
    audio: TcpStream,
    pos_wire_size: usize,
    latency_samples: i32,
    inbox: Vec<u8>,
    late_since: Option<std::time::Instant>,
}

/// How long a late reply is waited for before the connection counts as lost.
const LATE_LIMIT: Duration = Duration::from_secs(5);

impl AudioLink {
    /// `Ok(Some(audio))`: the processed block. `Ok(None)`: the server is late, play this block
    /// dry. `Err`: the connection is gone.
    pub fn exchange(
        &mut self,
        channels: &[Vec<f32>],
        midi: &[crate::QueuedMidi],
        position: PositionInfo,
    ) -> std::io::Result<Option<Vec<Vec<f32>>>> {
        if let Some(since) = self.late_since {
            match self.receive()? {
                Some(_) => self.late_since = None, // the late block: too late to use
                None if since.elapsed() > LATE_LIMIT => {
                    return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the server stopped answering audio"));
                }
                None => return Ok(None),
            }
        }
        let request = audio_request(self.pos_wire_size, channels, midi, position)?;
        self.audio.write_all(&request)?;
        match self.receive()? {
            Some((audio, latency)) => {
                self.latency_samples = latency;
                Ok(Some(audio))
            }
            None => {
                self.late_since = Some(std::time::Instant::now());
                Ok(None)
            }
        }
    }

    /// Read until one whole reply is in, or `AUDIO_TIMEOUT` passes (`None`, bytes kept).
    fn receive(&mut self) -> std::io::Result<Option<(Vec<Vec<f32>>, i32)>> {
        let deadline = std::time::Instant::now() + AUDIO_TIMEOUT;
        let mut chunk = [0u8; 65536];
        loop {
            if let Some((audio, latency, used)) = parse_audio_reply(&self.inbox)? {
                self.inbox.drain(..used);
                return Ok(Some((audio, latency)));
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            self.audio.set_read_timeout(Some((deadline - now).max(Duration::from_millis(1))))?;
            match self.audio.read(&mut chunk) {
                Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "audio connection closed")),
                Ok(n) => self.inbox.extend_from_slice(&chunk[..n]),
                Err(err) if is_wait(&err) => return Ok(None),
                Err(err) => return Err(err),
            }
        }
    }

    pub fn latency_samples(&self) -> i32 {
        self.latency_samples
    }
}

/// One whole audio reply at the start of `bytes`: audio, latency and the bytes it used.
/// `None` while it is still incomplete.
fn parse_audio_reply(bytes: &[u8]) -> std::io::Result<Option<(Vec<Vec<f32>>, i32, usize)>> {
    if bytes.len() < AudioResponseHeader::WIRE_SIZE {
        return Ok(None);
    }
    let header = AudioResponseHeader::from_bytes(bytes).ok_or_else(|| invalid("audio response"))?;
    if !(0..=64).contains(&header.channels) || !(0..=65_536).contains(&header.samples) || header.midi_events < 0 {
        return Err(invalid("audio response"));
    }
    let audio_end = AudioResponseHeader::WIRE_SIZE + header.channels as usize * header.samples as usize * 4;
    let mut at = audio_end;
    for _ in 0..header.midi_events {
        if bytes.len() < at + 8 {
            return Ok(None);
        }
        let size = i32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
        if !(0..=65_536).contains(&size) {
            return Err(invalid("midi size"));
        }
        at += 8 + size as usize;
    }
    if bytes.len() < at {
        return Ok(None);
    }
    let (_header, audio) = read_audio_response(&bytes[..audio_end]).ok_or_else(|| invalid("audio response"))?;
    Ok(Some((audio, header.latency_samples, at)))
}

/// The audio request bytes: header, samples, MIDI, playhead.
fn audio_request(pos_wire_size: usize, channels: &[Vec<f32>], midi: &[crate::QueuedMidi], position: PositionInfo) -> std::io::Result<Vec<u8>> {
    let samples = channels.first().map(|channel| channel.len()).unwrap_or(0);
    let count = channels.len() as i32;
    if samples == 0 || !(1..=2).contains(&count) || channels.iter().any(|channel| channel.len() != samples) {
        return Err(invalid("audio shape"));
    }
    let header = AudioRequestHeader {
        channels: count,
        samples: samples as i32,
        channels_requested: count,
        samples_requested: samples as i32,
        midi_events: midi.len() as i32,
        is_double: false,
        trace_id: [0; 16],
    };
    let mut bytes = Vec::with_capacity(AudioRequestHeader::WIRE_SIZE + channels.len() * samples * 4 + midi.len() * 11 + PositionInfo::WIRE_SIZE);
    bytes.extend_from_slice(&header.to_bytes());
    for channel in channels {
        for sample in channel {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }
    for event in midi {
        bytes.extend_from_slice(&event.sample_offset.to_le_bytes());
        bytes.extend_from_slice(&3i32.to_le_bytes());
        bytes.extend_from_slice(&event.data);
    }
    bytes.extend_from_slice(&position.to_bytes());
    if pos_wire_size > PositionInfo::WIRE_SIZE {
        bytes.resize(bytes.len() + (pos_wire_size - PositionInfo::WIRE_SIZE), 0);
    }
    Ok(bytes)
}

/// Send one block with its MIDI and playhead, read the processed block back.
/// Returns the audio, the server's latency, and the MIDI it returned.
fn exchange_audio(
    socket: &mut TcpStream,
    pos_wire_size: usize,
    channels: &[Vec<f32>],
    midi: &[crate::QueuedMidi],
    position: PositionInfo,
) -> std::io::Result<(Vec<Vec<f32>>, i32, Vec<crate::QueuedMidi>)> {
    let samples = channels.first().map(|channel| channel.len()).unwrap_or(0);
    let count = channels.len() as i32;
    if samples == 0 || !(1..=2).contains(&count) || channels.iter().any(|channel| channel.len() != samples) {
        return Err(invalid("audio shape"));
    }
    let header = AudioRequestHeader {
        channels: count,
        samples: samples as i32,
        channels_requested: count,
        samples_requested: samples as i32,
        midi_events: midi.len() as i32,
        is_double: false,
        trace_id: [0; 16],
    };
    let mut bytes = Vec::with_capacity(AudioRequestHeader::WIRE_SIZE + channels.len() * samples * 4 + midi.len() * 11 + PositionInfo::WIRE_SIZE);
    bytes.extend_from_slice(&header.to_bytes());
    for channel in channels {
        for sample in channel {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }
    for event in midi {
        bytes.extend_from_slice(&event.sample_offset.to_le_bytes());
        bytes.extend_from_slice(&3i32.to_le_bytes());
        bytes.extend_from_slice(&event.data);
    }
    bytes.extend_from_slice(&position.to_bytes());
    if pos_wire_size > PositionInfo::WIRE_SIZE {
        bytes.resize(bytes.len() + (pos_wire_size - PositionInfo::WIRE_SIZE), 0);
    }
    socket.write_all(&bytes)?;
    let mut response = vec![0u8; AudioResponseHeader::WIRE_SIZE + channels.len() * samples * 4];
    socket.read_exact(&mut response)?;
    let response_header = AudioResponseHeader::from_bytes(&response).ok_or_else(|| invalid("audio response"))?;
    let (_header, audio) = read_audio_response(&response).ok_or_else(|| invalid("audio response"))?;
    let response_midi = read_response_midi(socket, response_header.midi_events)?;
    Ok((audio, response_header.latency_samples, response_midi))
}

/// An unreachable network server must not hold the client's worker for the system's
/// TCP timeout (over a minute): try each resolved address for up to 3 s.
/// IPv4 addresses first: the server listens on IPv4, and a .local name lists its IPv6
/// addresses first. Returns the stream and the address that answered.
fn connect_with_timeout(host: &str, port: u16) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    use std::net::ToSocketAddrs;
    let mut addrs: Vec<std::net::SocketAddr> = (host, port).to_socket_addrs()?.collect();
    addrs.sort_by_key(|addr| !addr.is_ipv4());
    let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, format!("{host} did not resolve"));
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
            Ok(stream) => return Ok((stream, addr)),
            Err(err) => last = err,
        }
    }
    Err(last)
}

/// One message from a blocking reader socket; `None` when its read timeout passed.
pub fn read_pending(socket: &mut TcpStream) -> std::io::Result<Option<(i32, Vec<u8>)>> {
    try_read_message(socket)
}

/// A `ScreenCapture` body: width, height, padded size, scale, then the PNG.
pub fn screen_frame(body: &[u8]) -> Option<ScreenFrame> {
    parse_screen(body)
}

fn try_read_message(socket: &mut TcpStream) -> std::io::Result<Option<(i32, Vec<u8>)>> {
    let mut header = [0u8; command::CMD_HEADER];
    match socket.read_exact(&mut header) {
        Ok(()) => {}
        Err(err) if is_wait(&err) => return Ok(None),
        Err(err) => return Err(err),
    }
    let message_type = i32::from_le_bytes(header[0..4].try_into().unwrap());
    let size = i32::from_le_bytes(header[4..8].try_into().unwrap());
    if size < 0 || size as usize > command::MAX_BODY {
        return Err(invalid("command size"));
    }
    let mut body = vec![0u8; size as usize];
    if size > 0 {
        socket.read_exact(&mut body)?;
    }
    Ok(Some((message_type, body)))
}

fn parse_screen(body: &[u8]) -> Option<ScreenFrame> {
    if body.len() < 32 {
        return None;
    }
    let width = i32::from_le_bytes(body[0..4].try_into().ok()?);
    let height = i32::from_le_bytes(body[4..8].try_into().ok()?);
    let size = u64::from_le_bytes(body[24..32].try_into().ok()?) as usize;
    if width <= 0 || height <= 0 || size == 0 || body.len() < 32 + size {
        return None;
    }
    // OpenGridder's "OGRC" trailer: x, y, w, h of the rectangle this image updates.
    let trailer = &body[32 + size..];
    let rect = (trailer.len() >= 20 && &trailer[..4] == b"OGRC").then(|| {
        let word = |at: usize| u32::from_le_bytes(trailer[at..at + 4].try_into().unwrap()) as i32;
        (word(4), word(8), word(12), word(16))
    });
    Some(ScreenFrame { width, height, pixels: body[32..32 + size].to_vec(), rect })
}

fn is_wait(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::TimedOut || err.kind() == std::io::ErrorKind::WouldBlock
}

/// The server answered "no" (for example a plugin it cannot load).
fn refused(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, message.to_string())
}

/// A refusal leaves the command channel in step; every other error may not.
pub fn is_refusal(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::Other
}

fn invalid(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

/// Presets message: length-prefixed names separated by a vertical bar.
fn preset_names(body: &[u8]) -> Option<Vec<String>> {
    let bytes = command::read_length_prefixed(body)?;
    let text = std::str::from_utf8(bytes).ok()?;
    if text.is_empty() {
        return Some(Vec::new());
    }
    let names: Vec<String> = text.split('|').map(str::to_string).collect();
    if names.iter().any(|name| name.is_empty()) {
        return None;
    }
    Some(names)
}

/// Parameters message: length-prefixed msgpack array of name, index, and values.
fn parameter_list(body: &[u8]) -> Option<Vec<RemoteParam>> {
    let bytes = command::read_length_prefixed(body)?;
    let value = command::decode_msgpack(bytes)?;
    let items = value.as_array()?;
    Some(items.iter().filter_map(one_param).collect())
}

fn one_param(value: &serde_json::Value) -> Option<RemoteParam> {
    let idx = value.get("idx")?.as_i64()? as i32;
    let name = value.get("name")?.as_str()?.to_string();
    let default_value = json_f32(value.get("defaultValue"))?;
    let current = json_f32(value.get("currentValue")).unwrap_or(default_value);
    let label = value.get("label").and_then(|item| item.as_str()).unwrap_or("").to_string();
    Some(RemoteParam { idx, name, default_value, current, label })
}

fn json_f32(value: Option<&serde_json::Value>) -> Option<f32> {
    let number = value?.as_f64()?;
    number.is_finite().then_some(number as f32)
}

fn plugin_ids(packed: &[u8]) -> Vec<String> {
    let Some(value) = command::decode_msgpack(packed) else {
        return Vec::new();
    };
    value
        .get("plugins")
        .and_then(|plugins| plugins.as_array())
        .map(|plugins| {
            plugins
                .iter()
                .filter_map(|plugin| plugin.get("id").and_then(|id| id.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_catalog(packed: &[u8]) -> Vec<CatalogItem> {
    let Some(value) = command::decode_msgpack(packed) else {
        return Vec::new();
    };
    value
        .get("plugins")
        .and_then(|plugins| plugins.as_array())
        .map(|plugins| {
            plugins
                .iter()
                .filter_map(|p| {
                    let id = p.get("id").and_then(|v| v.as_str())?.to_string();
                    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or(&id).to_string();
                    let company = p.get("company").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let format = p.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let category = p.get("category").and_then(|v| v.as_str()).unwrap_or("Effect").to_string();
                    let is_instrument = p.get("isInstrument").and_then(|v| v.as_bool()).unwrap_or(false);
                    Some(CatalogItem {
                        name,
                        company,
                        id,
                        format,
                        category,
                        is_instrument,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    use og_protocol::command::PLUGIN_STATUS;
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use og_server::serve_worker;
    use openplugin::prelude::Plugin;

    #[test]
    fn command_and_screen_wait_45s_and_audio_waits_500ms() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();

        let command_read = session.command.read_timeout().unwrap();
        let command_write = session.command.write_timeout().unwrap();
        let inbound_read = session.inbound.read_timeout().unwrap();
        let inbound_write = session.inbound.write_timeout().unwrap();
        let screen_read = session.screen.read_timeout().unwrap();
        let screen_write = session.screen.write_timeout().unwrap();
        let audio_read = session.audio.read_timeout().unwrap();
        let audio_write = session.audio.write_timeout().unwrap();
        eprintln!(
            "timeouts command {command_read:?} {command_write:?} inbound {inbound_read:?} {inbound_write:?} screen {screen_read:?} {screen_write:?} audio {audio_read:?} {audio_write:?}"
        );

        // COMMAND_TIMEOUT: 45 s, so a slow plugin load (Acustica, Kontakt) is not a disconnect.
        let ten = COMMAND_TIMEOUT;
        let audio_limit = Duration::from_millis(500);
        assert_eq!(command_read, Some(ten), "command read");
        assert_eq!(command_write, Some(ten), "command write");
        assert_eq!(inbound_read, Some(ten), "inbound read");
        assert_eq!(inbound_write, Some(ten), "inbound write");
        assert_eq!(screen_read, Some(ten), "screen read");
        assert_eq!(screen_write, Some(ten), "screen write");
        assert_eq!(audio_read, Some(audio_limit), "audio read");
        assert_eq!(audio_write, Some(audio_limit), "audio write");

        session.add_builtin_gain(6.0).unwrap();
        let audio = session.exchange(&[vec![1.0; 32], vec![1.0; 32]]).unwrap();
        let expected = 10.0f32.powf(6.0 / 20.0);
        eprintln!("hosted {}", audio[0][0]);
        assert!((audio[0][0] - expected).abs() < 1.0e-4, "gain {}", audio[0][0]);
        assert!((audio[1][0] - expected).abs() < 1.0e-4, "right {}", audio[1][0]);

        drop(session);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn insert_midi_rides_the_audio_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        session.add_builtin_gain(6.0).unwrap();

        let mut plugin = crate::AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.prepare(48_000.0, 256, 2);
        for n in 0..257 {
            let status = if n == 256 { 0x80 } else { 0x90 };
            plugin.midi(n, [status, (n & 0x7f) as u8, 100]);
        }
        let mut io = vec![vec![1.0f32; 32], vec![1.0; 32]];
        plugin.process(&mut io, 32);
        let events = plugin.block_midi();
        let (audio, sent) = session.exchange_midi(&[vec![1.0; 32], vec![1.0; 32]], events).unwrap();
        eprintln!("midi wire events {sent} queued {} sample {}", events.len(), audio[0][0]);
        assert_eq!(sent, 256, "audio request midi count {sent}");
        assert_eq!(events.len(), 256);
        assert_eq!(events[0].data, [0x90, 0, 100]);
        let expected = 10.0f32.powf(6.0 / 20.0);
        assert!((audio[0][0] - expected).abs() < 1.0e-4, "gain {}", audio[0][0]);
        assert!((audio[1][0] - expected).abs() < 1.0e-4, "right {}", audio[1][0]);
        plugin.process(&mut io, 32);
        assert_eq!(plugin.block_midi().len(), 0);

        drop(session);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn assigned_automation_slot_sets_the_hosted_gain() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        session.add_builtin_gain(0.0).unwrap();

        let mut plugin = crate::AudioGridder::new();
        for slot in 0..crate::AUTOMATION_SLOTS {
            assert!(plugin.assign_slot(slot, 0, 0, 0));
            assert!(plugin.set_param(&format!("slot_{slot}"), 0.625));
        }
        let sent = session.apply_slot_writes(&mut plugin).unwrap();
        session.cpu_load().unwrap(); // command reply: the writes are applied before the block
        let audio = session.exchange(&[vec![1.0; 32], vec![1.0; 32]]).unwrap();
        let expected = 10.0f32.powf(6.0 / 20.0);
        eprintln!("slot writes {sent} sample {}", audio[0][0]);
        assert_eq!(sent, crate::AUTOMATION_SLOTS);
        assert!((audio[0][0] - expected).abs() < 1.0e-4, "hosted gain {}", audio[0][0]);

        drop(session);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn selection_loads_catalog_plugin_n_minus_one() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        let catalog = session.catalog_ids().to_vec();
        eprintln!("catalog {}", catalog.len());
        assert_eq!(catalog.first().map(String::as_str), Some(BUILTIN_GAIN_ID));

        let mut plugin = crate::AudioGridder::new();
        plugin.set_param("selection", 0.0);
        assert!(session.apply_selection(&plugin).unwrap().is_none());
        assert!(session.loaded().is_empty());

        plugin.set_param("selection", 1.0);
        let loaded = session.apply_selection(&plugin).unwrap();
        eprintln!("selection loaded {loaded:?} slots {}", session.loaded().len());
        assert_eq!(loaded.as_deref(), Some(catalog[0].as_str()));
        assert_eq!(session.loaded().len(), 1);
        assert_eq!(session.loaded()[0].id, BUILTIN_GAIN_ID);

        assert!(session.apply_selection(&plugin).unwrap().is_none());
        assert_eq!(session.loaded().len(), 1, "the same selection loaded another copy");

        session.set_parameter(0, 0, 0.625).unwrap();
        session.cpu_load().unwrap(); // command reply: the value is applied before the block
        let audio = session.exchange(&[vec![1.0; 32], vec![1.0; 32]]).unwrap();
        let expected = 10.0f32.powf(6.0 / 20.0);
        eprintln!("selection gain {}", audio[0][0]);
        assert!((audio[0][0] - expected).abs() < 1.0e-4, "loaded gain {}", audio[0][0]);

        plugin.set_param("selection", 0.0);
        assert!(session.apply_selection(&plugin).unwrap().is_none());
        assert_eq!(session.loaded().len(), 1, "selection 0 unloaded the chain");

        drop(session);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn add_keeps_factory_preset_names_and_can_select_one() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        let id = "au:aufx:0yow:SfTb";
        assert!(session.catalog_ids().iter().any(|item| item == id), "catalog missing {id}");
        session.add_plugin(id, "").unwrap();
        let names = session.slot_presets(0).to_vec();
        eprintln!("{id} presets {}", names.len());
        assert!(names.len() >= 2, "preset names were discarded");
        assert!(names.iter().all(|name| !name.is_empty()));
        let count = session.slot_parameters(0).len().min(32);
        let before: Vec<f32> = (0..count).map(|index| session.parameter_on(0, index as i32, 0).unwrap()).collect();
        session.set_preset(0, 1, 0).unwrap();
        let after: Vec<f32> = (0..count).map(|index| session.parameter_on(0, index as i32, 0).unwrap()).collect();
        let distance = before.iter().zip(&after).map(|(left, right)| (left - right).abs()).fold(0.0f32, f32::max);
        eprintln!("preset 1 of {} distance {distance}", names.len());
        assert!(distance > 1.0e-3, "preset 1 did not change a parameter");

        drop(session);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn add_keeps_the_parameter_list_from_the_worker() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        session.add_builtin_gain(-6.0).unwrap();
        let params = session.slot_parameters(0);
        eprintln!("params {}", params.len());
        assert_eq!(params.len(), 1, "parameter body was discarded");
        assert_eq!(params[0].idx, 0);
        assert_eq!(params[0].name, "Gain");
        assert_eq!(params[0].label, "dB");
        let expected_current = (-6.0 + 24.0) / 48.0;
        eprintln!("param {} current {} default {}", params[0].name, params[0].current, params[0].default_value);
        assert!((params[0].current - expected_current).abs() < 1.0e-5, "current {}", params[0].current);
        assert!((params[0].default_value - 0.5).abs() < 1.0e-5, "default {}", params[0].default_value);

        session.set_parameter(0, 0, 0.625).unwrap();
        session.cpu_load().unwrap(); // command reply: the value is applied before the block
        let updated = session.slot_parameters(0)[0].current;
        let audio = session.exchange(&[vec![1.0; 32], vec![1.0; 32]]).unwrap();
        let expected = 10.0f32.powf(6.0 / 20.0);
        eprintln!("after set stored {updated} sample {}", audio[0][0]);
        assert!((updated - 0.625).abs() < 1.0e-5, "stored {}", updated);
        assert!((audio[0][0] - expected).abs() < 1.0e-4, "hosted {}", audio[0][0]);
        assert!((audio[1][0] - expected).abs() < 1.0e-4, "right {}", audio[1][0]);

        drop(session);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn insert_plays_the_hosted_gain() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));

        let request = HandshakeRequest {
            samples_per_block: 32,
            ..HandshakeRequest::default()
        };
        let mut session = RemoteSession::connect(port, request).unwrap();
        session.add_builtin_gain(-6.0).unwrap();
        let remote = session.exchange(&[vec![1.0; 32], vec![1.0; 32]]).unwrap();
        let expected = 10.0f32.powf(-6.0 / 20.0);
        assert!((remote[0][0] - expected).abs() < 1.0e-5, "server {}", remote[0][0]);
        assert!((remote[1][0] - expected).abs() < 1.0e-5);

        let mut plugin = crate::AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.prepare(48_000.0, 256, 1);
        let mut io = vec![vec![1.0f32; 200]];
        let played = vec![vec![remote[0][0]; 200]];
        plugin.process_with_remote(&mut io, 200, Some(&played));
        assert!((io[0][150] - expected).abs() < 1.0e-4, "insert {}", io[0][150]);

        drop(session);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn insert_automation_changes_the_next_block() {
        let mut plugin = crate::AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.set_param("input_gain", -12.0);
        plugin.prepare(48_000.0, 256, 1);
        let mut io = vec![vec![1.0f32; 200]];
        plugin.process(&mut io, 200);
        let expected = 10.0f32.powf(-12.0 / 20.0);
        assert!((io[0][180] - expected).abs() < 1.0e-4, "automated input gain {}", io[0][180]);
    }

    #[test]
    fn set_server_reconnects_and_remembers_the_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let endpoint = format!("127.0.0.1:{port}");

        let mut plugin = crate::AudioGridder::new();
        assert_eq!(plugin.active_server(), "127.0.0.1:55056");
        assert!(plugin.discovered_servers().is_empty());
        assert!(plugin.set_server(&endpoint), "set_server rejected {endpoint}");
        assert_eq!(plugin.active_server(), endpoint);
        assert_eq!(plugin.discovered_servers(), &[endpoint.clone()]);
        assert!(plugin.set_server(&endpoint));
        assert_eq!(plugin.discovered_servers().len(), 1, "the same server was stored twice");
        assert!(plugin.set_server("example.test"));
        assert_eq!(plugin.active_server(), "example.test:55056");
        assert_eq!(plugin.discovered_servers().len(), 2);
        assert!(plugin.set_server("bad:nope"));
        assert_eq!(plugin.active_server(), "bad:nope:55056");
        assert!(plugin.set_server(&endpoint));

        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = plugin.reconnect(request).unwrap();
        session.add_builtin_gain(6.0).unwrap();
        let remote = session.exchange(&[vec![1.0f32; 32], vec![1.0; 32]]).unwrap();
        let expected = 10.0f32.powf(6.0 / 20.0);
        eprintln!("reconnect {}", remote[0][0]);
        assert!((remote[0][0] - expected).abs() < 1.0e-4, "reconnect {}", remote[0][0]);

        drop(session);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn offline_render_waits_for_the_server_instead_of_dry() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 256, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        session.add_builtin_gain(6.0).unwrap();

        let mut plugin = crate::AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.prepare(48_000.0, 256, 2);
        plugin.note_chain_ready(true);
        plugin.set_offline(true);
        assert_eq!(plugin.latency(), 128, "offline does not change the profile");

        let handle = plugin.wet_handle();
        let publisher = thread::spawn(move || {
            let block = session.exchange(&[vec![1.0f32; 200], vec![1.0; 200]]).unwrap();
            handle.submit(block);
        });
        let mut io = vec![vec![1.0f32; 200], vec![1.0; 200]];
        plugin.process(&mut io, 200);
        publisher.join().unwrap();
        let expected = 10.0f32.powf(6.0 / 20.0);
        eprintln!("offline {}", io[0][128]);
        assert_eq!(io[0][0], 0.0, "the profile delay still leads");
        assert!((io[0][128] - expected).abs() < 1.0e-4, "offline {}", io[0][128]);

        server.join().unwrap().unwrap();
    }

    #[test]
    fn audio_callback_does_not_wait_on_a_socket() {
        let mut plugin = crate::AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.prepare(48_000.0, 256, 2);
        let mut io = vec![vec![0.25f32; 200], vec![0.25; 200]];
        let started = std::time::Instant::now();
        plugin.process(&mut io, 200);
        assert!(started.elapsed() < Duration::from_millis(50), "process blocked for {:?}", started.elapsed());
        assert!((io[0][180] - 0.25).abs() < 1.0e-4, "callback {}", io[0][180]);
    }

    #[test]
    fn insert_latency_compensates_the_chain() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 128, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        session.add_builtin_gain(0.0).unwrap();
        let mut impulse = vec![0.0f32; 128];
        impulse[0] = 1.0;
        let (dry, latency_1x) = session.exchange_measured(&[impulse.clone(), impulse.clone()]).unwrap();
        session.set_oversampling(0, 2).unwrap();
        session.cpu_load().unwrap(); // command reply: oversampling is on before the block
        let (wet, latency_2x) = session.exchange_measured(&[impulse.clone(), impulse]).unwrap();
        let diff = dry[0].iter().zip(&wet[0]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(diff > 0.05, "2x oversampling left the impulse unchanged (max diff {diff})");
        assert!(latency_2x > latency_1x, "chain latency did not rise ({latency_1x} -> {latency_2x})");
        let mut plugin = crate::AudioGridder::new();
        plugin.set_param("latency_profile", 1.0);
        plugin.note_remote_latency(latency_2x as usize);
        assert_eq!(plugin.latency(), crate::profile_samples(1) + latency_2x as usize);
        drop(session);
        server.join().unwrap().unwrap();
    }

    /// Command-in carries status and parameter pushes. The screen socket carries the editor.
    /// Mouse and key go out on the command socket and come back as those pushes.
    #[test]
    fn insert_reads_command_in_and_the_screen_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 64, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        let mut loaded = None;
        for id in ["au:aufx:lpas:appl", "au:aufx:dely:appl", "au:aufx:lmtr:appl"] {
            if session.add_plugin(id, "").is_ok() {
                loaded = Some(id);
                break;
            }
        }
        let id = loaded.unwrap_or_else(|| panic!("no Apple effect accepted an add"));
        let (status_type, status_body) = session.poll_inbound().unwrap().unwrap_or_else(|| panic!("command-in was idle after add"));
        assert_eq!(status_type, PLUGIN_STATUS, "command-in message {status_type}");
        let status = command::read_json_payload(&status_body).unwrap_or_else(|| panic!("status {status_body:?}"));
        eprintln!("plugin status {status} for {id}");
        assert_eq!(status["ok"], true);
        assert_eq!(status["idx"], 0);

        let block = vec![vec![0.2f32; 64], vec![0.2; 64]];
        let (_audio, latency_1x) = session.exchange_measured(&block).unwrap();
        assert_eq!(session.latency_samples(), latency_1x, "session dropped the audio-response latency");
        session.set_oversampling(0, 2).unwrap();
        session.cpu_load().unwrap(); // command reply: oversampling is on before the block
        let (_audio, latency_2x) = session.exchange_measured(&block).unwrap();
        eprintln!("latency {latency_1x} -> {latency_2x}");
        assert_eq!(session.latency_samples(), latency_2x);
        assert!(latency_2x > latency_1x, "oversampling left latency at {latency_1x}");
        let mut plugin = crate::AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.note_remote_latency(session.latency_samples() as usize);
        assert_eq!(plugin.latency(), crate::profile_samples(0) + latency_2x as usize);

        session.set_parameter(0, 0, 0.0).unwrap();
        session.edit_plugin(0).unwrap();
        let frame = session.poll_screen().unwrap().unwrap_or_else(|| panic!("screen socket was idle"));
        let nonzero = frame.pixels.iter().filter(|byte| **byte != 0).count();
        eprintln!("screen {}x{} bytes {} nonzero {nonzero}", frame.width, frame.height, frame.pixels.len());
        assert!(frame.width > 0 && frame.height > 0, "frame {}x{}", frame.width, frame.height);
        assert!(!frame.pixels.is_empty(), "frame has no pixels");
        assert!(nonzero > 0, "frame pixels are empty");

        session.send_mouse(1, 10.0, 10.0).unwrap();
        let mouse_pushes = collect_until(&mut session, |pushes| pushes.iter().any(|(ty, _)| *ty == PARAMETER_VALUE));
        let value = mouse_pushes
            .iter()
            .find_map(|(ty, body)| (*ty == PARAMETER_VALUE).then(|| command::read_parameter_value(body)).flatten())
            .unwrap_or_else(|| panic!("mouse produced no parameter push: {mouse_pushes:?}"));
        eprintln!("mouse parameter push {value:?}");
        assert_eq!(value.0, 0);
        assert_eq!(value.1, 0);
        assert!((value.2 - 1.0).abs() < 1.0e-3, "left-down did not toggle parameter 0 to 1, got {}", value.2);
        assert_eq!(session.parameter_push(0, 0), Some(value.2));

        session.send_key(&[0x7D]).unwrap();
        let key_pushes = collect_until(&mut session, |pushes| pushes.iter().any(|(ty, _)| *ty == KEY));
        let codes = key_pushes.iter().find_map(|(ty, body)| (*ty == KEY).then(|| key_codes(body))).unwrap_or_default();
        eprintln!("key push {codes:?}");
        assert_eq!(codes, vec![0x7D], "command-in did not carry the down arrow");
        let stepped = key_pushes
            .iter()
            .find_map(|(ty, body)| (*ty == PARAMETER_VALUE).then(|| command::read_parameter_value(body)).flatten());
        let stepped = match stepped {
            Some(found) => found,
            None => {
                let more = collect_until(&mut session, |pushes| pushes.iter().any(|(ty, _)| *ty == PARAMETER_VALUE));
                more.iter()
                    .find_map(|(ty, body)| (*ty == PARAMETER_VALUE).then(|| command::read_parameter_value(body)).flatten())
                    .unwrap_or_else(|| panic!("down arrow did not push a parameter: {key_pushes:?} {more:?}"))
            }
        };
        eprintln!("key parameter push {stepped:?}");
        assert!((stepped.2 - 0.75).abs() < 1.0e-3, "down arrow left parameter 0 at {}", stepped.2);
        assert_eq!(session.parameter_push(0, 0), Some(stepped.2));

        drop(session);
        server.join().unwrap().unwrap();
    }

    /// AddPlugin carries layout and monoChannels. Mono mask 1 leaves the other
    /// channel dry. Multi-Mono keeps one gain instance per channel.
    #[test]
    fn insert_add_sends_layout_and_the_channel_mask() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        session.add_plugin_with(BUILTIN_GAIN_ID, "-6", "Mono", 1).unwrap();
        let result = session.add_result();
        eprintln!(
            "mono add layout {} mask {} instances {}",
            result["layout"], result["monoChannels"], result["channelInstances"]
        );
        assert_eq!(result["layout"], "Mono");
        assert_eq!(result["monoChannels"], 1);
        assert_eq!(result["channelInstances"], 1);
        let audio = session.exchange(&[vec![1.0; 32], vec![1.0; 32]]).unwrap();
        let quieter = 10.0f32.powf(-6.0 / 20.0);
        eprintln!("mono left {} right {}", audio[0][0], audio[1][0]);
        assert!((audio[0][0] - quieter).abs() < 1.0e-4, "left {}", audio[0][0]);
        assert!((audio[1][0] - 1.0).abs() < 1.0e-5, "unselected channel {}", audio[1][0]);
        drop(session);
        server.join().unwrap().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        session.add_plugin_with(BUILTIN_GAIN_ID, "6|-6", "Multi-Mono", 1).unwrap();
        let result = session.add_result();
        eprintln!(
            "multi-mono add layout {} mask {} instances {}",
            result["layout"], result["monoChannels"], result["channelInstances"]
        );
        assert_eq!(result["layout"], "Multi-Mono");
        assert_eq!(result["monoChannels"], 1);
        assert_eq!(result["channelInstances"], 2);
        let left_param = session.parameter_on(0, 0, 0).unwrap();
        let right_param = session.parameter_on(0, 0, 1).unwrap();
        eprintln!("multi-mono params channel0 {left_param} channel1 {right_param}");
        assert!((left_param - 0.625).abs() < 1.0e-4, "channel 0 gain {left_param}");
        assert!((right_param - 0.375).abs() < 1.0e-4, "channel 1 gain {right_param}");
        let audio = session.exchange(&[vec![1.0; 32], vec![1.0; 32]]).unwrap();
        let louder = 10.0f32.powf(6.0 / 20.0);
        eprintln!("multi-mono mask 1 left {} right {}", audio[0][0], audio[1][0]);
        assert!((audio[0][0] - louder).abs() < 1.0e-4, "left {}", audio[0][0]);
        assert!((audio[1][0] - 1.0).abs() < 1.0e-5, "masked channel {}", audio[1][0]);
        session.set_mono_channels(0, 2).unwrap();
        let _ = session.parameter_on(0, 0, 1).unwrap();
        let audio = session.exchange(&[vec![1.0; 32], vec![1.0; 32]]).unwrap();
        eprintln!("multi-mono mask 2 left {} right {}", audio[0][0], audio[1][0]);
        assert!((audio[0][0] - 1.0).abs() < 1.0e-5, "cleared channel {}", audio[0][0]);
        assert!((audio[1][0] - quieter).abs() < 1.0e-4, "second instance {}", audio[1][0]);
        drop(session);
        server.join().unwrap().unwrap();
    }

    /// The insert's audio request carries the host playhead. GetPluginSettings reads it back.
    #[test]
    fn insert_exchange_sends_the_host_playhead() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        session.add_builtin_gain(0.0).unwrap();
        let block = vec![vec![1.0f32; 32], vec![1.0; 32]];
        let first = PositionInfo {
            bpm: 173.0,
            time_in_samples: 999,
            ppq_position: 3.25,
            playing: true,
            ..PositionInfo::default()
        };
        let second = PositionInfo {
            bpm: 90.0,
            time_in_samples: 10,
            ppq_position: 0.5,
            playing: false,
            ..PositionInfo::default()
        };
        let audio = session.exchange_at(&block, first).unwrap();
        assert!((audio[0][0] - 1.0).abs() < 1.0e-5, "unity gain {}", audio[0][0]);
        let settings = session.plugin_settings(0).unwrap();
        let playhead = &settings["playhead"];
        eprintln!("playhead {playhead}");
        assert_eq!(playhead["bpm"], 173.0);
        assert_eq!(playhead["playing"], true);
        assert_eq!(playhead["samples"], 999);
        assert_eq!(playhead["ppq"], 3.25);
        session.exchange_at(&block, second).unwrap();
        let settings = session.plugin_settings(0).unwrap();
        let playhead = &settings["playhead"];
        eprintln!("playhead {playhead}");
        assert_eq!(playhead["bpm"], 90.0);
        assert_eq!(playhead["playing"], false);
        assert_eq!(playhead["samples"], 10);
        assert_eq!(playhead["ppq"], 0.5);
        let gain = settings["params"][0].as_f64().unwrap();
        assert!((gain - 0.5).abs() < 1.0e-4, "playhead changed gain {gain}");
        drop(session);
        server.join().unwrap().unwrap();
    }

    /// Load two gains, bypass one, move it, delete it, then replay the bypass on a new worker.
    #[test]
    fn insert_loads_unloads_bypasses_and_reorders_the_chain() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        session.add_plugin_with(BUILTIN_GAIN_ID, "-6", "Stereo", 0).unwrap();
        session.add_plugin_with(BUILTIN_GAIN_ID, "6", "Stereo", 0).unwrap();
        assert_eq!(session.loaded().len(), 2);
        let block = vec![vec![1.0f32; 32], vec![1.0; 32]];
        let both = session.exchange(&block).unwrap();
        let quieter = 10.0f32.powf(-6.0 / 20.0);
        let louder = 10.0f32.powf(6.0 / 20.0);
        eprintln!("chain both {}", both[0][0]);
        assert!((both[0][0] - 1.0).abs() < 1.0e-4, "series {}", both[0][0]);

        session.bypass_plugin(0).unwrap();
        let _ = session.parameter_on(0, 0, 0).unwrap();
        let bypassed = session.exchange(&block).unwrap();
        eprintln!("bypass slot 0 {}", bypassed[0][0]);
        assert!((bypassed[0][0] - louder).abs() < 1.0e-3, "bypassed series {}", bypassed[0][0]);
        assert!(session.loaded()[0].bypassed);

        session.unbypass_plugin(0).unwrap();
        let _ = session.parameter_on(0, 0, 0).unwrap();
        let restored = session.exchange(&block).unwrap();
        eprintln!("unbypass slot 0 {}", restored[0][0]);
        assert!((restored[0][0] - 1.0).abs() < 1.0e-4, "unbypass {}", restored[0][0]);

        session.move_down(0).unwrap();
        let _ = session.parameter_on(0, 0, 0).unwrap();
        let moved_left = session.parameter_on(0, 0, 0).unwrap();
        let moved_right = session.parameter_on(1, 0, 0).unwrap();
        eprintln!("after move down params {moved_left} {moved_right}");
        assert!((moved_left - 0.625).abs() < 1.0e-4, "slot 0 {moved_left}");
        assert!((moved_right - 0.375).abs() < 1.0e-4, "slot 1 {moved_right}");
        assert_eq!(session.loaded()[0].settings, "6");
        assert_eq!(session.loaded()[1].settings, "-6");

        session.move_up(1).unwrap();
        let _ = session.parameter_on(0, 0, 0).unwrap();
        let up_left = session.parameter_on(0, 0, 0).unwrap();
        let up_right = session.parameter_on(1, 0, 0).unwrap();
        eprintln!("after move up params {up_left} {up_right}");
        assert!((up_left - 0.375).abs() < 1.0e-4, "slot 0 after move up {up_left}");
        assert!((up_right - 0.625).abs() < 1.0e-4, "slot 1 after move up {up_right}");
        assert_eq!(session.loaded()[0].settings, "-6");
        session.move_down(0).unwrap();
        let _ = session.parameter_on(0, 0, 0).unwrap();

        session.bypass_plugin(0).unwrap();
        let _ = session.parameter_on(1, 0, 0).unwrap();
        let moved_audio = session.exchange(&block).unwrap();
        eprintln!("bypass moved slot {}", moved_audio[0][0]);
        assert!((moved_audio[0][0] - quieter).abs() < 1.0e-3, "moved bypass {}", moved_audio[0][0]);

        session.unload_plugin(0).unwrap();
        assert_eq!(session.loaded().len(), 1);
        assert_eq!(session.latency_samples(), 0);
        let remaining = session.parameter_on(0, 0, 0).unwrap();
        let remaining_audio = session.exchange(&block).unwrap();
        eprintln!("after unload param {remaining} audio {}", remaining_audio[0][0]);
        assert!((remaining - 0.375).abs() < 1.0e-4, "remaining param {remaining}");
        assert!((remaining_audio[0][0] - quieter).abs() < 1.0e-3, "remaining audio {}", remaining_audio[0][0]);

        session.bypass_plugin(0).unwrap();
        let _ = session.parameter_on(0, 0, 0).unwrap();
        let dry = session.exchange(&block).unwrap();
        eprintln!("bypassed remainder {}", dry[0][0]);
        assert!((dry[0][0] - 1.0).abs() < 1.0e-4, "dry {}", dry[0][0]);
        let saved = session.loaded().to_vec();
        assert!(saved[0].bypassed);
        drop(session);
        server.join().unwrap().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let mut session = RemoteSession::connect(port, HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() }).unwrap();
        session.replay(&saved).unwrap();
        assert_eq!(session.loaded().len(), 1);
        assert!(session.loaded()[0].bypassed);
        let _ = session.parameter_on(0, 0, 0).unwrap();
        let replayed = session.exchange(&block).unwrap();
        eprintln!("replayed bypass {}", replayed[0][0]);
        assert!((replayed[0][0] - 1.0).abs() < 1.0e-4, "replay {}", replayed[0][0]);
        drop(session);
        server.join().unwrap().unwrap();
    }

    fn collect_until(session: &mut RemoteSession, done: impl Fn(&[(i32, Vec<u8>)]) -> bool) -> Vec<(i32, Vec<u8>)> {
        let mut found = Vec::new();
        for _ in 0..8 {
            if done(&found) {
                break;
            }
            match session.poll_inbound().unwrap() {
                Some(message) => found.push(message),
                None => break,
            }
        }
        found
    }

    fn key_codes(body: &[u8]) -> Vec<u16> {
        let bytes = command::read_length_prefixed(body).unwrap_or(body);
        bytes.chunks_exact(2).map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]])).collect()
    }
}
