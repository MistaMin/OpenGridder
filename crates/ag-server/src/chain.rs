//! One client's plugin chain.
//!
//! AudioGridder's `Worker` dispatches every command in `Message.hpp`. Builtin
//! gain stays the utility slot. Anything else is `host::instantiate`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ag_protocol::command::{
    self, ADD_PLUGIN, ADD_PLUGIN_RESULT, BYPASS_PLUGIN, CLIPBOARD, CPU_LOAD, DEL_PLUGIN, EDIT_PLUGIN,
    EXCHANGE_PLUGINS, GET_ALL_PARAMETER_VALUES, GET_PARAMETER_VALUE, GET_PLUGIN_SETTINGS, GET_SCREEN_BOUNDS,
    HIDE_PLUGIN, KEY, MOUSE, PARAMETER_GESTURE, PARAMETER_VALUE, PARAMETERS, PLUGIN_LIST, PLUGIN_SETTINGS,
    PLUGIN_STATUS,
    PRESET, PRESETS, RECENTS_LIST, RESULT, SCREEN_BOUNDS, SERVER_ERROR, SET_MONO_CHANNELS,
    SET_PLUGIN_OVERSAMPLING, SET_PLUGIN_SETTINGS, UNBYPASS_PLUGIN, UPDATE_SCREEN_CAPTURE_AREA,
};
use ag_protocol::PositionInfo;
use openplugin::prelude::*;

use crate::host::{self, EditorFrame};
use crate::screen::ScreenDiff;
use crate::HostedGain;

pub const BUILTIN_GAIN_ID: &str = "Builtin-gain";

enum Processor {
    Gain(HostedGain),
    Plugin(Box<dyn host::Hosted>),
}

struct Slot {
    processor: Processor,
    /// Channels 1..N-1 when `layout` is Multi-Mono. Channel 0 is `processor`.
    more: Vec<Processor>,
    bypass: bool,
    oversampling: i32,
    os_latency: i32,
    settings: String,
    /// AddPlugin JSON `layout`. Empty becomes Stereo, which is one instance on every channel.
    layout: String,
    mono: u64,
    program: i32,
    /// Pixels added to the capture rectangle. AudioGridder floors this at 0.
    capture_extra: i32,
    /// `SCAREA_FULLSCREEN` toggles this. The capture rect becomes the main display.
    capture_fullscreen: bool,
    os_a: Vec<Oversampler2x>,
    os_b: Vec<Oversampler2x>,
}

pub struct PluginChain {
    slots: Vec<Slot>,
    /// Newest-first plugin JSON lines, keyed by the client address. Cap 10.
    recents: HashMap<String, Vec<String>>,
    client_host: String,
    channels_out: i32,
    /// Handshake flag 1 keeps every plugin. Clear, an effect session hides instruments.
    list_all: bool,
    channels_in: i32,
    channels_sc: i32,
    /// Handshake `activeChannels`. Input bits are `0..channels_in`. Output bits
    /// start at 32 when the chain has an input.
    active_channels: u64,
    sample_rate: f32,
    max_block: usize,
    channels: usize,
    awaiting_settings: Option<usize>,
    editor: Option<usize>,
    editor_frame: Option<EditorFrame>,
    /// Width and height of the open editor. `take_screen` consumes the
    /// pixels, and this size stays until hide.
    editor_size: Option<(i32, i32)>,
    /// Previous editor image and the capture counter from `ScreenWorker`.
    screen: ScreenDiff,
    /// The open editor's window (`crate::ui`), 0 when none. The session's screen thread
    /// reads the shared copy to capture and stream it without taking the chain lock.
    editor_window: u64,
    live_editor: Arc<AtomicU64>,
    /// Instances created outside the chain lock for the next ADD_PLUGIN (`preload`).
    /// An empty list means creating them failed.
    preloaded: Option<Vec<Box<dyn host::Hosted>>>,
    /// The preloaded instances already had the slot's saved state restored.
    state_restored: bool,
    outbound: Vec<(i32, Vec<u8>)>,
    /// Last playhead the audio thread applied. AudioGridder does not put this
    /// in the plugin settings chunk. GetPluginSettings appends it so a client
    /// can observe the worker, because the audio response has no tempo field.
    last_playhead: PositionInfo,
}

impl PluginChain {
    pub fn new(channels_out: i32) -> Self {
        Self {
            slots: Vec::new(),
            recents: HashMap::new(),
            client_host: String::new(),
            channels_out,
            list_all: false,
            channels_in: channels_out,
            channels_sc: 0,
            active_channels: 0,
            sample_rate: 48_000.0,
            max_block: 512,
            channels: channels_out.max(1) as usize,
            awaiting_settings: None,
            editor: None,
            editor_frame: None,
            editor_size: None,
            screen: ScreenDiff::new(),
            editor_window: 0,
            live_editor: Arc::new(AtomicU64::new(0)),
            preloaded: None,
            state_restored: false,
            outbound: Vec::new(),
            last_playhead: PositionInfo::default(),
        }
    }

    /// AudioGridder keys recents by the command socket's client host.
    pub fn set_client_host(&mut self, host: String) {
        if !host.is_empty() {
            self.client_host = host;
        }
    }

    /// `AudioWorker::init` stores the handshake channel counts and active bits.
    pub fn set_route(&mut self, channels_in: i32, channels_sc: i32, active_channels: u64) {
        self.channels_in = channels_in;
        self.channels_sc = channels_sc;
        self.active_channels = active_channels;
    }

    /// `NO_PLUGINLIST_FILTER` keeps the whole catalog. Otherwise a plugin is listed
    /// when its input count matches, when an effect session sees any inputs, or
    /// when an instrument session sees an instrument.
    pub fn set_plugin_list_filter(&mut self, flags: u8) {
        self.list_all = flags & command::FLAG_NO_PLUGINLIST_FILTER != 0;
    }

    fn lists_plugin(&self, is_instrument: bool) -> bool {
        if self.list_all {
            return true;
        }
        let inputs = if is_instrument { 0 } else { 2 };
        let channels_in = self.channels_in;
        channels_in == inputs || (channels_in > 0 && inputs > 0) || (channels_in == 0 && is_instrument)
    }

    /// `AudioWorker::run` closes the socket when the active input count is
    /// greater than the buffer's channel count.
    pub fn buffer_covers_active_inputs(&self, buffer_channels: usize) -> bool {
        crate::channels::active_input_count(self.active_channels, self.channels_in, self.channels_sc)
            <= buffer_channels as i32
    }

    pub fn prepare(&mut self, sample_rate: f32, max_block: usize, channels: usize) {
        self.sample_rate = sample_rate.max(8_000.0);
        self.max_block = max_block.clamp(1, 8192);
        self.channels = channels.clamp(1, 2);
        for slot in &mut self.slots {
            slot.prepare(self.sample_rate, self.max_block, self.channels);
        }
    }

    pub fn process(&mut self, audio: &mut [Vec<f32>], frames: usize) {
        self.process_midi(audio, &[], frames);
    }

    pub fn process_midi(&mut self, audio: &mut [Vec<f32>], midi: &[[u8; 3]], frames: usize) {
        self.process_at(audio, midi, frames, PositionInfo::default());
    }

    /// `AudioWorker::run` calls `setPlayHead` with the block position before process.
    /// A buffer narrower than the chain is mapped the way `processBlock` maps it.
    pub fn process_at(&mut self, audio: &mut [Vec<f32>], midi: &[[u8; 3]], frames: usize, position: PositionInfo) {
        self.last_playhead = position;
        let width = crate::channels::chain_width(self.channels_in, self.channels_sc, self.channels_out);
        if width <= audio.len() {
            self.run_slots_f32(audio, midi, frames, &position);
            return;
        }
        let samples = audio.first().map(|channel| channel.len()).unwrap_or(frames);
        let mut proc_buffer = vec![vec![0.0f32; samples]; width];
        let map = crate::channels::ChannelMap::server(self.active_channels, self.channels_in, self.channels_sc, self.channels_out);
        if crate::channels::active_input_count(self.active_channels, self.channels_in, self.channels_sc) > 0 {
            crate::channels::apply_map(audio, &mut proc_buffer, map.forward());
        }
        self.run_slots_f32(&mut proc_buffer, midi, samples, &position);
        crate::channels::apply_map(&proc_buffer, audio, map.reverse());
    }

