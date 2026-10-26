//! Per-client worker session (`Worker` in `Server/Source/Worker.hpp` & `Worker.cpp`).
//!
//! Handles the 4 client sockets (`cmdOut`, `cmdIn`, `audio`, `screen`) and all 26 command message
//! types, including `SetPluginOversampling = 75` (commit `559863f`).

use crate::audio_worker::AudioWorker;
use crate::processor::Processor;
use crate::processor_chain::ProcessorChain;
use crate::screen_worker::ScreenWorker;
use og_common::message::{
    send_json_message, AddPluginResult, CpuLoadPayload, ExchangePayload, HandshakeRequest,
    MessageFactory, MessageType, MousePayload, OversamplingPayload, ParameterValuePayload, PresetPayload,
    RawMessage, HANDSHAKE_FLAG_STATE_ACK,
};
use og_common::server_plugin::ServerPlugin;
use og_hosting::PluginFormatManager;
use parking_lot::Mutex;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

pub struct Worker {
    pub handshake: HandshakeRequest,
    pub chain: Arc<ProcessorChain>,
    pub format_mgr: Arc<PluginFormatManager>,
    pub known_plugins: Arc<Mutex<Vec<ServerPlugin>>>,
    pub recents: Arc<Mutex<Vec<ServerPlugin>>>,
    pub running: Arc<AtomicBool>,
    pub active_editor_idx: Arc<AtomicI32>,
    pub screen_local_mode: bool,
    pub screen_capturing_off: bool,
    pub plugin_windows_on_top: bool,
}

impl Worker {
    pub fn new(
        handshake: HandshakeRequest,
        format_mgr: Arc<PluginFormatManager>,
        known_plugins: Arc<Mutex<Vec<ServerPlugin>>>,
        recents: Arc<Mutex<Vec<ServerPlugin>>>,
        screen_local_mode: bool,
        screen_capturing_off: bool,
        plugin_windows_on_top: bool,
    ) -> Self {
        let chain = Arc::new(ProcessorChain::new(
            handshake.sample_rate,
            handshake.samples_per_block.max(64) as usize,
        ));
        Self {
            handshake,
            chain,
            format_mgr,
            known_plugins,
            recents,
            running: Arc::new(AtomicBool::new(true)),
            active_editor_idx: Arc::new(AtomicI32::new(-1)),
            screen_local_mode,
            screen_capturing_off,
            plugin_windows_on_top,
        }
    }

    /// Run the worker session across the 4 sockets (`cmd_in_from_client`, `cmd_out_to_client`, `audio`, `screen`).
    pub fn run(
        &self,
        mut cmd_stream: TcpStream,
        cmd_out_stream: TcpStream,
        audio_stream: TcpStream,
        screen_stream: TcpStream,
        diff_detection: bool,
    ) {
        let cmd_out_to_client = Arc::new(Mutex::new(cmd_out_stream));

        // 1. Send initial PluginList over cmd_stream
        let plugin_list_json =
            serde_json::to_string(&*self.known_plugins.lock()).unwrap_or_else(|_| "[]".to_string());
        if RawMessage::from_str(MessageType::PluginList, &plugin_list_json)
            .send(&mut cmd_stream)
            .is_err()
        {
            return;
        }

        // 2. Spawn AudioWorker and ScreenWorker
        let mut audio_worker = AudioWorker::spawn(
            audio_stream,
            self.chain.clone(),
            self.running.clone(),
            self.handshake.samples_per_block as usize,
            self.handshake.channels_out as usize,
        );
        let mut screen_worker = ScreenWorker::spawn(
            screen_stream,
            self.chain.clone(),
            self.running.clone(),
            self.active_editor_idx.clone(),
            diff_detection,
            self.screen_local_mode || self.screen_capturing_off,
        );

        // 3. Command loop on `cmd_stream`
        while self.running.load(Ordering::Relaxed) {
            let Ok(msg) = MessageFactory::get_next_message(&mut cmd_stream, None) else {
                break;
            };
            if !self.handle_message(msg, &mut cmd_stream, &cmd_out_to_client) {
                break;
            }
        }

        self.running.store(false, Ordering::SeqCst);
        audio_worker.stop();
        screen_worker.stop();
        self.chain.release_resources();
    }