    pub fn process_f64(&mut self, audio: &mut [Vec<f64>], midi: &[[u8; 3]], frames: usize) {
        self.process_f64_at(audio, midi, frames, PositionInfo::default());
    }

    pub fn process_f64_at(&mut self, audio: &mut [Vec<f64>], midi: &[[u8; 3]], frames: usize, position: PositionInfo) {
        self.last_playhead = position;
        let width = crate::channels::chain_width(self.channels_in, self.channels_sc, self.channels_out);
        if width <= audio.len() {
            self.run_slots_f64(audio, midi, frames, &position);
            return;
        }
        let samples = audio.first().map(|channel| channel.len()).unwrap_or(frames);
        let mut proc_buffer = vec![vec![0.0f64; samples]; width];
        let map = crate::channels::ChannelMap::server(self.active_channels, self.channels_in, self.channels_sc, self.channels_out);
        if crate::channels::active_input_count(self.active_channels, self.channels_in, self.channels_sc) > 0 {
            crate::channels::apply_map(audio, &mut proc_buffer, map.forward());
        }
        self.run_slots_f64(&mut proc_buffer, midi, samples, &position);
        crate::channels::apply_map(&proc_buffer, audio, map.reverse());
    }

    fn run_slots_f32(&mut self, audio: &mut [Vec<f32>], midi: &[[u8; 3]], frames: usize, position: &PositionInfo) {
        let rate = self.sample_rate;
        for slot in &mut self.slots {
            if slot.bypass {
                continue;
            }
            slot.apply_transport(position);
            slot.process_f32(audio, midi, frames, rate);
        }
    }

    fn run_slots_f64(&mut self, audio: &mut [Vec<f64>], midi: &[[u8; 3]], frames: usize, position: &PositionInfo) {
        for slot in &mut self.slots {
            if slot.bypass {
                continue;
            }
            slot.apply_transport(position);
            slot.process_f64(audio, midi, frames);
        }
    }

    pub fn latency(&self) -> i32 {
        self.slots.iter().map(|slot| slot.latency()).sum()
    }

    /// `AudioMessage::sendToClient` writes whatever `processBlock` left in the
    /// MIDI buffer. A hosted AudioUnit takes the events with
    /// `MusicDeviceMIDIEvent`, and this process has no MIDI output callback,
    /// so that buffer is empty. Builtin gain does not touch it. A bypassed
    /// slot is skipped, the same way `processBlockBypassed` leaves the events.
    pub fn plugin_replaced_midi(&self) -> bool {
        self.slots.iter().any(|slot| !slot.bypass && slot.hosts_plugin())
    }

    pub fn take_editor_frame(&mut self) -> Option<EditorFrame> {
        self.editor_frame.take()
    }

    /// Encode the pending editor image the way `ScreenWorker::runNative` does
    /// when diff detection is on. `None` means the capture was identical, or
    /// the editor was reset because the image was mostly black or white.
    pub fn take_screen(&mut self) -> Option<(i32, i32, Vec<u8>)> {
        let frame = self.take_editor_frame()?;
        self.screen.capture(frame.width, frame.height, &frame.pixels).map(|png| (frame.width, frame.height, png))
    }

    /// Async pushes AudioGridder writes on the client's command-in socket.
    pub fn take_outbound(&mut self) -> Vec<(i32, Vec<u8>)> {
        std::mem::take(&mut self.outbound)
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn editor_is_open(&self) -> bool {
        self.editor.is_some()
    }

    /// Editor window id shared with the session's screen thread (0 = none).
    pub fn live_editor(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.live_editor)
    }

    /// ADD_PLUGIN for a hosted plugin: what to instantiate. Read under the chain lock; the
    /// caller instantiates without it (`instantiate_for`) and hands the result to `preload`,
    /// so a slow plugin load does not stall audio past the client's timeout.
    pub fn load_request(&self, body: &[u8]) -> Option<LoadRequest> {
        let json = command::read_json_payload(body)?;
        let id = json.get("id").and_then(|v| v.as_str())?.to_string();
        if id.is_empty() || id == BUILTIN_GAIN_ID || id == "gain" {
            return None;
        }
        let layout = layout_name(json.get("layout").and_then(|v| v.as_str()).unwrap_or(""));
        let count = if layout == "Multi-Mono" { self.channels.max(1) } else { 1 };
        let bus_channels = if layout == "Mono" || layout == "Multi-Mono" { 1 } else { self.channels };
        let settings = json.get("settings").and_then(|v| v.as_str()).unwrap_or("").to_string();
        Some(LoadRequest { id, settings, sample_rate: self.sample_rate, max_block: self.max_block, bus_channels, count })
    }

    pub fn instantiate_for(request: &LoadRequest) -> Vec<Box<dyn host::Hosted>> {
        let mut instances = Vec::with_capacity(request.count);
        for _ in 0..request.count {
            match host::instantiate(&request.id, request.sample_rate, request.max_block, request.bus_channels) {
                Some(mut plugin) => {
                    // The plugin's own saved state (AudioGridder's setStateInformation), before
                    // it goes live. Multi-mono instances all get the same state.
                    if let (Some(chunk), Some(target)) = (state_blob(&request.settings), plugin.editor_target()) {
                        if !crate::ui::set_state(target, &chunk) {
                            eprintln!("ag-server: {} refused its saved state", request.id);
                        }
                    }
                    // Then the saved parameter values on top: some plugins (Acustica, Softube
                    // AUs) accept their state blob without the values their controls show.
                    // For plugins whose state already has them, this changes nothing.
                    if let Some(values) = state_values(&request.settings) {
                        for (index, value) in values.iter().enumerate() {
                            plugin.set_param(index as i32, *value);
                        }
                    }
                    // Warm it up off the audio path: plugins (Acustica, iZotope) do lazy setup
                    // and start with silence in their first blocks; that would otherwise stall
                    // the live chain and drop out when the slot is inserted.
                    let mut silence = vec![vec![0.0f32; request.max_block.max(1)]; request.bus_channels.max(1)];
                    for _ in 0..8 {
                        plugin.process_f32(&mut silence, &[], request.max_block.max(1));
                    }
                    instances.push(plugin)
                }
                None => return Vec::new(),
            }
        }
        instances
    }

    pub fn preload(&mut self, instances: Vec<Box<dyn host::Hosted>>, state_restored: bool) {
        self.preloaded = Some(instances);
        self.state_restored = state_restored;
    }

    /// GET_PLUGIN_SETTINGS, first half (under the lock): the plugin whose state to read.
    /// The state itself is read without the lock (`ui::get_state`): it can be large and slow
    /// (Acustica) and the audio thread needs the chain meanwhile.
    pub fn settings_target(&self, body: &[u8]) -> Option<host::EditorTarget> {
        let index = command::read_number(body).and_then(usize_index)?;
        match &self.slots.get(index)?.processor {
            Processor::Plugin(plugin) => plugin.editor_target(),
            Processor::Gain(_) => None,
        }
    }

    /// GET_PLUGIN_SETTINGS, second half: the parameter values plus the plugin's own state.
    pub fn settings_reply(&self, body: &[u8], state: Option<Vec<u8>>) -> Vec<(i32, Vec<u8>)> {
        let mut replies = self.get_settings(body);
        let Some(state) = state else { return replies };
        if let Some((PLUGIN_SETTINGS, reply)) = replies.first_mut().map(|(t, b)| (*t, b)) {
            if let Some(text) = command::read_length_prefixed(reply).and_then(|b| std::str::from_utf8(b).ok()) {
                if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(text) {
                    value["v"] = serde_json::json!(2);
                    value["state"] = serde_json::json!(base64_encode(&state));
                    *reply = command::length_prefixed(value.to_string().as_bytes());
                }
            }
        }
        replies
    }

    /// EDIT_PLUGIN, first half (under the lock): close the open editor, select the slot,
    /// and return what to open. A plugin without its own editor gets the generic one here
    /// and `None` is returned.
    pub fn begin_edit(&mut self, body: &[u8]) -> Option<(usize, host::EditorTarget, String, i32, i32)> {
        self.check_window_closed();
        self.close_window();
        self.editor_frame = None;
        self.editor_size = None;
        self.screen.clear_baseline();
        let (index, _, x, y) = command::read_edit_plugin(body)?;
        self.editor = usize_index(index).filter(|i| *i < self.slots.len());
        let index = self.editor?;
        let Processor::Plugin(plugin) = &mut self.slots[index].processor else { return None };
        if let Some(target) = plugin.editor_target() {
            return Some((index, target, plugin.name(), x, y));
        }
        plugin.show_editor();
        self.editor_frame = plugin.editor_frame();
        self.editor_size = self.editor_frame.as_ref().map(|frame| (frame.width, frame.height));
        None
    }

    /// EDIT_PLUGIN, second half (under the lock): the window `crate::ui::open_target` made,
    /// 0 when it could not; then the generic editor stands in.
    pub fn finish_edit(&mut self, index: usize, window: u64) {
        if self.editor != Some(index) {
            crate::ui::close(window);
            return;
        }
        if window != 0 {
            self.editor_window = window;
            self.live_editor.store(window, Ordering::Release);
            self.editor_size = crate::ui::size(window);
            return;
        }
        if let Some(Processor::Plugin(plugin)) = self.slots.get_mut(index).map(|slot| &mut slot.processor) {
            plugin.show_editor();
            self.editor_frame = plugin.editor_frame();
            self.editor_size = self.editor_frame.as_ref().map(|frame| (frame.width, frame.height));
        }
    }

    /// Close the real editor window, if one is open. Runs before its plugin can be dropped.
    fn close_window(&mut self) {
        if self.editor_window != 0 {
            let id = std::mem::take(&mut self.editor_window);
            self.live_editor.store(0, Ordering::Release);
            crate::ui::close(id);
        }
    }

    /// The window was closed on the server's screen: forget it and tell the client
    /// (`Worker::sendHideEditor`).
    fn check_window_closed(&mut self) {
        if self.editor_window != 0 && !crate::ui::is_open(self.editor_window) {
            self.close_window();
            if let Some(index) = self.editor.take() {
                self.editor_size = None;
                self.push_hide(index);
            }
        }
    }

    /// `Defaults::SCAREA_FULLSCREEN`. Any other nonzero value adds to the
    /// capture margin. Zero leaves the margin alone.
    const CAPTURE_FULLSCREEN: i32 = 0xFFFF;

    /// `Processor::updateScreenCaptureArea`. The open editor's processor owns
    /// the margin. A closed editor ignores the nudge.
    fn nudge_capture(&mut self, value: i32) {
        if value == 0 {
            return;
        }
        let Some(index) = self.editor else { return };
        let Some(slot) = self.slots.get_mut(index) else { return };
        if value == Self::CAPTURE_FULLSCREEN {
            slot.capture_fullscreen = !slot.capture_fullscreen;
            return;
        }
        let next = slot.capture_extra.saturating_add(value);
        slot.capture_extra = if next > 0 { next } else { 0 };
    }

    /// `ProcessorWindow::getScreenCaptureRect`. Fullscreen is the main display.
    /// Otherwise the editor size grows by the capture margin on both axes.
    fn capture_bounds(&self) -> (i32, i32, i32, i32) {
        let Some(index) = self.editor else { return (0, 0, 0, 0) };
        let Some(slot) = self.slots.get(index) else { return (0, 0, 0, 0) };
        if slot.capture_fullscreen {
            let (width, height) = crate::clipboard::main_display();
            return (0, 0, width, height);
        }
        let (width, height) = self.editor_size.unwrap_or((0, 0));
        let extra = slot.capture_extra.max(0);
        (0, 0, width.saturating_add(extra), height.saturating_add(extra))
    }

    /// `Worker::sendKeys` writes the keys the server editor applied. Modifiers
    /// stay local. Up and down are the codes `apply_editor_key` consumes.
    fn forward_applied_keys(&mut self, codes: &[u16]) {
        let applied: Vec<u16> = codes.iter().copied().filter(|code| matches!(code, 0x7D | 0x7E)).collect();
        if applied.is_empty() {
            return;
        }
        let mut bytes = Vec::with_capacity(applied.len() * 2);
        for code in &applied {
            bytes.extend_from_slice(&code.to_le_bytes());
        }
        self.outbound.push((KEY, command::length_prefixed(&bytes)));
    }

    /// `Worker::sendParamValueChange` writes ParameterValue on the client's
    /// command-in socket. Call it after the editor borrow ends.
    fn push_parameter_if_changed(&mut self, index: usize, param: i32, before: f32) {
        let after = self.slots.get(index).map(|slot| slot.param(param)).unwrap_or(before);
        if (after - before).abs() <= 1.0e-5 {
            return;
        }
        self.outbound.push((PARAMETER_VALUE, command::parameter_value(index as i32, param, after, 0).to_vec()));
    }

    /// `Worker::sendParamGestureChange`. Left-down starts, left-up ends.
    fn push_gesture(&mut self, index: usize, param: i32, starting: bool) {
        self.outbound.push((PARAMETER_GESTURE, command::parameter_gesture(index as i32, param, starting, 0).to_vec()));
    }

    /// `Worker::sendStatusChange`. A ready plugin is `ok` with an empty error.
    fn push_status(&mut self, index: usize, ok: bool, err: &str) {
        let body = command::json_payload(&serde_json::json!({"idx": index, "ok": ok, "err": err}));
        self.outbound.push((PLUGIN_STATUS, body));
    }

    /// `Worker::sendHideEditor`. The body is the slot index, not a JSON object.
    fn push_hide(&mut self, index: usize) {
        self.outbound.push((HIDE_PLUGIN, command::number(index as i32).to_vec()));
    }