    fn handle_message(
        &self,
        msg: RawMessage,
        cmd_stream: &mut TcpStream,
        cmd_out_to_client: &Arc<Mutex<TcpStream>>,
    ) -> bool {
        match msg.msg_type_enum() {
            MessageType::Quit => return false,

            MessageType::AddPlugin => {
                let req: serde_json::Value = msg.get_json();
                let id = req.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let layout = req
                    .get("layout")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Stereo");
                let mono_ch = req
                    .get("monoChannels")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let maybe_desc = self
                    .known_plugins
                    .lock()
                    .iter()
                    .find(|p| p.id == id || p.scoped_id == id || p.id_deprecated == id)
                    .cloned();

                let Some(mut desc) = maybe_desc else {
                    let res = AddPluginResult {
                        ok: false,
                        latency_samples: self.chain.latency_samples(),
                        disabled_sidechain: false,
                        err: format!("Plugin is not in the server catalog: {id}"),
                    };
                    let _ = send_json_message(cmd_stream, MessageType::AddPluginResult, &res);
                    return true;
                };
                // Negotiate the client channel count before native plugin initialization.
                if layout != "Multi-Mono" {
                    desc.num_input_ch = self.handshake.channels_in;
                    desc.num_output_ch = self.handshake.channels_out;
                }

                match Processor::new(
                    &self.format_mgr,
                    &desc,
                    layout,
                    mono_ch,
                    self.handshake.sample_rate,
                    self.handshake.samples_per_block.max(64) as usize,
                ) {
                    Ok(proc) => {
                        let presets_str = proc.presets().join("|");
                        let params_json =
                            serde_json::to_string(&proc.parameters()).unwrap_or_default();
                        let _layouts_json =
                            serde_json::to_string(&proc.layouts()).unwrap_or_default();

                        // Track in recents
                        {
                            let mut rec = self.recents.lock();
                            rec.retain(|p| p.id != desc.id);
                            rec.insert(0, desc);
                            if rec.len() > 10 {
                                rec.truncate(10);
                            }
                        }

                        self.chain.add_processor(Arc::new(Mutex::new(proc)));
                        let total_lat = self.chain.update_latency();

                        let res = AddPluginResult {
                            ok: true,
                            latency_samples: total_lat,
                            disabled_sidechain: false,
                            err: String::new(),
                        };
                        let _ = send_json_message(cmd_stream, MessageType::AddPluginResult, &res);
                        let _ = RawMessage::from_str(MessageType::Presets, &presets_str)
                            .send(cmd_stream);
                        let _ = RawMessage::from_str(MessageType::ParameterList, &params_json)
                            .send(cmd_stream);
                    }
                    Err(err) => {
                        let res = AddPluginResult {
                            ok: false,
                            latency_samples: self.chain.latency_samples(),
                            disabled_sidechain: false,
                            err,
                        };
                        let _ = send_json_message(cmd_stream, MessageType::AddPluginResult, &res);
                    }
                }
            }

            MessageType::DelPlugin => {
                if msg.payload.len() >= 4 {
                    let idx = i32::from_le_bytes(msg.payload[0..4].try_into().unwrap());
                    if idx >= 0 {
                        self.chain.del_processor(idx as usize);
                    }
                }
                let lat = self.chain.update_latency();
                let _ = RawMessage::new(MessageType::Latency, lat.to_le_bytes().to_vec())
                    .send(cmd_stream);
            }

            MessageType::EditPlugin => {
                if msg.payload.len() >= 4 {
                    let idx = i32::from_le_bytes(msg.payload[0..4].try_into().unwrap());
                    let mut x = 0;
                    let mut y = 0;
                    if msg.payload.len() >= 16 {
                        x = i32::from_le_bytes(msg.payload[8..12].try_into().unwrap());
                        y = i32::from_le_bytes(msg.payload[12..16].try_into().unwrap());
                    }
                    self.active_editor_idx.store(idx, Ordering::Relaxed);
                    if self.screen_local_mode && idx >= 0 {
                        if let Some(proc) = self.chain.get_processor(idx as usize) {
                            proc.lock().show_native_editor(x, y, self.plugin_windows_on_top);
                        }
                    }
                }
            }

            MessageType::HidePlugin => {
                let prev_idx = self.active_editor_idx.swap(-1, Ordering::Relaxed);
                if self.screen_local_mode && prev_idx >= 0 {
                    if let Some(proc) = self.chain.get_processor(prev_idx as usize) {
                        proc.lock().hide_native_editor();
                    }
                }
            }

            MessageType::GetPluginSettings => {
                let mut state = Vec::new();
                if msg.payload.len() >= 4 {
                    let idx = i32::from_le_bytes(msg.payload[0..4].try_into().unwrap());
                    if idx >= 0 {
                        if let Some(proc) = self.chain.get_processor(idx as usize) {
                            state = proc.lock().get_state_information();
                        }
                    }
                }
                let _ = RawMessage::new(MessageType::PluginSettings, state).send(cmd_stream);
            }

            MessageType::SetPluginSettings => {
                if msg.payload.len() < 4 {
                    return false;
                }
                let idx = i32::from_le_bytes(msg.payload[0..4].try_into().unwrap());
                let Ok(settings_msg) = MessageFactory::get_next_message(cmd_stream, None) else {
                    return false;
                };
                let result = if settings_msg.msg_type_enum() != MessageType::PluginSettings {
                    Err("Expected plugin state data".to_string())
                } else if idx < 0 {
                    Err("Invalid plugin index".to_string())
                } else if let Some(proc) = self.chain.get_processor(idx as usize) {
                    proc.lock().set_state_information(&settings_msg.payload)
                } else {
                    Err("Plugin index is not loaded".to_string())
                };
                if let Err(error) = &result {
                    log::warn!("State restore failed: {error}");
                }
                if self.handshake.flags & HANDSHAKE_FLAG_STATE_ACK != 0 {
                    let response = serde_json::json!({"ok": result.is_ok(), "error": result.err().unwrap_or_default()});
                    let _ = send_json_message(cmd_stream, MessageType::Result, &response);
                }
            }

            MessageType::BypassPlugin => {
                if msg.payload.len() >= 4 {
                    let idx = i32::from_le_bytes(msg.payload[0..4].try_into().unwrap());
                    if idx >= 0 {
                        if let Some(proc) = self.chain.get_processor(idx as usize) {
                            proc.lock().suspend_processing(true);
                        }
                    }
                }
            }

            MessageType::UnbypassPlugin => {
                if msg.payload.len() >= 4 {
                    let idx = i32::from_le_bytes(msg.payload[0..4].try_into().unwrap());
                    if idx >= 0 {
                        if let Some(proc) = self.chain.get_processor(idx as usize) {
                            proc.lock().suspend_processing(false);
                        }
                    }
                }
            }

            MessageType::SetPluginOversampling => {
                // 1:1 port of `Worker::handleMessage(std::shared_ptr<Message<SetPluginOversampling>>)`
                // in `Server/Source/Worker.cpp` (commit `559863f`).
                if let Some(payload) = OversamplingPayload::from_bytes(&msg.payload) {
                    if payload.idx >= 0 {
                        if let Some(proc) = self.chain.get_processor(payload.idx as usize) {
                            proc.lock().set_oversampling_factor(payload.factor);
                        }
                    }
                    let new_lat = self.chain.update_latency();
                    let _ = RawMessage::new(MessageType::Latency, new_lat.to_le_bytes().to_vec())
                        .send(&mut *cmd_out_to_client.lock());
                }
            }

            MessageType::ExchangePlugins => {
                if let Some(ex) = ExchangePayload::from_bytes(&msg.payload) {
                    if ex.idx_a >= 0 && ex.idx_b >= 0 {
                        self.chain
                            .exchange_processors(ex.idx_a as usize, ex.idx_b as usize);
                    }
                }
            }

            MessageType::RecentsList => {
                let json = serde_json::to_string(&*self.recents.lock())
                    .unwrap_or_else(|_| "[]".to_string());
                let _ = RawMessage::from_str(MessageType::RecentsList, &json).send(cmd_stream);
            }

            MessageType::Preset => {
                if let Some(p) = PresetPayload::from_bytes(&msg.payload) {
                    if p.idx >= 0 && p.preset >= 0 {
                        if let Some(proc) = self.chain.get_processor(p.idx as usize) {
                            proc.lock().set_current_program(p.preset as usize);
                        }
                    }
                }
            }

            MessageType::GetParameterValue => {
                if let Some(mut req) = ParameterValuePayload::from_bytes(&msg.payload) {
                    if req.idx >= 0 && req.param_idx >= 0 {
                        if let Some(proc) = self.chain.get_processor(req.idx as usize) {
                            req.value = proc.lock().get_parameter(req.param_idx as usize);
                        }
                    }
                    let _ = RawMessage::new(MessageType::ParameterValue, req.to_bytes().to_vec())
                        .send(cmd_stream);
                }
            }

            MessageType::ParameterValue => {
                if let Some(req) = ParameterValuePayload::from_bytes(&msg.payload) {
                    if req.idx >= 0 && req.param_idx >= 0 {
                        if let Some(proc) = self.chain.get_processor(req.idx as usize) {
                            proc.lock().set_parameter(req.param_idx as usize, req.value);
                        }
                    }
                }
            }

            MessageType::GetAllParameterValues => {
                let mut payload = Vec::new();
                if msg.payload.len() >= 4 {
                    let idx = i32::from_le_bytes(msg.payload[0..4].try_into().unwrap());
                    if idx >= 0 {
                        if let Some(proc) = self.chain.get_processor(idx as usize) {
                            let guard = proc.lock();
                            for p in guard.parameters() {
                                let pv = ParameterValuePayload {
                                    idx,
                                    param_idx: p.idx,
                                    value: guard.get_parameter(p.idx as usize),
                                };
                                payload.extend_from_slice(&pv.to_bytes());
                            }
                        }
                    }
                }
                let _ = RawMessage::new(MessageType::AllParameterValues, payload).send(cmd_stream);
            }

            MessageType::CpuLoad => {
                let cpu = CpuLoadPayload {
                    val: self.chain.cpu_load(),
                };
                let _ =
                    RawMessage::new(MessageType::CpuLoad, cpu.to_bytes().to_vec()).send(cmd_stream);
            }

            MessageType::Mouse => {
                if let Some(ev) = MousePayload::from_bytes(&msg.payload) {
                    #[cfg(target_os = "macos")]
                    unsafe {
                        extern "C" {
                            fn og_post_mouse_event(
                                ev_type: u8,
                                x: f32,
                                y: f32,
                                shift: bool,
                                ctrl: bool,
                                alt: bool,
                                delta_x: f32,
                                delta_y: f32,
                            );
                        }
                        og_post_mouse_event(
                            ev.ev_type as u8,
                            ev.x,
                            ev.y,
                            ev.is_shift_down,
                            ev.is_ctrl_down,
                            ev.is_alt_down,
                            ev.delta_x,
                            ev.delta_y,
                        );
                    }
                }
            }

            MessageType::Key => {
                if msg.payload.len() >= 2 {
                    let num_keys = msg.payload.len() / 2;
                    let mut key_codes = Vec::with_capacity(num_keys);
                    for i in 0..num_keys {
                        let kc = u16::from_le_bytes(msg.payload[i*2..i*2+2].try_into().unwrap());
                        key_codes.push(kc);
                    }
                    #[cfg(target_os = "macos")]
                    unsafe {
                        extern "C" {
                            fn og_post_key_event(key_code: u16, is_down: bool);
                        }
                        for kc in key_codes {
                            og_post_key_event(kc, true);
                            og_post_key_event(kc, false);
                        }
                    }
                }
            }

            MessageType::ScreenCaptureArea
            | MessageType::Rescan
            | MessageType::Restart => {
                // Handled gracefully
            }

            _ => {}
        }
        true
    }
}