    /// Replies go back on the command socket the client writes to, matching
    /// `Worker::handleMessage`.
    pub fn handle(&mut self, message_type: i32, body: &[u8]) -> Vec<(i32, Vec<u8>)> {
        self.check_window_closed();
        if let Some(index) = self.awaiting_settings.take() {
            if message_type == PLUGIN_SETTINGS {
                if let Some(text) = command::read_length_prefixed(body).and_then(|b| std::str::from_utf8(b).ok()) {
                    self.apply_state_chunk(index, text);
                }
                return Vec::new();
            }
        }
        match message_type {
            ADD_PLUGIN => self.add_plugin(body),
            DEL_PLUGIN => self.delete_plugin(body),
            BYPASS_PLUGIN => {
                self.set_bypass(body, true);
                Vec::new()
            }
            UNBYPASS_PLUGIN => {
                self.set_bypass(body, false);
                Vec::new()
            }
            EXCHANGE_PLUGINS => {
                if let Some((a, b)) = command::read_pair_i32(body) {
                    if let (Some(a), Some(b)) = (usize_index(a), usize_index(b)) {
                        if a < self.slots.len() && b < self.slots.len() {
                            self.slots.swap(a, b);
                        }
                    }
                }
                Vec::new()
            }
            SET_PLUGIN_OVERSAMPLING => {
                if let Some((idx, factor)) = command::read_pair_i32(body) {
                    let factor = match factor {
                        2 => 2,
                        4 => 4,
                        _ => 1,
                    };
                    let rate = self.sample_rate;
                    if let Some(slot) = usize_index(idx).and_then(|i| self.slots.get_mut(i)) {
                        slot.set_oversampling(factor, rate);
                    }
                }
                Vec::new()
            }
            SET_MONO_CHANNELS => {
                if let Some((idx, channels)) = command::read_mono_channels(body) {
                    if let Some(slot) = usize_index(idx).and_then(|i| self.slots.get_mut(i)) {
                        slot.mono = channels;
                    }
                }
                Vec::new()
            }
            PARAMETER_VALUE => {
                if let Some((idx, param, value, channel)) = command::read_parameter_value(body) {
                    if let Some(slot) = usize_index(idx).and_then(|i| self.slots.get_mut(i)) {
                        slot.set_param_on(channel, param, value);
                    }
                }
                Vec::new()
            }
            GET_PARAMETER_VALUE => self.get_parameter(body),
            GET_ALL_PARAMETER_VALUES => self.all_parameters(body),
            GET_PLUGIN_SETTINGS => self.get_settings(body),
            SET_PLUGIN_SETTINGS => {
                if let Some(index) = command::read_number(body).and_then(usize_index) {
                    if index < self.slots.len() {
                        self.awaiting_settings = Some(index);
                    }
                }
                Vec::new()
            }
            PLUGIN_LIST => vec![(PLUGIN_LIST, command::msgpack_payload(&self.catalog()))],
            RECENTS_LIST => {
                let lines = self.recents.get(&self.client_host).map(|list| list.join("\n")).unwrap_or_default();
                vec![(RECENTS_LIST, command::length_prefixed(lines.as_bytes()))]
            }
            CPU_LOAD => {
                crate::cpu::start();
                vec![(CPU_LOAD, command::float_bits(crate::cpu::usage()).to_vec())]
            }
            GET_SCREEN_BOUNDS => {
                if let Some((w, h)) = crate::ui::size(self.editor_window) {
                    return vec![(SCREEN_BOUNDS, command::screen_bounds(0, 0, w, h).to_vec())];
                }
                let (x, y, w, h) = self.capture_bounds();
                vec![(SCREEN_BOUNDS, command::screen_bounds(x, y, w, h).to_vec())]
            }
            EDIT_PLUGIN => {
                // The session calls begin_edit / open_target / finish_edit itself so the
                // window opens without the chain lock; this inline path serves tests.
                if let Some((index, target, title, x, y)) = self.begin_edit(body) {
                    let window = crate::ui::open_target(target, &title, x, y);
                    self.finish_edit(index, window);
                }
                Vec::new()
            }
            HIDE_PLUGIN => {
                // The client's HidePlugin clears the local editor. AudioGridder
                // does not echo it. The command-in push is only the server close.
                self.close_window();
                if let Some(index) = self.editor {
                    if let Some(slot) = self.slots.get_mut(index) {
                        if let Processor::Plugin(plugin) = &mut slot.processor {
                            plugin.hide_editor();
                        }
                    }
                }
                self.editor = None;
                self.editor_frame = None;
                self.editor_size = None;
                self.screen.clear_baseline();
                Vec::new()
            }
            MOUSE => {
                if self.editor_window != 0 {
                    if let Some((kind, x, y, shift, ctrl, alt, delta_x, delta_y)) = read_mouse(body) {
                        crate::ui::mouse(self.editor_window, kind, x, y, shift, ctrl, alt, delta_x, delta_y);
                    }
                    return Vec::new();
                }
                if let Some(index) = self.editor {
                    if let Some((kind, x, y, shift, ctrl, alt, delta_x, delta_y)) = read_mouse(body) {
                        // Top-left 8x8 is the generic editor's close box, the
                        // stand-in for ProcessorWindow::closeButtonPressed.
                        // (10, 10) stays the parameter click.
                        if kind == 1 && x < 8.0 && y < 8.0 {
                            self.close_editor(index);
                        } else {
                            let before = self.slots.get(index).map(|slot| slot.param(0)).unwrap_or(0.0);
                            let mut touched = false;
                            if let Some(slot) = self.slots.get_mut(index) {
                                if let Processor::Plugin(plugin) = &mut slot.processor {
                                    plugin.mouse(kind, x, y, shift, ctrl, alt, delta_x, delta_y);
                                    self.editor_frame = plugin.editor_frame();
                                    touched = true;
                                }
                            }
                            if touched {
                                if kind == 1 {
                                    self.push_gesture(index, 0, true);
                                }
                                self.push_parameter_if_changed(index, 0, before);
                                if kind == 2 {
                                    self.push_gesture(index, 0, false);
                                }
                            }
                        }
                    }
                }
                Vec::new()
            }
            KEY => {
                if self.editor_window != 0 {
                    crate::ui::keys(self.editor_window, &read_keys(body));
                    return Vec::new();
                }
                if let Some(index) = self.editor {
                    let codes = read_keys(body);
                    let before = self.slots.get(index).map(|slot| slot.param(0)).unwrap_or(0.0);
                    let mut applied = false;
                    if let Some(slot) = self.slots.get_mut(index) {
                        if let Processor::Plugin(plugin) = &mut slot.processor {
                            plugin.key(&codes);
                            self.editor_frame = plugin.editor_frame();
                            applied = true;
                        }
                    }
                    if applied {
                        self.forward_applied_keys(&codes);
                        self.push_parameter_if_changed(index, 0, before);
                    }
                }
                Vec::new()
            }
            PRESET => {
                if let Some((idx, preset, channel)) = command::read_preset(body) {
                    // One instance stands in for channel 0. Any other channel
                    // is a different AudioGridder instance and must not touch it.
                    if channel == 0 {
                        if let Some(slot) = usize_index(idx).and_then(|i| self.slots.get_mut(i)) {
                            slot.set_program(preset);
                        }
                    }
                }
                Vec::new()
            }
            CLIPBOARD => {
                if let Some(bytes) = command::read_length_prefixed(body) {
                    let text = String::from_utf8_lossy(bytes);
                    let _ = crate::clipboard::set_text(&text);
                }
                Vec::new()
            }
            UPDATE_SCREEN_CAPTURE_AREA => {
                if let Some(value) = command::read_number(body) {
                    self.nudge_capture(value);
                }
                Vec::new()
            }
            SERVER_ERROR | PARAMETER_GESTURE | command::QUIT | command::ANY => Vec::new(),
            // AudioGridder logs an unknown type and sends nothing. Type 200 is
            // the async screen-capture error, not this reply.
            _ => Vec::new(),
        }
    }

    fn add_plugin(&mut self, body: &[u8]) -> Vec<(i32, Vec<u8>)> {
        let json = command::read_json_payload(body).unwrap_or(serde_json::json!({}));
        let id = json.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let settings = json.get("settings").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let layout = layout_name(json.get("layout").and_then(|v| v.as_str()).unwrap_or(""));
        let mono = json.get("monoChannels").and_then(|v| v.as_u64()).unwrap_or(0);
        let count = if layout == "Multi-Mono" { self.channels.max(1) } else { 1 };
        if id == BUILTIN_GAIN_ID || id == "gain" {
            let parts: Vec<&str> = if layout == "Multi-Mono" { settings.split('|').collect() } else { vec![settings.as_str()] };
            let mut instances: Vec<Processor> = (0..count)
                .map(|index| {
                    let text = parts.get(index).copied().unwrap_or("");
                    let mut gain = gain_from_settings(text);
                    gain.prepare(self.sample_rate, self.max_block, if layout == "Multi-Mono" { 1 } else { self.channels });
                    Processor::Gain(gain)
                })
                .collect();
            let processor = instances.remove(0);
            self.push_slot(processor, instances, settings.clone(), layout.clone(), mono);
            self.apply_state_chunk(self.slots.len() - 1, &settings);
            self.remember_recent(BUILTIN_GAIN_ID);
            self.push_status(self.slots.len() - 1, true, "");
            let instances = self.slots.last().map(Slot::channel_instances).unwrap_or(1);
            return self.added_reply(
                "AudioGridder Host Gain",
                false,
                true,
                builtin_params(self.slots.last().unwrap()),
                "",
                &layout,
                mono,
                instances,
            );
        }
        let bus_channels = if layout == "Mono" || layout == "Multi-Mono" { 1 } else { self.channels };
        let mut instances = Vec::with_capacity(count);
        if let Some(preloaded) = self.preloaded.take() {
            instances.extend(preloaded.into_iter().map(Processor::Plugin));
        } else {
            for _ in 0..count {
                let Some(plugin) = host::instantiate(id, self.sample_rate, self.max_block, bus_channels) else {
                    let fail = serde_json::json!({"success": false, "err": format!("no plugin named {id}")});
                    return vec![(ADD_PLUGIN_RESULT, command::json_payload(&fail))];
                };
                instances.push(Processor::Plugin(plugin));
            }
        }
        if instances.len() != count {
            let fail = serde_json::json!({"success": false, "err": format!("no plugin named {id}")});
            return vec![(ADD_PLUGIN_RESULT, command::json_payload(&fail))];
        }
        let Processor::Plugin(first) = &instances[0] else {
            let fail = serde_json::json!({"success": false, "err": format!("no plugin named {id}")});
            return vec![(ADD_PLUGIN_RESULT, command::json_payload(&fail))];
        };
        let name = first.name();
        let has_editor = first.has_editor();
        let supports_double = first.supports_double();
        let processor = instances.remove(0);
        self.push_slot(processor, instances, settings.clone(), layout.clone(), mono);
        self.apply_state_chunk(self.slots.len() - 1, &settings);
        // The values after the saved settings were applied (the client keeps these).
        let params = match self.slots.last().map(|slot| &slot.processor) {
            Some(Processor::Plugin(plugin)) => plugin.parameters(),
            _ => serde_json::json!([]),
        };
        self.remember_recent(id);
        self.push_status(self.slots.len() - 1, true, "");
        let presets = self.slots.last().map(Slot::preset_list).unwrap_or_default();
        let instances = self.slots.last().map(Slot::channel_instances).unwrap_or(1);
        self.added_reply(&name, has_editor, supports_double, params, &presets, &layout, mono, instances)
    }

    fn push_slot(&mut self, processor: Processor, more: Vec<Processor>, settings: String, layout: String, mono: u64) {
        let mut slot = Slot {
            processor,
            more,
            bypass: false,
            oversampling: 1,
            os_latency: 0,
            settings,
            layout,
            mono,
            program: 0,
            capture_extra: 0,
            capture_fullscreen: false,
            os_a: Vec::new(),
            os_b: Vec::new(),
        };
        slot.prepare(self.sample_rate, self.max_block, self.channels);
        self.slots.push(slot);
    }

    fn close_editor(&mut self, index: usize) {
        if self.editor == Some(index) {
            self.close_window();
        }
        if let Some(slot) = self.slots.get_mut(index) {
            if let Processor::Plugin(plugin) = &mut slot.processor {
                plugin.hide_editor();
            }
        }
        if self.editor == Some(index) {
            self.editor = None;
            self.editor_frame = None;
            self.editor_size = None;
            self.screen.clear_baseline();
        }
        self.push_hide(index);
    }

    fn added_reply(
        &self,
        name: &str,
        has_editor: bool,
        supports_double: bool,
        params: serde_json::Value,
        presets: &str,
        layout: &str,
        mono: u64,
        instances: i32,
    ) -> Vec<(i32, Vec<u8>)> {
        let ok = serde_json::json!({
            "success": true,
            "err": "",
            "latency": self.latency(),
            "disabledSideChain": false,
            "name": name,
            "hasEditor": has_editor,
            "supportsDoublePrecision": supports_double,
            "channelInstances": instances,
            "tailSeconds": 0.0,
            "numOutputChannels": self.channels_out,
            "layout": layout,
            "monoChannels": mono
        });
        vec![
            (ADD_PLUGIN_RESULT, command::json_payload(&ok)),
            (PRESETS, command::length_prefixed(presets.as_bytes())),
            (PARAMETERS, command::msgpack_payload(&params)),
        ]
    }

    fn delete_plugin(&mut self, body: &[u8]) -> Vec<(i32, Vec<u8>)> {
        if let Some(index) = command::read_number(body).and_then(usize_index) {
            if index < self.slots.len() {
                if self.editor == Some(index) {
                    self.close_window();
                }
                self.slots.remove(index);
                if self.editor == Some(index) {
                    self.editor = None;
                    self.editor_frame = None;
                    self.editor_size = None;
                    self.screen.clear_baseline();
                }
            }
        }
        vec![(RESULT, command::result(self.latency(), ""))]
    }

    fn set_bypass(&mut self, body: &[u8], bypass: bool) {
        if let Some(slot) = command::read_number(body).and_then(usize_index).and_then(|i| self.slots.get_mut(i)) {
            slot.bypass = bypass;
        }
    }

    fn get_parameter(&self, body: &[u8]) -> Vec<(i32, Vec<u8>)> {
        let Some((idx, param, channel)) = read_get_parameter(body) else {
            return Vec::new();
        };
        let value = self.slots.get(idx as usize).map(|slot| slot.param_on(channel, param)).unwrap_or(0.0);
        vec![(PARAMETER_VALUE, command::parameter_value(idx, param, value, channel).to_vec())]
    }

    fn all_parameters(&self, body: &[u8]) -> Vec<(i32, Vec<u8>)> {
        let Some(idx) = command::read_number(body) else {
            return Vec::new();
        };
        let Some(slot) = usize_index(idx).and_then(|i| self.slots.get(i)) else {
            return Vec::new();
        };
        match &slot.processor {
            Processor::Gain(_) => {
                vec![(PARAMETER_VALUE, command::parameter_value(idx, 0, slot.param(0), 0).to_vec())]
            }
            Processor::Plugin(plugin) => plugin
                .parameters()
                .as_array()
                .map(|items| {
                    items.iter()
                        .enumerate()
                        .map(|(param, _)| {
                            (PARAMETER_VALUE, command::parameter_value(idx, param as i32, slot.param(param as i32), 0).to_vec())
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn get_settings(&self, body: &[u8]) -> Vec<(i32, Vec<u8>)> {
        let text = command::read_number(body)
            .and_then(usize_index)
            .and_then(|i| self.slots.get(i))
            .map(Slot::state_chunk)
            .unwrap_or_else(|| serde_json::json!({"v": 1, "params": []}).to_string());
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({"v": 1, "params": []}));
        if let Some(object) = value.as_object_mut() {
            object.insert("playhead".to_string(), playhead_json(&self.last_playhead));
        }
        let text = value.to_string();
        vec![(PLUGIN_SETTINGS, command::length_prefixed(text.as_bytes()))]
    }

    /// `{"v":1,"params":[...]}` applies normalized values. A bare number does not.
    fn apply_state_chunk(&mut self, index: usize, text: &str) {
        let Some(slot) = self.slots.get_mut(index) else {
            return;
        };
        slot.settings = text.to_string();
        let restored = std::mem::take(&mut self.state_restored);
        // Settings with the plugin's own state: that state is authoritative. It was restored
        // when the instance was created (`instantiate_for`); otherwise restore it here.
        if restored {
            return; // state and values were restored when the instance was created
        }
        if let Some(chunk) = state_blob(text) {
            if let Processor::Plugin(plugin) = &slot.processor {
                if let Some(target) = plugin.editor_target() {
                    crate::ui::set_state(target, &chunk);
                }
            }
        }
        let Some(values) = state_values(text) else {
            return;
        };
        for (i, value) in values.iter().enumerate() {
            slot.set_param(i as i32, *value);
        }
    }

    fn remember_recent(&mut self, id: &str) {
        let line = self.recent_line(id);
        let list = self.recents.entry(self.client_host.clone()).or_default();
        list.retain(|existing| recent_id(existing).as_deref() != Some(id));
        list.insert(0, line);
        if list.len() > 10 {
            list.truncate(10);
        }
    }

    fn recent_line(&self, id: &str) -> String {
        self.catalog()
            .get("plugins")
            .and_then(|value| value.as_array())
            .and_then(|plugins| plugins.iter().find(|plugin| plugin.get("id").and_then(|value| value.as_str()) == Some(id)))
            .map(|plugin| plugin.to_string())
            .unwrap_or_else(|| serde_json::json!({"id": id, "id2": id, "name": id}).to_string())
    }

    /// `Worker::handleMessage(PluginList)` keeps a layout whose output count
    /// equals `channelsOut`. Multi-Mono is added when a mono layout exists and
    /// the session has more than one output. Names are ordered by that count.
    /// Scanned plugins are treated as stereo: this host does not query buses.
    fn layouts_for(&self, is_instrument: bool, supports_mono: bool, supports_stereo: bool) -> Vec<&'static str> {
        let outs = self.channels_out;
        let mut names = Vec::new();
        if self.channels_in > 0 {
            if supports_mono && outs == 1 {
                names.push("Mono");
            }
            if supports_stereo && outs == 2 {
                names.push("Stereo");
            }
            if supports_mono && outs > 1 {
                names.push("Multi-Mono");
            }
        } else if is_instrument {
            if supports_mono && outs >= 1 {
                names.push("Mono");
            }
            if supports_stereo && outs >= 2 {
                names.push("Stereo");
            }
        }
        if names.is_empty() {
            names.push("Default");
        }
        names.sort_by_key(|name| match *name {
            "Mono" | "Multi-Mono" => 1,
            "Stereo" => 2,
            _ => 9,
        });
        names
    }

    fn catalog(&self) -> serde_json::Value {
        let mut plugins = vec![serde_json::json!({
            "name": "AudioGridder Host Gain",
            "company": "OpenGrid",
            "id": BUILTIN_GAIN_ID,
            "id2": BUILTIN_GAIN_ID,
            "type": "Builtin",
            "category": "Utility",
            "isInstrument": false,
            "layouts": self.layouts_for(false, true, true)
        })];
        for plugin in host::scan() {
            if self.lists_plugin(plugin.is_instrument) {
                let mut entry = host::catalog_entry(&plugin);
                entry["layouts"] = serde_json::json!(self.layouts_for(plugin.is_instrument, false, true));
                plugins.push(entry);
            }
        }
        serde_json::json!({ "plugins": plugins })
    }
}

impl Slot {
    fn channel_instances(&self) -> i32 {
        if self.layout == "Multi-Mono" {
            (1 + self.more.len()) as i32
        } else {
            1
        }
    }

    fn hosts_plugin(&self) -> bool {
        matches!(self.processor, Processor::Plugin(_))
            || self.more.iter().any(|extra| matches!(extra, Processor::Plugin(_)))
    }

    fn prepare(&mut self, sample_rate: f32, max_block: usize, channels: usize) {
        let per_channel = if self.layout == "Multi-Mono" { 1 } else { channels };
        if let Processor::Gain(gain) = &mut self.processor {
            gain.prepare(sample_rate, max_block, per_channel);
        }
        for extra in &mut self.more {
            if let Processor::Gain(gain) = extra {
                gain.prepare(sample_rate, max_block, 1);
            }
        }
        self.ensure_oversamplers(sample_rate, channels);
        if self.oversampling > 1 {
            self.os_latency = measure_oversampling_latency(sample_rate, self.oversampling);
        }
    }

    fn ensure_oversamplers(&mut self, sample_rate: f32, channels: usize) {
        if self.os_a.len() == channels {
            return;
        }
        self.os_a = (0..channels).map(|_| Oversampler2x::new(sample_rate)).collect();
        self.os_b = (0..channels).map(|_| Oversampler2x::new(sample_rate * 2.0)).collect();
    }

    fn set_oversampling(&mut self, factor: i32, sample_rate: f32) {
        self.oversampling = factor;
        let channels = self.os_a.len().max(1);
        self.os_a = (0..channels).map(|_| Oversampler2x::new(sample_rate)).collect();
        self.os_b = (0..channels).map(|_| Oversampler2x::new(sample_rate * 2.0)).collect();
        self.os_latency = measure_oversampling_latency(sample_rate, factor);
        if let Processor::Plugin(plugin) = &mut self.processor {
            plugin.set_oversampling(factor);
        }
        for extra in &mut self.more {
            if let Processor::Plugin(plugin) = extra {
                plugin.set_oversampling(factor);
            }
        }
    }

    fn latency(&self) -> i32 {
        let inner = match &self.processor {
            Processor::Gain(gain) => gain.latency() as i32,
            Processor::Plugin(plugin) => plugin.latency().saturating_add(plugin.oversampling_latency()),
        };
        inner.saturating_add(self.os_latency)
    }

    fn set_param(&mut self, index: i32, value: f32) {
        self.set_param_on(0, index, value);
    }

    fn set_param_on(&mut self, channel: i32, index: i32, value: f32) {
        let Some(processor) = self.processor_mut(channel) else {
            return;
        };
        match processor {
            Processor::Gain(gain) if index == 0 => gain.set_gain_db(norm_to_db(value)),
            Processor::Plugin(plugin) => {
                plugin.set_param(index, value);
            }
            Processor::Gain(_) => {}
        }
    }

    fn param(&self, index: i32) -> f32 {
        self.param_on(0, index)
    }

    fn param_on(&self, channel: i32, index: i32) -> f32 {
        match self.processor_ref(channel) {
            Some(Processor::Gain(gain)) if index == 0 => db_to_norm(gain.gain_db()),
            Some(Processor::Plugin(plugin)) => plugin.param(index),
            _ => 0.0,
        }
    }

    fn processor_mut(&mut self, channel: i32) -> Option<&mut Processor> {
        if channel <= 0 {
            return Some(&mut self.processor);
        }
        self.more.get_mut(channel as usize - 1)
    }

    fn processor_ref(&self, channel: i32) -> Option<&Processor> {
        if channel <= 0 {
            return Some(&self.processor);
        }
        self.more.get(channel as usize - 1)
    }

    fn param_count(&self) -> usize {
        match &self.processor {
            Processor::Gain(_) => 1,
            Processor::Plugin(plugin) => plugin.parameters().as_array().map(|items| items.len()).unwrap_or(0),
        }
    }

    fn state_chunk(&self) -> String {
        let params: Vec<f32> = (0..self.param_count()).map(|index| self.param(index as i32)).collect();
        serde_json::json!({"v": 1, "params": params}).to_string()
    }

    fn preset_list(&self) -> String {
        match &self.processor {
            Processor::Plugin(plugin) => plugin.preset_names().join("|"),
            Processor::Gain(_) => String::new(),
        }
    }

    fn set_program(&mut self, index: i32) -> bool {
        let applied = match &mut self.processor {
            Processor::Plugin(plugin) => plugin.set_program(index),
            Processor::Gain(_) => false,
        };
        if applied {
            self.program = index;
        }
        applied
    }

    fn apply_transport(&mut self, position: &PositionInfo) {
        apply_transport_one(&mut self.processor, position);
        for extra in &mut self.more {
            apply_transport_one(extra, position);
        }
    }

    fn process_f32(&mut self, audio: &mut [Vec<f32>], midi: &[[u8; 3]], frames: usize, sample_rate: f32) {
        if frames == 0 || audio.is_empty() {
            return;
        }
        let dry = inactive_snapshot(&self.layout, self.mono, audio, frames);
        let channels = audio.len();
        self.ensure_oversamplers(sample_rate, channels);
        if self.oversampling < 2 {
            self.run_f32(audio, midi, frames);
            restore_inactive(audio, &dry);
            return;
        }
        let up_frames = frames * 2;
        let mut up2: Vec<Vec<f32>> = (0..channels).map(|_| vec![0.0; up_frames]).collect();
        for ch in 0..channels {
            let count = frames.min(audio[ch].len());
            self.os_a[ch].upsample_2x(&audio[ch][..count], &mut up2[ch][..count * 2]);
        }
        if self.oversampling >= 4 {
            let up4_frames = frames * 4;
            let mut up4: Vec<Vec<f32>> = (0..channels).map(|_| vec![0.0; up4_frames]).collect();
            for ch in 0..channels {
                self.os_b[ch].upsample_2x(&up2[ch], &mut up4[ch]);
            }
            self.run_f32(&mut up4, midi, up4_frames);
            for ch in 0..channels {
                self.os_b[ch].downsample_2x(&up4[ch], &mut up2[ch]);
            }
        } else {
            self.run_f32(&mut up2, midi, up_frames);
        }
        for ch in 0..channels {
            let count = frames.min(audio[ch].len());
            self.os_a[ch].downsample_2x(&up2[ch][..count * 2], &mut audio[ch][..count]);
        }
        restore_inactive(audio, &dry);
    }

    fn process_f64(&mut self, audio: &mut [Vec<f64>], midi: &[[u8; 3]], frames: usize) {
        if audio.is_empty() || frames == 0 {
            return;
        }
        if self.layout == "Multi-Mono" {
            let active: Vec<bool> = (0..audio.len()).map(|ch| channel_is_active(&self.layout, self.mono, ch)).collect();
            for ch in 0..audio.len() {
                if !active[ch] {
                    continue;
                }
                let mut one = vec![audio[ch].clone()];
                if ch == 0 {
                    apply_f64(&mut self.processor, &mut one, midi, frames);
                } else if let Some(extra) = self.more.get_mut(ch - 1) {
                    apply_f64(extra, &mut one, midi, frames);
                } else {
                    continue;
                }
                let count = frames.min(audio[ch].len()).min(one[0].len());
                audio[ch][..count].copy_from_slice(&one[0][..count]);
            }
            return;
        }
        if self.layout == "Mono" {
            let active: Vec<usize> = (0..audio.len()).filter(|ch| channel_is_active(&self.layout, self.mono, *ch)).collect();
            if active.is_empty() {
                return;
            }
            let mut subset: Vec<Vec<f64>> = active.iter().map(|ch| audio[*ch].clone()).collect();
            apply_f64(&mut self.processor, &mut subset, midi, frames);
            for (ch, buf) in active.into_iter().zip(subset) {
                let count = frames.min(audio[ch].len()).min(buf.len());
                audio[ch][..count].copy_from_slice(&buf[..count]);
            }
            return;
        }
        apply_f64(&mut self.processor, audio, midi, frames);
    }

    fn run_f32(&mut self, audio: &mut [Vec<f32>], midi: &[[u8; 3]], frames: usize) {
        if self.layout == "Multi-Mono" {
            let active: Vec<bool> = (0..audio.len()).map(|ch| channel_is_active(&self.layout, self.mono, ch)).collect();
            for ch in 0..audio.len() {
                if !active[ch] {
                    continue;
                }
                let mut one = vec![audio[ch].clone()];
                if ch == 0 {
                    run_processor(&mut self.processor, &mut one, midi, frames);
                } else if let Some(extra) = self.more.get_mut(ch - 1) {
                    run_processor(extra, &mut one, midi, frames);
                } else {
                    continue;
                }
                let count = frames.min(audio[ch].len()).min(one[0].len());
                audio[ch][..count].copy_from_slice(&one[0][..count]);
            }
            return;
        }
        if self.layout == "Mono" {
            let active: Vec<usize> = (0..audio.len()).filter(|ch| channel_is_active(&self.layout, self.mono, *ch)).collect();
            if active.is_empty() {
                return;
            }
            let mut subset: Vec<Vec<f32>> = active.iter().map(|ch| audio[*ch].clone()).collect();
            run_processor(&mut self.processor, &mut subset, midi, frames);
            for (ch, buf) in active.into_iter().zip(subset) {
                let count = frames.min(audio[ch].len()).min(buf.len());
                audio[ch][..count].copy_from_slice(&buf[..count]);
            }
            return;
        }
        run_processor(&mut self.processor, audio, midi, frames);
    }
}

/// A hosted plugin to create for ADD_PLUGIN (see `PluginChain::load_request`).
pub struct LoadRequest {
    pub id: String,
    /// The slot's saved settings; their plugin state is restored before the slot goes live.
    pub settings: String,
    pub sample_rate: f32,
    pub max_block: usize,
    pub bus_channels: usize,
    pub count: usize,
}

impl Drop for PluginChain {
    /// The session ended: close the editor window before the plugins go away.
    fn drop(&mut self) {
        self.close_window();
    }
}

/// The plugin's own state in a settings JSON (`"state"`, base64), if any.
fn state_blob(settings: &str) -> Option<Vec<u8>> {
    let value: serde_json::Value = serde_json::from_str(settings).ok()?;
    base64_decode(value.get("state")?.as_str()?)
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(BASE64[(n >> 18) as usize & 63] as char);
        out.push(BASE64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { BASE64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { BASE64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

fn layout_name(layout: &str) -> String {
    match layout {
        "Mono" | "Stereo" | "Multi-Mono" => layout.to_string(),
        _ => "Stereo".to_string(),
    }
}

fn gain_from_settings(text: &str) -> HostedGain {
    let mut gain = HostedGain::new();
    if let Some(values) = state_values(text) {
        if let Some(value) = values.first() {
            gain.set_gain_db(norm_to_db(*value));
        }
    } else if let Ok(db) = text.parse::<f32>() {
        gain.set_gain_db(db);
    }
    gain
}

/// AudioGridder treats a zero Multi-Mono mask as every output. Any other mask
/// processes only the set bits. Mono is one instance and uses the same bits.
/// A zero Mono mask writes channel 0 only, because the bus itself is mono.
fn channel_is_active(layout: &str, mono: u64, channel: usize) -> bool {
    let bit = 1u64 << channel.min(63);
    match layout {
        "Multi-Mono" => mono == 0 || (mono & bit) != 0,
        "Mono" => {
            if mono == 0 {
                channel == 0
            } else {
                (mono & bit) != 0
            }
        }
        _ => true,
    }
}

fn inactive_snapshot(layout: &str, mono: u64, audio: &[Vec<f32>], frames: usize) -> Vec<(usize, Vec<f32>)> {
    audio
        .iter()
        .enumerate()
        .filter(|(channel, _)| !channel_is_active(layout, mono, *channel))
        .map(|(channel, samples)| {
            let count = frames.min(samples.len());
            (channel, samples[..count].to_vec())
        })
        .collect()
}

fn restore_inactive(audio: &mut [Vec<f32>], dry: &[(usize, Vec<f32>)]) {
    for (channel, saved) in dry {
        if let Some(samples) = audio.get_mut(*channel) {
            let count = saved.len().min(samples.len());
            samples[..count].copy_from_slice(&saved[..count]);
        }
    }
}

fn run_processor(processor: &mut Processor, audio: &mut [Vec<f32>], midi: &[[u8; 3]], frames: usize) {
    match processor {
        Processor::Gain(gain) => gain.process(audio, frames),
        Processor::Plugin(plugin) => plugin.process_f32(audio, midi, frames),
    }
}

fn apply_f64(processor: &mut Processor, audio: &mut [Vec<f64>], midi: &[[u8; 3]], frames: usize) {
    match processor {
        Processor::Gain(gain) => {
            let linear = gain.linear_f64();
            for channel in audio.iter_mut() {
                for sample in channel.iter_mut().take(frames) {
                    *sample *= linear;
                }
            }
        }
        Processor::Plugin(plugin) => plugin.process_f64(audio, midi, frames),
    }
}

fn apply_transport_one(processor: &mut Processor, position: &PositionInfo) {
    match processor {
        Processor::Gain(gain) => gain.set_transport(&crate::transport_from(position)),
        Processor::Plugin(plugin) => plugin.set_transport(position),
    }
}

fn measure_oversampling_latency(sample_rate: f32, factor: i32) -> i32 {
    if factor < 2 {
        return 0;
    }
    let frames = 256;
    let mut impulse = vec![0.0f32; frames];
    impulse[0] = 1.0;
    let filtered = oversampled_identity(sample_rate, factor, &impulse);
    let mut weight = 0.0f32;
    let mut total = 0.0f32;
    for (index, sample) in filtered.iter().enumerate() {
        let energy = sample.abs();
        weight += index as f32 * energy;
        total += energy;
    }
    if total < 1.0e-8 {
        return 1;
    }
    (weight / total).round().max(1.0) as i32
}

fn oversampled_identity(sample_rate: f32, factor: i32, input: &[f32]) -> Vec<f32> {
    let frames = input.len();
    let mut stage_a = Oversampler2x::new(sample_rate);
    let mut up2 = vec![0.0f32; frames * 2];
    stage_a.upsample_2x(input, &mut up2);
    if factor >= 4 {
        let mut stage_b = Oversampler2x::new(sample_rate * 2.0);
        let mut up4 = vec![0.0f32; frames * 4];
        stage_b.upsample_2x(&up2, &mut up4);
        let mut mid = vec![0.0f32; frames * 2];
        stage_b.downsample_2x(&up4, &mut mid);
        up2 = mid;
    }
    let mut out = vec![0.0f32; frames];
    stage_a.downsample_2x(&up2, &mut out);
    out
}

fn builtin_params(slot: &Slot) -> serde_json::Value {
    serde_json::json!([{
        "idx": 0,
        "name": "Gain",
        "defaultValue": 0.5,
        "currentValue": slot.param(0),
        "category": 0,
        "label": "dB",
        "numSteps": 0x7fff_ffff_u32,
        "isBoolean": false,
        "isDiscrete": false,
        "isMeta": false,
        "isOrientInv": false,
        "minValue": "-24.0 dB",
        "maxValue": "24.0 dB",
        "allValues": []
    }])
}

fn playhead_json(position: &PositionInfo) -> serde_json::Value {
    serde_json::json!({
        "bpm": position.bpm,
        "playing": position.playing,
        "samples": position.time_in_samples,
        "ppq": position.ppq_position,
    })
}

fn usize_index(index: i32) -> Option<usize> {
    (index >= 0).then_some(index as usize)
}

/// Plugin-settings chunk. A JSON number or a bare gain string is not a chunk.
fn state_values(text: &str) -> Option<Vec<f32>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    // v1: values only; v2: values plus the plugin's own state.
    if !matches!(value.get("v").and_then(|v| v.as_i64()), Some(1) | Some(2)) {
        return None;
    }
    let params = value.get("params")?.as_array()?;
    params.iter().map(|item| item.as_f64().map(|number| number as f32)).collect()
}

fn recent_id(line: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(line).ok()?.get("id")?.as_str().map(str::to_string)
}

fn norm_to_db(value: f32) -> f32 {
    -24.0 + value.clamp(0.0, 1.0) * 48.0
}

fn db_to_norm(db: f32) -> f32 {
    ((db + 24.0) / 48.0).clamp(0.0, 1.0)
}

fn read_get_parameter(body: &[u8]) -> Option<(i32, i32, i32)> {
    if body.len() < 12 {
        return None;
    }
    let word = |at: usize| -> Option<i32> { Some(i32::from_le_bytes(body[at..at + 4].try_into().ok()?)) };
    Some((word(0)?, word(4)?, word(8)?))
}

fn read_mouse(body: &[u8]) -> Option<(u8, f32, f32, bool, bool, bool, f32, f32)> {
    if body.len() < 15 {
        return None;
    }
    let x = f32::from_le_bytes(body[4..8].try_into().ok()?);
    let y = f32::from_le_bytes(body[8..12].try_into().ok()?);
    // mouseevent_t puts deltaX at byte 16 and deltaY at byte 20. A short body
    // is a click and both deltas stay 0.
    let delta_x = if body.len() >= 20 { f32::from_le_bytes(body[16..20].try_into().ok()?) } else { 0.0 };
    let delta_y = if body.len() >= 24 { f32::from_le_bytes(body[20..24].try_into().ok()?) } else { 0.0 };
    Some((body[0], x, y, body[12] != 0, body[13] != 0, body[14] != 0, delta_x, delta_y))
}

fn read_keys(body: &[u8]) -> Vec<u16> {
    let bytes = command::read_length_prefixed(body).unwrap_or(body);
    bytes.chunks_exact(2).map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_protocol::command::{ADD_PLUGIN, ADD_PLUGIN_RESULT, PARAMETERS, PRESETS, SET_MONO_CHANNELS};

    #[test]
    fn add_plugin_replies_with_result_presets_and_parameters() {
        let mut chain = PluginChain::new(2);
        let body = command::json_payload(&serde_json::json!({"id": BUILTIN_GAIN_ID, "settings": "-6"}));
        let replies = chain.handle(ADD_PLUGIN, &body);
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0].0, ADD_PLUGIN_RESULT);
        assert_eq!(replies[1].0, PRESETS);
        let preset_body = command::read_length_prefixed(&replies[1].1).unwrap();
        assert!(preset_body.is_empty(), "builtin gain has no factory presets");
        assert_eq!(replies[2].0, PARAMETERS);
        let result = command::read_json_payload(&replies[0].1).unwrap();
        assert_eq!(result["success"], true);
        let Processor::Gain(gain) = &chain.slots[0].processor else {
            panic!("builtin slot");
        };
        assert!((gain.gain_db() + 6.0).abs() < 1.0e-6);
    }

    #[test]
    fn mono_layout_leaves_the_unselected_channel_dry() {
        let mut chain = PluginChain::new(2);
        chain.prepare(48_000.0, 32, 2);
        let body = command::json_payload(&serde_json::json!({
            "id": BUILTIN_GAIN_ID,
            "settings": "-6",
            "layout": "Mono",
            "monoChannels": 1
        }));
        let replies = chain.handle(ADD_PLUGIN, &body);
        let result = command::read_json_payload(&replies[0].1).unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["layout"], "Mono");
        assert_eq!(result["monoChannels"], 1);
        assert_eq!(result["channelInstances"], 1);
        let mut audio = vec![vec![1.0f32; 8], vec![1.0; 8]];
        chain.process(&mut audio, 8);
        let expected = 10.0f32.powf(-6.0 / 20.0);
        assert!((audio[0][0] - expected).abs() < 1.0e-5, "left {}", audio[0][0]);
        assert!((audio[1][0] - 1.0).abs() < 1.0e-5, "right {}", audio[1][0]);
    }

    #[test]
    fn multi_mono_mask_uses_a_separate_instance_per_channel() {
        let mut chain = PluginChain::new(2);
        chain.prepare(48_000.0, 32, 2);
        let body = command::json_payload(&serde_json::json!({
            "id": BUILTIN_GAIN_ID,
            "settings": "6|-6",
            "layout": "Multi-Mono",
            "monoChannels": 1
        }));
        let replies = chain.handle(ADD_PLUGIN, &body);
        let result = command::read_json_payload(&replies[0].1).unwrap();
        assert_eq!(result["layout"], "Multi-Mono");
        assert_eq!(result["monoChannels"], 1);
        assert_eq!(result["channelInstances"], 2);
        assert_eq!(chain.slots[0].more.len(), 1, "multi-mono did not keep a second instance");
        let up = 10.0f32.powf(6.0 / 20.0);
        let down = 10.0f32.powf(-6.0 / 20.0);
        let mut audio = vec![vec![1.0f32; 8], vec![1.0; 8]];
        chain.process(&mut audio, 8);
        assert!((audio[0][0] - up).abs() < 1.0e-4, "left {}", audio[0][0]);
        assert!((audio[1][0] - 1.0).abs() < 1.0e-5, "masked channel {}", audio[1][0]);
        chain.handle(SET_MONO_CHANNELS, &command::mono_channels(0, 2));
        let mut audio = vec![vec![1.0f32; 8], vec![1.0; 8]];
        chain.process(&mut audio, 8);
        assert!((audio[0][0] - 1.0).abs() < 1.0e-5, "cleared channel {}", audio[0][0]);
        assert!((audio[1][0] - down).abs() < 1.0e-4, "second instance {}", audio[1][0]);
    }
}
