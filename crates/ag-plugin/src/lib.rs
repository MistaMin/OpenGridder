//! AudioGridder insert, built on OpenPluginSDK.
//!
//! The audio callback does not allocate, lock, or touch the network.
//! A missing server return is the local post-gain signal, so the console
//! (input gain, drive, output gain, VU) is audible before a host connects.

mod drive;
mod session;
mod spsc;
pub mod live;

pub use session::{LoadedSlot, RemoteSession, BUILTIN_GAIN_ID};

use std::sync::{Arc, Condvar, Mutex};

use openplugin::prelude::*;

const PROFILES: &[&str] = &["Low 128", "Std 256", "High 512", "Studio 1024"];
const CHARACTERS: &[&str] = &["Clean", "Tube", "Transformer"];
const QUALITIES: &[&str] = &["ADAA", "2x", "4x"];

const CONSOLE_PARAMS: usize = 9;
pub const AUTOMATION_SLOTS: usize = 16;
const STATE_VERSION: i32 = 7;

static PARAMS: [ParamInfo; CONSOLE_PARAMS + AUTOMATION_SLOTS] = [
    ParamInfo::new("bypass", "Bypass", 0.0, 1.0, 0.0, Unit::Toggle),
    ParamInfo::new("selection", "Remote Plugin", 0.0, 128.0, 0.0, Unit::None),
    ParamInfo::new("latency_profile", "Latency Profile", 0.0, 3.0, 1.0, Unit::Choice).with_choices(PROFILES),
    ParamInfo::new("input_gain", "Input Gain", -24.0, 24.0, 0.0, Unit::Db),
    ParamInfo::new("mix", "Wet/Dry", 0.0, 100.0, 100.0, Unit::Percent),
    ParamInfo::new("drive", "Drive", 0.0, 10.0, 0.0, Unit::None),
    ParamInfo::new("output_gain", "Output Gain", -24.0, 24.0, 0.0, Unit::Db),
    ParamInfo::new("drive_character", "Drive Character", 0.0, 2.0, 0.0, Unit::Choice).with_choices(CHARACTERS),
    ParamInfo::new("drive_quality", "Drive Quality", 0.0, 2.0, 0.0, Unit::Choice).with_choices(QUALITIES),
    ParamInfo::new("slot_0", "Slot 0", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_1", "Slot 1", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_2", "Slot 2", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_3", "Slot 3", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_4", "Slot 4", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_5", "Slot 5", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_6", "Slot 6", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_7", "Slot 7", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_8", "Slot 8", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_9", "Slot 9", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_10", "Slot 10", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_11", "Slot 11", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_12", "Slot 12", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_13", "Slot 13", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_14", "Slot 14", 0.0, 1.0, 0.0, Unit::None),
    ParamInfo::new("slot_15", "Slot 15", 0.0, 1.0, 0.0, Unit::None),
];

static INFO: PluginInfo = PluginInfo {
    id: "com.opengrid.audiogridder",
    name: "OpenGridder",
    vendor: "OpenGrid",
    url: "https://opengrid.studio",
    email: "contact@mistamin.com",
    version: "0.1.1-alpha",
    category: Category::Utility,
    params: &PARAMS,
    is_instrument: false,
    has_editor: true,
};

pub fn profile_samples(profile: i32) -> usize {
    match profile {
        0 => 128,
        1 => 256,
        2 => 512,
        _ => 1024,
    }
}

/// Split `host:port`. A missing or unparsable port becomes 55056, and the whole text stays the host.
pub fn parse_server(text: &str) -> (String, u16) {
    text.rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().ok().map(|port| (host.to_string(), port)))
        .unwrap_or_else(|| (text.to_string(), ag_protocol::SERVER_PORT))
}

/// OpenGridder client transport. 32 through 8192 are kept. Anything else is 256.
pub fn transport_budget(configured: i32) -> usize {
    if (32..=8192).contains(&configured) {
        configured as usize
    } else {
        256
    }
}

/// One plugin remembered in the DAW state chain. This is not a live socket.
#[derive(Clone, Debug)]
pub struct ChainPlugin {
    pub id: String,
    pub settings: String,
    pub bypassed: bool,
    pub layout: String,
    pub mono_channels: u64,
    pub latency: i32,
    pub sidechain_disabled: bool,
    pub oversampling: i32,
    pub params: Vec<(String, f32)>,
}

/// One hosted parameter write produced by an assigned automation slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotWrite {
    pub slot: usize,
    pub plugin: i32,
    pub channel: i32,
    pub param: i32,
    pub value: f32,
}

/// One 3-byte MIDI event queued for the next audio block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueuedMidi {
    pub sample_offset: i32,
    pub data: [u8; 3],
}

fn db_to_lin(db: f32) -> f32 {
    if db.abs() < 1.0e-6 {
        1.0
    } else {
        10.0f32.powf(db / 20.0)
    }
}

fn param_index(id: &str) -> Option<usize> {
    PARAMS[..CONSOLE_PARAMS].iter().position(|p| p.id == id)
}

fn slot_index(id: &str) -> Option<usize> {
    let index = id.strip_prefix("slot_")?.parse::<usize>().ok()?;
    (index < AUTOMATION_SLOTS).then_some(index)
}

#[derive(Clone, Copy)]
struct AutomationSlot {
    plugin: i32,
    channel: i32,
    param: i32,
    value: f32,
    dirty: bool,
}

impl AutomationSlot {
    const fn clear() -> Self {
        Self { plugin: -1, channel: 0, param: 0, value: 0.0, dirty: false }
    }
}

struct WetSlot {
    mutex: Mutex<Option<Vec<Vec<f32>>>>,
    ready: Condvar,
}

impl WetSlot {
    fn new() -> Self {
        Self { mutex: Mutex::new(None), ready: Condvar::new() }
    }
}

/// Handle the network thread uses to hand one wet block to an offline render.
#[derive(Clone)]
pub struct WetHandle {
    slot: Arc<WetSlot>,
}

impl WetHandle {
    pub fn submit(&self, block: Vec<Vec<f32>>) {
        let mut guard = self.slot.mutex.lock().unwrap_or_else(|poison| poison.into_inner());
        *guard = Some(block);
        self.slot.ready.notify_one();
    }
}

const WET_RING: usize = 8193;

struct Delay {
    buffer: Vec<f32>,
    position: usize,
}

impl Delay {
    fn new() -> Self {
        // 8192 samples of transport plus one, so read() does not shorten the maximum.
        Self { buffer: vec![0.0; 8193], position: 0 }
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.position = 0;
    }

    fn read(&mut self, sample: f32, delay: usize) -> f32 {
        if delay == 0 {
            return sample;
        }
        let len = self.buffer.len();
        let delay = delay.min(len - 1);
        self.buffer[self.position] = sample;
        let read_at = (self.position + len - delay) % len;
        let out = self.buffer[read_at];
        self.position = (self.position + 1) % len;
        out
    }
}

pub struct AudioGridder {
    values: [f32; 9],
    sample_rate: f32,
    max_frames: usize,
    channels: usize,
    smooth: f32,
    target_in: f32,
    target_out: f32,
    target_mix: f32,
    sm_in: f32,
    sm_out: f32,
    sm_mix: f32,
    dry: [Delay; 2],
    wet: [Delay; 2],
    input_vu: [VuMeter; 2],
    output_vu: [VuMeter; 2],
    drive_stage: drive::DriveStage,
    drive_env: Vec<f32>,
    in_env: Vec<f32>,
    out_env: Vec<f32>,
    mix_env: Vec<f32>,
    target_drive: f32,
    sm_drive: f32,
    scratch_in: Vec<f32>,
    scratch_out: Vec<f32>,
    /// Post-input-gain samples. The input meter reads this, not the wet mix.
    gained_in: Vec<f32>,
    /// Post-input-gain block for the network thread. The callback does not send it.
    outbound: [Vec<f32>; 2],
    outbound_frames: usize,
    outbound_channels: usize,
    f32_left: Vec<f32>,
    f32_right: Vec<f32>,
    midi_queue: [QueuedMidi; MAX_MIDI_EVENTS],
    midi_count: usize,
    block_events: [QueuedMidi; MAX_MIDI_EVENTS],
    block_count: usize,
    chain_latency: usize,
    active_latency: usize,
    pending_latency: usize,
    manual_latency: usize,
    seen_frames: usize,
    transport: Option<usize>,
    offline: bool,
    chain_ready: bool,
    underruns: u32,
    wait_timeouts: u8,
    pending: Arc<WetSlot>,
    slots: [AutomationSlot; AUTOMATION_SLOTS],
    settings_a: Vec<u8>,
    settings_b: Vec<u8>,
    active_is_b: bool,
    active_server: String,
    discovered: Vec<String>,
    loaded_plugins: Vec<ChainPlugin>,
    active_plugin: i32,
    /// Host CPU percent from the worker. None until a CPU_LOAD reply is stored.
    server_cpu: Option<f32>,
    /// Sample clock for the wet ring. A tag behind this clock is stale.
    clock: u64,
    wet_ring: [Vec<f32>; 2],
    wet_tag: Vec<u64>,
}

const MAX_MIDI_EVENTS: usize = 256;

impl Default for AudioGridder {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioGridder {
    pub fn new() -> Self {
        let mut plugin = Self {
            values: [0.0, 0.0, 1.0, 0.0, 100.0, 0.0, 0.0, 0.0, 0.0],
            sample_rate: 48_000.0,
            max_frames: 512,
            channels: 2,
            smooth: 1.0,
            target_in: 1.0,
            target_out: 1.0,
            target_mix: 1.0,
            sm_in: 1.0,
            sm_out: 1.0,
            sm_mix: 1.0,
            dry: [Delay::new(), Delay::new()],
            wet: [Delay::new(), Delay::new()],
            input_vu: [VuMeter::new(), VuMeter::new()],
            output_vu: [VuMeter::new(), VuMeter::new()],
            drive_stage: drive::DriveStage::new(),
            drive_env: vec![0.0; 512],
            in_env: vec![0.0; 512],
            out_env: vec![0.0; 512],
            mix_env: vec![0.0; 512],
            target_drive: 0.0,
            sm_drive: 0.0,
            scratch_in: vec![0.0; 512],
            scratch_out: vec![0.0; 512],
            gained_in: vec![0.0; 512],
            outbound: [vec![0.0; 512], vec![0.0; 512]],
            outbound_frames: 0,
            outbound_channels: 0,
            f32_left: vec![0.0; 512],
            f32_right: vec![0.0; 512],
            midi_queue: [QueuedMidi { sample_offset: 0, data: [0; 3] }; MAX_MIDI_EVENTS],
            midi_count: 0,
            block_events: [QueuedMidi { sample_offset: 0, data: [0; 3] }; MAX_MIDI_EVENTS],
            block_count: 0,
            chain_latency: 0,
            active_latency: 256,
            pending_latency: 256,
            manual_latency: 0,
            seen_frames: 0,
            transport: None,
            offline: false,
            chain_ready: false,
            underruns: 0,
            wait_timeouts: 0,
            pending: Arc::new(WetSlot::new()),
            slots: [AutomationSlot::clear(); AUTOMATION_SLOTS],
            settings_a: Vec::new(),
            settings_b: Vec::new(),
            active_is_b: false,
            active_server: format!("127.0.0.1:{}", ag_protocol::SERVER_PORT),
            discovered: Vec::new(),
            loaded_plugins: Vec::new(),
            active_plugin: -1,
            server_cpu: None,
            clock: 0,
            wet_ring: [vec![0.0; WET_RING], vec![0.0; WET_RING]],
            wet_tag: vec![u64::MAX; WET_RING],
        };
        plugin.snap_targets();
        plugin
    }

    pub fn output_vu_db(&self, channel: usize) -> f32 {
        self.output_vu.get(channel).map(VuMeter::level_db).unwrap_or(-100.0)
    }

    pub fn input_vu_db(&self, channel: usize) -> f32 {
        self.input_vu.get(channel).map(VuMeter::level_db).unwrap_or(-100.0)
    }

    /// Remember `host:port` and make it the active server. A missing or unparsable port uses 55056.
    pub fn set_server(&mut self, host_port: &str) -> bool {
        let (host, port) = parse_server(host_port);
        let formatted = format!("{host}:{port}");
        self.active_server = formatted.clone();
        if !self.discovered.iter().any(|known| known == &formatted) {
            self.discovered.push(formatted);
        }
        true
    }

    /// `host:port` of the active server. The default is the AudioGridder port.
    pub fn active_server(&self) -> &str {
        &self.active_server
    }

    /// Servers passed to `set_server`, in first-seen order.
    pub fn discovered_servers(&self) -> &[String] {
        &self.discovered
    }

    /// Remember one chain slot for the DAW state document. This does not open a socket.
    pub fn note_loaded_plugin(&mut self, plugin: ChainPlugin) {
        self.loaded_plugins.push(plugin);
    }

    /// Replace the chain stored for the state document (the live chain at save time).
    pub fn set_loaded_plugins(&mut self, plugins: Vec<ChainPlugin>) {
        self.loaded_plugins = plugins;
    }

    /// Chain slots stored for the state document.
    pub fn loaded_plugins(&self) -> &[ChainPlugin] {
        &self.loaded_plugins
    }

    /// Which chain slot the generic knobs draw. A negative index draws none.
    pub fn set_active_plugin(&mut self, index: i32) {
        self.active_plugin = index;
    }

    /// Store one CPU_LOAD reply for the status bar. Non-finite readings are ignored.
    /// This does not open a socket.
    pub fn note_server_cpu(&mut self, load: f32) {
        if load.is_finite() {
            self.server_cpu = Some(load.clamp(0.0, 100.0));
        }
    }

    /// Open the active server. The audio callback does not call this.
    pub fn reconnect(&self, request: ag_protocol::HandshakeRequest) -> std::io::Result<RemoteSession> {
        let (host, port) = parse_server(&self.active_server);
        RemoteSession::connect_host(&host, port, request)
    }

    /// Use a free transport size. Values outside 32..=8192 become 256.
    /// This replaces the latency profile until the plugin is recreated.
    pub fn set_transport(&mut self, configured: i32) {
        self.transport = Some(transport_budget(configured));
        self.update_requested_latency();
    }

    /// Offline bounce may wait for one wet block. The audio callback still does not open a socket.
    pub fn set_offline(&mut self, offline: bool) {
        self.offline = offline;
        if !offline {
            self.wait_timeouts = 0;
        }
    }

    /// The remote chain is loaded. Offline waiting stays off until this is set.
    pub fn note_chain_ready(&mut self, ready: bool) {
        self.chain_ready = ready;
    }

    /// Blocks where a ready chain had no wet sample. A local callback stays at 0.
    pub fn underruns(&self) -> u32 {
        self.underruns
    }

    /// Place one wet block on the ring at `start` plus the profile delay.
    /// A target behind the clock, or past the ring, is dropped.
    pub fn push_tagged_wet(&mut self, start: u64, block: &[Vec<f32>]) {
        let frames = block.first().map(|channel| channel.len()).unwrap_or(0);
        self.push_tagged_wet_frames(start, block, frames);
    }

    /// The live transport reuses full-capacity buffers; only these frames belong to the block.
    pub fn push_tagged_wet_frames(&mut self, start: u64, block: &[Vec<f32>], frames: usize) {
        let quality = self.values[8].round() as i32;
        let align = self.latency().saturating_sub(drive::quality_latency(quality)) as u64;
        let frames = frames.min(block.iter().map(Vec::len).min().unwrap_or(0));
        let channels = block.len().min(self.wet_ring.len());
        let size = self.wet_tag.len() as u64;
        if size == 0 {
            return;
        }
        for n in 0..frames {
            let target = start.saturating_add(n as u64).saturating_add(align);
            if target < self.clock || target >= self.clock.saturating_add(size) {
                continue;
            }
            let index = (target % size) as usize;
            self.wet_tag[index] = target;
            for ch in 0..channels {
                if n < block[ch].len() {
                    self.wet_ring[ch][index] = block[ch][n];
                }
            }
        }
    }

    fn tagged_sample(&self, channel: usize, position: u64) -> Option<f32> {
        let size = self.wet_tag.len();
        if size == 0 || channel >= self.wet_ring.len() {
            return None;
        }
        let index = (position % size as u64) as usize;
        (self.wet_tag[index] == position).then_some(self.wet_ring[channel][index])
    }

    fn wet_covers(&self, frames: usize) -> bool {
        let size = self.wet_tag.len() as u64;
        if size == 0 || frames == 0 {
            return false;
        }
        (0..frames).any(|index| {
            let position = self.clock.saturating_add(index as u64);
            self.wet_tag[(position % size) as usize] == position
        })
    }

    fn clear_wet_ring(&mut self) {
        for channel in &mut self.wet_ring {
            channel.fill(0.0);
        }
        self.wet_tag.fill(u64::MAX);
    }

    /// Clone this onto the thread that receives the server block.
    pub fn wet_handle(&self) -> WetHandle {
        WetHandle { slot: Arc::clone(&self.pending) }
    }

    /// Wait up to 250 ms for a wet block while offline. Three timeouts stop the wait.
    fn offline_block(&mut self, frames: usize) -> Option<Vec<Vec<f32>>> {
        if !self.offline || !self.chain_ready || self.values[0] >= 0.5 || self.wait_timeouts >= 3 || frames == 0 {
            return None;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
        let mut guard = self.pending.mutex.lock().unwrap_or_else(|poison| poison.into_inner());
        loop {
            let ready = guard.as_ref().and_then(|block| block.first()).is_some_and(|channel| channel.len() >= frames);
            if ready {
                self.wait_timeouts = 0;
                return guard.take();
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                self.wait_timeouts = self.wait_timeouts.saturating_add(1);
                return None;
            }
            let waited = self.pending.ready.wait_timeout(guard, deadline - now).unwrap_or_else(|poison| poison.into_inner());
            guard = waited.0;
            if waited.1.timed_out() && !guard.as_ref().and_then(|block| block.first()).is_some_and(|channel| channel.len() >= frames) {
                self.wait_timeouts = self.wait_timeouts.saturating_add(1);
                return None;
            }
        }
    }

    /// Post-input-gain samples from the last callback, then clear them.
    /// The audio callback does not write a socket. The network thread sends this block.
    pub fn take_outbound(&mut self) -> Vec<Vec<f32>> {
        let frames = self.outbound_frames;
        let channels = self.outbound_channels;
        self.outbound_frames = 0;
        self.outbound_channels = 0;
        self.outbound.iter().take(channels).map(|channel| channel[..frames].to_vec()).collect()
    }

    /// Samples the remote chain added on top of the latency profile.
    /// The audio callback does not read this from a socket; the client copies
    /// the last audio-response latency in before the host asks.
    pub fn note_remote_latency(&mut self, samples: usize) {
        self.chain_latency = samples;
        self.update_requested_latency();
    }

    /// Set manual latency in samples (AudioGridder: plugin + NUM_OF_BUFFERS * block + manual).
    pub fn set_manual_latency(&mut self, samples: usize) {
        self.manual_latency = samples;
        self.update_requested_latency();
    }

    pub fn manual_latency(&self) -> usize {
        self.manual_latency
    }

    /// Recompute requested latency taking into account base transport (grown if host block > base),
    /// chain latency, and manual latency.
    fn update_requested_latency(&mut self) {
        let base = self.transport.unwrap_or_else(|| profile_samples(self.values[2].round() as i32));
        // C11: If host block exceeds profile, transport becomes next_pow2(block)+profile, capped at 8192
        let transport = if self.seen_frames > base {
            ((self.seen_frames.next_power_of_two()).saturating_add(base)).min(8192)
        } else {
            base
        };
        // C13: plugin + NUM_OF_BUFFERS * block + manual steps (AudioGridder formula)
        self.pending_latency = transport
            .saturating_add(self.chain_latency)
            .saturating_add(self.manual_latency);
    }

    /// Latency requested from the host.
    pub fn requested_latency(&self) -> usize {
        self.pending_latency
    }

    /// Host latency acceptance gate (C10): applied only after host acknowledges it.
    pub fn accept_latency(&mut self, total: usize) {
        if total == self.pending_latency {
            self.active_latency = self.pending_latency;
        }
    }

    /// Latency currently accepted and active for the audio buffer.
    pub fn active_latency(&self) -> usize {
        self.active_latency
    }

    /// Bind automation slot `slot` to a hosted parameter. A negative plugin clears it.
    pub fn assign_slot(&mut self, slot: usize, plugin: i32, channel: i32, param: i32) -> bool {
        let Some(state) = self.slots.get_mut(slot) else {
            return false;
        };
        if plugin < 0 {
            *state = AutomationSlot::clear();
            return true;
        }
        state.plugin = plugin;
        state.channel = channel;
        state.param = param;
        state.dirty = false;
        true
    }

    /// Snapshot the console onto side A. `toggle_ab` restores it.
    pub fn store_settings_a(&mut self) {
        self.settings_a = self.save_state().unwrap_or_default();
    }

    /// Snapshot the console onto side B.
    pub fn store_settings_b(&mut self) {
        self.settings_b = self.save_state().unwrap_or_default();
    }

    /// Leave the active side and load the other snapshot. The side being left is saved
    /// first, so edits on either side are kept; a side never visited starts as a copy.
    /// (The plug-in formats use `ag_insert_toggle_ab`, which does the same with the remote chain.)
    pub fn toggle_ab(&mut self) -> bool {
        let next = if self.active_is_b {
            self.store_settings_b();
            self.active_is_b = false;
            self.settings_a.clone()
        } else {
            self.store_settings_a();
            self.active_is_b = true;
            self.settings_b.clone()
        };
        next.is_empty() || self.load_state(&next)
    }

    /// Hosted writes queued by an assigned slot since the last take.
    pub fn take_slot_writes(&mut self) -> Vec<SlotWrite> {
        let mut writes = Vec::new();
        for (slot, state) in self.slots.iter_mut().enumerate() {
            if state.plugin < 0 || !state.dirty {
                continue;
            }
            state.dirty = false;
            writes.push(SlotWrite {
                slot,
                plugin: state.plugin,
                channel: state.channel,
                param: state.param,
                value: state.value,
            });
        }
        writes
    }

    /// MIDI bound to the audio block most recently processed. The queue is cleared
    /// onto that block, and the next block does not repeat it.
    pub fn block_midi(&self) -> &[QueuedMidi] {
        &self.block_events[..self.block_count]
    }

    fn bind_midi(&mut self) {
        let count = self.midi_count;
        self.block_events[..count].copy_from_slice(&self.midi_queue[..count]);
        self.block_count = count;
        self.midi_count = 0;
    }

    /// Convert each f64 sample to f32, run the insert, and write f32 results back.
    pub fn process_f64(&mut self, io: &mut [Vec<f64>], frames: usize) {
        let channels = io.len().min(self.channels).min(2);
        if channels == 0 || frames == 0 {
            return;
        }
        let frames = frames.min(self.max_frames).min(io.iter().take(channels).map(|channel| channel.len()).min().unwrap_or(0));
        if frames == 0 {
            return;
        }
        let mut pair = [std::mem::take(&mut self.f32_left), std::mem::take(&mut self.f32_right)];
        for channel in 0..channels {
            for index in 0..frames {
                pair[channel][index] = io[channel][index] as f32;
            }
        }
        self.process_with_remote(&mut pair[..channels], frames, None);
        for channel in 0..channels {
            for index in 0..frames {
                io[channel][index] = f64::from(pair[channel][index]);
            }
        }
        self.f32_left = std::mem::take(&mut pair[0]);
        self.f32_right = std::mem::take(&mut pair[1]);
    }

    /// Editor draw list. This is not the audio callback.
    /// The British console is scaled from the 864-wide layout so it stays inside 720.
    pub fn render_editor(&self, list: &mut DrawList) {
        self.render_editor_with_levels(list, [self.input_vu_db(0), self.input_vu_db(1), self.output_vu_db(0), self.output_vu_db(1)]);
    }

    /// The native editor receives meter snapshots instead of reading live DSP state.
    pub fn render_editor_with_levels(&self, list: &mut DrawList, levels: [f32; 4]) {
        const EDITOR_W: f32 = 720.0;
        const EDITOR_H: f32 = 420.0;
        let place = |x: f32| x * EDITOR_W / 864.0;
        draw_faceplate(list, EDITOR_W, EDITOR_H, FaceplateStyle::BritishConsole);
        let input = PARAMS[3].format(self.values[3]);
        let output = PARAMS[6].format(self.values[6]);
        let drive = PARAMS[5].format(self.values[5]);
        let mix = PARAMS[4].format(self.values[4]);
        draw_knob(list, place(120.0), 250.0, 36.0, PARAMS[3].to_normalized(self.values[3]), "INPUT", &input, KnobStyle::Brit, Color::BRIT_BLUE);
        draw_knob(list, place(250.0), 250.0, 36.0, PARAMS[5].to_normalized(self.values[5]), "DRIVE", &drive, KnobStyle::Bakelite, Color::BAKELITE_BROWN);
        draw_knob(list, place(390.0), 280.0, 28.0, PARAMS[4].to_normalized(self.values[4]), "MIX", &mix, KnobStyle::Brit, Color::BRIT_BLUE);
        draw_knob(list, place(700.0), 250.0, 36.0, PARAMS[6].to_normalized(self.values[6]), "OUTPUT", &output, KnobStyle::Brit, Color::BRIT_RED);
        draw_vu_meter(list, 180.0, 70.0, 110.0, 50.0, levels[0], MeterStyle::AnalogVu);
        draw_vu_meter(list, 180.0, 124.0, 110.0, 50.0, levels[1], MeterStyle::AnalogVu);
        draw_vu_meter(list, place(360.0), 70.0, place(180.0), 110.0, levels[2], MeterStyle::AnalogVu);
        draw_vu_meter(list, place(560.0), 70.0, place(180.0), 110.0, levels[3], MeterStyle::AnalogVu);
        let ink = Color::VINTAGE_CREAM;
        list.text("IN L".to_string(), 186.0, 68.0, 11.0, ink, false);
        list.text("IN R".to_string(), 186.0, 122.0, 11.0, ink, false);
        list.text(format!("BYPASS {}", PARAMS[0].format(self.values[0])), place(48.0), 48.0, 13.0, ink, false);
        list.text(format!("PLUGIN {}", PARAMS[1].format(self.values[1])), place(200.0), 48.0, 13.0, ink, false);
        list.text(format!("LATENCY {}", PARAMS[2].format(self.values[2])), place(360.0), 48.0, 13.0, ink, false);
        list.text(format!("CHARACTER {}", PARAMS[7].format(self.values[7])), place(48.0), 360.0, 13.0, ink, false);
        list.text(format!("QUALITY {}", PARAMS[8].format(self.values[8])), place(250.0), 360.0, 13.0, ink, false);
        for (index, plugin) in self.loaded_plugins.iter().enumerate() {
            let y = 88.0 + index as f32 * 26.0;
            if y > 200.0 {
                break;
            }
            list.fill_rounded_rect(16.0, y, 150.0, 22.0, 4.0, Color::BRIT_BLUE);
            let state = if plugin.bypassed { "BYP" } else { "ON" };
            list.text(
                format!("SLOT {} {state} {}x UP DOWN DEL", plugin.id, plugin.oversampling),
                22.0,
                y + 16.0,
                11.0,
                ink,
                false,
            );
        }
        if let Some(plugin) = self.active_plugin.try_into().ok().and_then(|index: usize| self.loaded_plugins.get(index)) {
            for (index, (name, value)) in plugin.params.iter().enumerate() {
                let x = 180.0 + index as f32 * 70.0;
                if x + 16.0 > EDITOR_W {
                    break;
                }
                let shown = value.clamp(0.0, 1.0);
                draw_knob(list, x, 150.0, 16.0, shown, name, &format!("{:.0}%", shown * 100.0), KnobStyle::Brit, Color::BRIT_BLUE);
            }
        }
        let (server_name, cpu_percent, cpu_color) = match self.server_cpu {
            Some(load) => {
                let color = if load < 50.0 {
                    Color::rgb(0.0, 1.0, 0.0)
                } else if load < 90.0 {
                    Color::rgb(1.0, 1.0, 0.0)
                } else {
                    Color::rgb(1.0, 0.0, 0.0)
                };
                (self.active_server.as_str(), load.round() as i32, color)
            }
            None => ("not connected", 0, Color::WHITE),
        };
        list.text(server_name.to_string(), 16.0, 18.0, 12.0, ink, false);
        list.text(format!("{cpu_percent}%"), 240.0, 18.0, 12.0, cpu_color, false);
    }

    /// `remote` is one block already returned by the server, aligned to this
    /// callback. `None` keeps the insert local.
    pub fn process_with_remote(&mut self, io: &mut [Vec<f32>], frames: usize, remote: Option<&[Vec<f32>]>) {
        self.bind_midi();
        let channels = io.len().min(self.channels).min(2);
        if channels == 0 || frames == 0 {
            self.outbound_frames = 0;
            self.outbound_channels = 0;
            return;
        }
        let frames = frames.min(self.max_frames).min(io.iter().take(channels).map(|c| c.len()).min().unwrap_or(0));
        if frames == 0 {
            self.outbound_frames = 0;
            self.outbound_channels = 0;
            return;
        }
        self.outbound_frames = frames;
        self.outbound_channels = channels;
        if frames > self.seen_frames {
            self.seen_frames = frames;
            self.update_requested_latency();
        }
        let reported = self.latency();
        let bypass = self.values[0] >= 0.5;
        let character = self.values[7].round() as i32;
        let quality = self.values[8].round() as i32;
        let drive_latency = drive::quality_latency(quality);
        let align = reported.saturating_sub(drive_latency);
        for index in 0..frames {
            self.sm_drive += self.smooth * (self.target_drive - self.sm_drive);
            self.drive_env[index] = self.sm_drive;
            self.sm_in += self.smooth * (self.target_in - self.sm_in);
            self.sm_out += self.smooth * (self.target_out - self.sm_out);
            self.sm_mix += self.smooth * (self.target_mix - self.sm_mix);
            self.in_env[index] = self.sm_in;
            self.out_env[index] = self.sm_out;
            self.mix_env[index] = self.sm_mix;
        }

        for ch in 0..channels {
            for i in 0..frames {
                let input = io[ch][i];
                let gained = input * self.in_env[i];
                self.gained_in[i] = gained;
                self.outbound[ch][i] = gained;
                if bypass {
                    let dry = self.dry[ch].read(input, align);
                    self.scratch_in[i] = dry;
                    continue;
                }
                let position = self.clock.saturating_add(i as u64);
                let tagged = self.tagged_sample(ch, position);
                let returned = tagged.unwrap_or_else(|| {
                    remote.and_then(|block| block.get(ch)).and_then(|channel| channel.get(i).copied()).unwrap_or(gained)
                });
                let dry = self.dry[ch].read(gained, align);
                let feed = if tagged.is_some() { gained } else { returned };
                let delayed = self.wet[ch].read(feed, align);
                let wet = tagged.unwrap_or(delayed);
                let mix = self.mix_env[i].clamp(0.0, 1.0);
                self.scratch_in[i] = wet * mix + dry * (1.0 - mix);
            }
            self.input_vu[ch].process_samples(&self.gained_in[..frames]);
            self.apply_drive(ch, frames, bypass, quality, character);
            for i in 0..frames {
                let sample = if bypass { self.scratch_out[i] } else { self.scratch_out[i] * self.out_env[i] };
                io[ch][i] = sample;
                self.scratch_in[i] = sample;
            }
            self.output_vu[ch].process_samples(&self.scratch_in[..frames]);
        }
        if channels == 1 {
            self.input_vu[1].process_samples(&self.gained_in[..frames]);
        }
        self.clock = self.clock.saturating_add(frames as u64);
    }

    fn apply_drive(&mut self, channel: usize, frames: usize, bypass: bool, quality: i32, character: i32) {
        if bypass {
            for index in 0..frames {
                self.scratch_out[index] = self.drive_stage.delay_only(channel, self.scratch_in[index], quality);
            }
            return;
        }
        for index in 0..frames {
            let input = self.scratch_in[index];
            let drive = self.drive_env[index];
            self.scratch_out[index] = self.drive_stage.process_sample(channel, input, drive, quality, character);
        }
    }

    fn snap_targets(&mut self) {
        self.target_in = db_to_lin(self.values[3]);
        self.target_out = db_to_lin(self.values[6]);
        self.target_mix = (self.values[4] / 100.0).clamp(0.0, 1.0);
        self.target_drive = self.values[5];
        self.sm_in = self.target_in;
        self.sm_out = self.target_out;
        self.sm_mix = self.target_mix;
        self.sm_drive = self.target_drive;
    }

    fn ensure_buffers(&mut self) {
        let frames = self.max_frames.max(1);
        if self.scratch_in.len() < frames {
            self.scratch_in.resize(frames, 0.0);
        }
        if self.scratch_out.len() < frames {
            self.scratch_out.resize(frames, 0.0);
        }
        if self.gained_in.len() < frames {
            self.gained_in.resize(frames, 0.0);
        }
        for channel in &mut self.outbound {
            if channel.len() < frames {
                channel.resize(frames, 0.0);
            }
        }
        if self.drive_env.len() < frames {
            self.drive_env.resize(frames, 0.0);
        }
        if self.in_env.len() < frames {
            self.in_env.resize(frames, 0.0);
        }
        if self.out_env.len() < frames {
            self.out_env.resize(frames, 0.0);
        }
        if self.mix_env.len() < frames {
            self.mix_env.resize(frames, 0.0);
        }
        if self.f32_left.len() < frames {
            self.f32_left.resize(frames, 0.0);
        }
        if self.f32_right.len() < frames {
            self.f32_right.resize(frames, 0.0);
        }
    }

    fn apply_console_json(&mut self, value: &serde_json::Value) -> bool {
        const PAIRS: &[(&str, &str)] = &[
            ("bypass", "bypass"),
            ("selectedPlugin", "selection"),
            ("latencyProfile", "latency_profile"),
            ("inputGain", "input_gain"),
            ("wetDryMix", "mix"),
            ("drive", "drive"),
            ("outputGain", "output_gain"),
            ("driveCharacter", "drive_character"),
            ("driveQuality", "drive_quality"),
        ];
        // Validate the whole console document before changing any live value.
        // A DAW may keep processing after rejecting a corrupt state chunk.
        let mut pending = Vec::with_capacity(PAIRS.len());
        for (key, id) in PAIRS {
            let Some(raw) = value.get(*key) else {
                continue;
            };
            let Some(number) = console_json_value(key, raw) else {
                return false;
            };
            pending.push((*id, number));
        }
        for (id, number) in pending {
            self.set_param(id, number);
        }
        true
    }
}

impl Plugin for AudioGridder {
    fn info(&self) -> &'static PluginInfo {
        &INFO
    }

    fn prepare(&mut self, sample_rate: f32, max_block: usize, channels: usize) {
        if !sample_rate.is_finite()
            || sample_rate < 8_000.0
            || sample_rate > 768_000.0
            || !(1..=8192).contains(&max_block)
            || !(1..=2).contains(&channels)
        {
            return;
        }
        self.sample_rate = sample_rate;
        self.max_frames = max_block;
        self.seen_frames = 0;
        self.channels = channels;
        self.smooth = 1.0 - (-1.0 / (0.010 * self.sample_rate)).exp();
        self.ensure_buffers();
        for delay in self.dry.iter_mut().chain(self.wet.iter_mut()) {
            delay.clear();
        }
        for meter in self.input_vu.iter_mut().chain(self.output_vu.iter_mut()) {
            meter.set_sample_rate(self.sample_rate);
            meter.reset();
        }
        self.drive_stage.set_sample_rate(self.sample_rate);
        self.drive_stage.reset();
        self.clock = 0;
        self.clear_wet_ring();
        self.snap_targets();
        self.update_requested_latency();
        self.active_latency = self.pending_latency;
    }

    fn reset(&mut self) {
        for delay in self.dry.iter_mut().chain(self.wet.iter_mut()) {
            delay.clear();
        }
        self.clear_wet_ring();
        for meter in self.input_vu.iter_mut().chain(self.output_vu.iter_mut()) {
            meter.reset();
        }
        self.drive_stage.reset();
    }

    fn set_param(&mut self, id: &str, value: f32) -> bool {
        if let Some(slot) = slot_index(id) {
            let state = &mut self.slots[slot];
            if state.plugin < 0 || !value.is_finite() {
                return true;
            }
            state.value = value.clamp(0.0, 1.0);
            state.dirty = true;
            return true;
        }
        let Some(index) = param_index(id) else {
            return false;
        };
        if !value.is_finite() {
            return true;
        }
        self.values[index] = PARAMS[index].clamp(value);
        self.target_in = db_to_lin(self.values[3]);
        self.target_out = db_to_lin(self.values[6]);
        self.target_mix = (self.values[4] / 100.0).clamp(0.0, 1.0);
        self.target_drive = self.values[5];
        if index == 2 {
            self.update_requested_latency();
        }
        true
    }

    fn param(&self, id: &str) -> Option<f32> {
        if let Some(slot) = slot_index(id) {
            let state = &self.slots[slot];
            return Some(if state.plugin < 0 { 0.0 } else { state.value });
        }
        param_index(id).map(|index| self.values[index])
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        let remote = self.offline_block(frames);
        let missed = self.chain_ready && self.values[0] < 0.5 && frames > 0 && remote.is_none() && !self.wet_covers(frames);
        self.process_with_remote(io, frames, remote.as_deref());
        if missed {
            self.underruns = self.underruns.saturating_add(1);
        }
    }

    fn midi(&mut self, offset: usize, msg: [u8; 3]) {
        if self.midi_count >= MAX_MIDI_EVENTS {
            return;
        }
        self.midi_queue[self.midi_count] = QueuedMidi { sample_offset: offset as i32, data: msg };
        self.midi_count += 1;
    }

    fn latency(&self) -> usize {
        let base = self.transport.unwrap_or_else(|| profile_samples(self.values[2].round() as i32));
        base.saturating_add(self.chain_latency).saturating_add(self.manual_latency)
    }

    fn save_state(&mut self) -> Option<Vec<u8>> {
        let state = serde_json::json!({
            "version": STATE_VERSION,
            "bypass": self.values[0] >= 0.5,
            "selectedPlugin": self.values[1],
            "latencyProfile": self.values[2],
            "inputGain": self.values[3],
            "wetDryMix": self.values[4],
            "drive": self.values[5],
            "outputGain": self.values[6],
            "driveCharacter": self.values[7],
            "driveQuality": self.values[8],
            "activeServer": self.active_server,
            "loadedPlugins": self.loaded_plugins.iter().map(chain_to_json).collect::<Vec<_>>(),
        });
        serde_json::to_vec(&state).ok()
    }

    fn load_state(&mut self, data: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(data) else {
            return false;
        };
        if text.starts_with("AG1 ") {
            let mut pending = Vec::new();
            for token in text.split_whitespace().skip(1) {
                let Some((key, raw)) = token.split_once('=') else {
                    return false;
                };
                let Ok(value) = raw.parse::<f32>() else {
                    return false;
                };
                let id = key_to_id(key);
                if !value.is_finite() || param_index(id).is_none() {
                    return false;
                }
                pending.push((id, value));
            }
            for (id, value) in pending {
                self.set_param(id, value);
            }
            self.snap_targets();
            return true;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) else {
            return false;
        };
        let Some(version) = value.get("version").and_then(|item| item.as_i64()) else {
            return false;
        };
        if !(2..=7).contains(&version) {
            return false;
        }
        if !self.apply_console_json(&value) {
            return false;
        }
        if let Some(server) = value.get("activeServer").and_then(|item| item.as_str()) {
            self.set_server(server);
        }
        if let Some(items) = value.get("loadedPlugins").and_then(|item| item.as_array()) {
            self.loaded_plugins.clear();
            for item in items {
                if let Some(plugin) = chain_from_json(item, version) {
                    self.loaded_plugins.push(plugin);
                }
            }
        }
        self.snap_targets();
        true
    }
}

fn chain_to_json(plugin: &ChainPlugin) -> serde_json::Value {
    serde_json::json!([
        plugin.id,
        plugin.id,
        plugin.settings,
        serde_json::Value::Null,
        [],
        plugin.bypassed,
        plugin.layout,
        plugin.mono_channels,
        plugin.latency,
        "",
        plugin.sidechain_disabled,
        plugin.oversampling,
    ])
}

fn chain_from_json(item: &serde_json::Value, version: i64) -> Option<ChainPlugin> {
    let row = item.as_array()?;
    if row.len() < 6 {
        return None;
    }
    let id = row.first()?.as_str()?.to_string();
    let settings = row.get(2).and_then(|value| value.as_str()).unwrap_or("").to_string();
    let bypassed = row.get(5).and_then(|value| value.as_bool()).unwrap_or(false);
    let layout = if version >= 3 {
        row.get(6).and_then(|value| value.as_str()).unwrap_or("Stereo").to_string()
    } else {
        "Stereo".to_string()
    };
    let mono_channels = if version >= 4 { row.get(7).and_then(|value| value.as_u64()).unwrap_or(0) } else { 0 };
    let latency = if version >= 4 { row.get(8).and_then(|value| value.as_i64()).unwrap_or(0) as i32 } else { 0 };
    let sidechain_disabled = if version >= 6 { row.get(10).and_then(|value| value.as_bool()).unwrap_or(false) } else { false };
    let oversampling = if version >= 7 { row.get(11).and_then(|value| value.as_i64()).unwrap_or(1) as i32 } else { 1 };
    Some(ChainPlugin { id, settings, bypassed, layout, mono_channels, latency, sidechain_disabled, oversampling, params: Vec::new() })
}

fn console_json_value(key: &str, raw: &serde_json::Value) -> Option<f32> {
    if key == "bypass" {
        if let Some(flag) = raw.as_bool() {
            return Some(if flag { 1.0 } else { 0.0 });
        }
    }
    let number = raw.as_f64()?;
    let narrow = number as f32;
    narrow.is_finite().then_some(narrow)
}

fn key_to_id(key: &str) -> &str {
    match key {
        "bypass" => "bypass",
        "selection" => "selection",
        "profile" => "latency_profile",
        "input" => "input_gain",
        "mix" => "mix",
        "drive" => "drive",
        "output" => "output_gain",
        "character" => "drive_character",
        "quality" => "drive_quality",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared() -> AudioGridder {
        let mut plugin = AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.prepare(48_000.0, 4096, 1);
        plugin
    }

    fn render(plugin: &mut AudioGridder, input: &[f32]) -> Vec<f32> {
        let mut io = vec![input.to_vec()];
        plugin.process(&mut io, input.len());
        io.pop().unwrap()
    }

    #[test]
    fn transport_budget_uses_32_through_8192_and_falls_back_to_256() {
        let mut plugin = AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.prepare(48_000.0, 8192, 1);
        assert_eq!(plugin.latency(), 128, "unset transport keeps the profile");
        plugin.set_transport(32);
        assert_eq!(plugin.latency(), 32, "transport 32 replaces profile 128");
        let mut impulse = vec![0.0f32; 64];
        impulse[0] = 0.5;
        let delayed = render(&mut plugin, &impulse);
        assert_eq!(delayed[31], 0.0);
        assert!((delayed[32] - 0.5).abs() < 1.0e-5, "32-sample transport {}", delayed[32]);

        plugin.reset();
        plugin.set_transport(31);
        assert_eq!(plugin.latency(), 256, "31 falls back to 256, not the 128 profile");
        plugin.set_transport(8193);
        assert_eq!(plugin.latency(), 256, "8193 falls back to 256");
        let mut wide = vec![0.0f32; 300];
        wide[0] = 0.25;
        let fallback = render(&mut plugin, &wide);
        assert_eq!(fallback[128], 0.0);
        assert!((fallback[256] - 0.25).abs() < 1.0e-5, "fallback delay {}", fallback[256]);

        plugin.reset();
        plugin.set_transport(8192);
        assert_eq!(plugin.latency(), 8192);
        let mut first = vec![vec![0.0f32; 8192]];
        first[0][0] = 0.25;
        plugin.process(&mut first, 8192);
        assert!(first[0].iter().all(|sample| *sample == 0.0), "first block stays silent");
        let mut second = vec![vec![0.0f32; 8192]];
        plugin.process(&mut second, 8192);
        assert!((second[0][0] - 0.25).abs() < 1.0e-5, "8192-sample impulse {}", second[0][0]);
    }

    #[test]
    fn unity_is_delayed_and_otherwise_unchanged() {
        let mut plugin = prepared();
        let mut input = vec![0.25f32; 256];
        input[0] = 0.5;
        let output = render(&mut plugin, &input);
        assert_eq!(output[127], 0.0);
        assert!((output[128] - 0.5).abs() < 1.0e-5, "delayed sample {}", output[128]);
        assert!((output[255] - 0.25).abs() < 1.0e-5);
    }

    #[test]
    fn input_gain_is_minus_24_db() {
        let mut plugin = prepared();
        plugin.set_param("input_gain", -24.0);
        plugin.prepare(48_000.0, 4096, 1);
        let output = render(&mut plugin, &[0.5; 256]);
        let expected = 0.5 * 10.0f32.powf(-24.0 / 20.0);
        assert!((output[200] - expected).abs() < 1.0e-5, "{} vs {expected}", output[200]);
    }

    #[test]
    fn bypass_ignores_gain() {
        let mut plugin = prepared();
        plugin.set_param("bypass", 1.0);
        plugin.set_param("input_gain", -24.0);
        plugin.prepare(48_000.0, 4096, 1);
        let output = render(&mut plugin, &[0.4; 200]);
        assert!((output[128] - 0.4).abs() < 1.0e-6);
    }

    #[test]
    fn drive_changes_level_and_tube_differs_from_clean() {
        let mut clean = prepared();
        clean.set_param("drive", 8.0);
        clean.prepare(48_000.0, 4096, 1);
        let clean_out = render(&mut clean, &[0.8; 256]);
        let linear = 0.8 * (1.0 + 8.0 * 0.9);
        assert!(clean_out[200].is_finite());
        assert!(clean_out[200].abs() < linear, "saturation should sit under the linear drive {}", clean_out[200]);

        let mut tube = prepared();
        tube.set_param("drive", 8.0);
        tube.set_param("drive_character", 1.0);
        tube.prepare(48_000.0, 4096, 1);
        let tube_out = render(&mut tube, &[0.8; 256]);
        assert!((tube_out[200] - clean_out[200]).abs() > 1.0e-4);

        let mut transformer = prepared();
        transformer.set_param("drive", 8.0);
        transformer.set_param("drive_character", 2.0);
        transformer.prepare(48_000.0, 4096, 1);
        let transformer_out = render(&mut transformer, &[0.8; 256]);
        assert!((transformer_out[200] - clean_out[200]).abs() > 1.0e-4, "transformer differs from clean");
    }

    #[test]
    fn oversampled_drive_stays_finite() {
        let mut plugin = prepared();
        plugin.set_param("drive", 4.0);
        plugin.set_param("drive_quality", 2.0);
        plugin.prepare(48_000.0, 4096, 1);
        let output = render(&mut plugin, &[0.2; 512]);
        assert!(output.iter().all(|s| s.is_finite() && s.abs() < 2.0));
    }

    #[test]
    fn mix_selects_the_server_return() {
        let mut plugin = prepared();
        plugin.set_param("mix", 100.0);
        plugin.prepare(48_000.0, 4096, 1);
        let mut io = vec![vec![0.1f32; 200]];
        let remote = vec![vec![0.7f32; 200]];
        plugin.process_with_remote(&mut io, 200, Some(&remote));
        assert!((io[0][150] - 0.7).abs() < 1.0e-5);

        let mut dry = prepared();
        dry.set_param("mix", 0.0);
        dry.prepare(48_000.0, 4096, 1);
        let mut io = vec![vec![0.1f32; 200]];
        dry.process_with_remote(&mut io, 200, Some(&remote));
        assert!((io[0][150] - 0.1).abs() < 1.0e-5);
    }

    #[test]
    fn input_gain_is_applied_before_the_block_is_sent() {
        use std::net::TcpListener;
        use std::thread;

        use ag_protocol::HandshakeRequest;
        use ag_server::serve_worker;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        session.add_builtin_gain(0.0).unwrap();

        let mut plugin = AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.set_param("input_gain", 6.0);
        plugin.prepare(48_000.0, 256, 2);
        let mut io = vec![vec![1.0f32; 32], vec![1.0; 32]];
        plugin.process(&mut io, 32);
        let staged = plugin.take_outbound();
        let heard = session.exchange(&staged).unwrap();
        let expected = 10.0f32.powf(6.0 / 20.0);
        eprintln!(
            "staged {} {} heard {} callback {} latency {}",
            staged[0][0], staged[1][0], heard[0][0], io[0][0], plugin.latency()
        );
        assert!((staged[0][0] - expected).abs() < 1.0e-4, "left send {}", staged[0][0]);
        assert!((staged[1][0] - expected).abs() < 1.0e-4, "right send {}", staged[1][0]);
        assert!((heard[0][0] - expected).abs() < 1.0e-4, "server {}", heard[0][0]);
        assert!((heard[1][0] - expected).abs() < 1.0e-4, "server right {}", heard[1][0]);
        assert!(io[0][0].abs() < 1.0e-6, "callback played the send {}", io[0][0]);
        assert_eq!(plugin.latency(), 128);
        assert!(plugin.take_outbound().is_empty(), "a second take still had a block");

        let mut bypassed = AudioGridder::new();
        bypassed.set_param("latency_profile", 0.0);
        bypassed.set_param("input_gain", 6.0);
        bypassed.set_param("bypass", 1.0);
        bypassed.prepare(48_000.0, 256, 2);
        let mut raw = vec![vec![1.0f32; 32], vec![1.0; 32]];
        bypassed.process(&mut raw, 32);
        let bypass_send = bypassed.take_outbound();
        eprintln!("bypass send {} heard {}", bypass_send[0][0], raw[0][0]);
        assert!((bypass_send[0][0] - expected).abs() < 1.0e-4, "bypass send {}", bypass_send[0][0]);
        assert!(raw[0][0].abs() < 1.0e-6, "bypass played the send {}", raw[0][0]);
        assert_eq!(bypassed.latency(), 128);

        drop(session);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn input_meter_follows_post_gain_not_the_wet_return() {
        let mut silent = AudioGridder::new();
        silent.set_param("latency_profile", 0.0);
        silent.prepare(48_000.0, 256, 2);
        let mut zeros = vec![vec![0.0f32; 256], vec![0.0; 256]];
        silent.process_with_remote(&mut zeros, 256, None);
        eprintln!("silence {} {}", silent.input_vu_db(0), silent.input_vu_db(1));
        assert_eq!(silent.input_vu_db(0), -100.0, "silence left");
        assert_eq!(silent.input_vu_db(1), -100.0, "silence right");

        let mut plugin = AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.set_param("mix", 100.0);
        plugin.set_param("input_gain", 20.0);
        plugin.prepare(48_000.0, 4096, 2);
        assert_eq!(plugin.latency(), 128, "profile 0 left 128");
        let remote = vec![vec![0.05f32; 4096], vec![0.05; 4096]];
        for _ in 0..24 {
            let mut io = vec![vec![0.05f32; 4096], vec![0.05; 4096]];
            plugin.process_with_remote(&mut io, 4096, Some(&remote));
        }
        let input_db = plugin.input_vu_db(0);
        let output_db = plugin.output_vu_db(0);
        eprintln!("post-gain input {input_db} output {output_db} right {}", plugin.input_vu_db(1));

        let mut mono = AudioGridder::new();
        mono.set_param("latency_profile", 0.0);
        mono.set_param("input_gain", 20.0);
        mono.prepare(48_000.0, 4096, 1);
        for _ in 0..24 {
            let mut io = vec![vec![0.05f32; 4096]];
            mono.process_with_remote(&mut io, 4096, None);
        }
        eprintln!("mono left {} right {}", mono.input_vu_db(0), mono.input_vu_db(1));

        let mut bypassed = AudioGridder::new();
        bypassed.set_param("latency_profile", 0.0);
        bypassed.set_param("bypass", 1.0);
        bypassed.set_param("input_gain", 20.0);
        bypassed.prepare(48_000.0, 4096, 1);
        let mut heard = 0.0f32;
        for _ in 0..24 {
            let mut io = vec![vec![0.05f32; 4096]];
            bypassed.process_with_remote(&mut io, 4096, None);
            heard = io[0][4095];
        }
        eprintln!("bypass heard {heard} meter {}", bypassed.input_vu_db(0));

        let labels = editor_labels(&plugin);
        let has_left = labels.iter().any(|label| label == "IN L");
        let has_right = labels.iter().any(|label| label == "IN R");
        eprintln!("drawn in-l {has_left} in-r {has_right}");

        assert!(input_db > -12.0, "input meter followed the wet return {input_db}");
        assert!(output_db < -20.0, "output meter left the wet return {output_db}");
        assert!((mono.input_vu_db(1) - mono.input_vu_db(0)).abs() < 1.0, "mono right meter was not the left peak");
        assert!((heard - 0.05).abs() < 1.0e-4, "bypass audio changed {heard}");
        assert!(bypassed.input_vu_db(0) > -12.0, "bypass stopped metering post-gain {}", bypassed.input_vu_db(0));
        assert!(has_left && has_right, "input meters are not drawn");
        assert_eq!(plugin.latency(), 128, "metering changed latency");
    }

    #[test]
    fn vu_reads_silence_and_a_sine() {
        let mut plugin = prepared();
        plugin.prepare(48_000.0, 4096, 1);
        let _ = render(&mut plugin, &[0.0; 4800]);
        assert_eq!(plugin.output_vu_db(0), -100.0);

        let sine: Vec<f32> = (0..48_000).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin()).collect();
        for chunk in sine.chunks(4096) {
            let _ = render(&mut plugin, chunk);
        }
        let db = plugin.output_vu_db(0);
        assert!(db > -12.0 && db < 3.0, "vu {db}");
    }

    #[test]
    fn state_records_loaded_plugins() {
        let empty = AudioGridder::new().save_state().unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&empty).unwrap();
        let count = doc.get("loadedPlugins").and_then(|value| value.as_array()).map(|items| items.len());
        eprintln!("loadedPlugins {count:?}");
        assert_eq!(count, Some(0), "saved state has no loadedPlugins");

        let mut plugin = AudioGridder::new();
        plugin.note_loaded_plugin(ChainPlugin {
            id: "Builtin-gain".into(),
            settings: "6".into(),
            bypassed: true,
            layout: "Mono".into(),
            mono_channels: 1,
            latency: 54,
            sidechain_disabled: true,
            oversampling: 4,
            params: Vec::new(),
        });
        let bytes = plugin.save_state().unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let row = &saved["loadedPlugins"][0];
        eprintln!("row {row}");
        assert_eq!(row[0].as_str(), Some("Builtin-gain"));
        assert_eq!(row[2].as_str(), Some("6"));
        assert_eq!(row[5].as_bool(), Some(true));
        assert_eq!(row[6].as_str(), Some("Mono"));
        assert_eq!(row[7].as_u64(), Some(1));
        assert_eq!(row[8].as_i64(), Some(54));
        assert_eq!(row[10].as_bool(), Some(true));
        assert_eq!(row[11].as_i64(), Some(4));

        let mut restored = AudioGridder::new();
        assert!(restored.load_state(&bytes));
        let chain = restored.loaded_plugins();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].id, "Builtin-gain");
        assert_eq!(chain[0].settings, "6");
        assert!(chain[0].bypassed);
        assert_eq!(chain[0].layout, "Mono");
        assert_eq!(chain[0].mono_channels, 1);
        assert_eq!(chain[0].latency, 54);
        assert!(chain[0].sidechain_disabled);
        assert_eq!(chain[0].oversampling, 4);
        assert_eq!(restored.latency(), 256, "the chain record does not change reported latency");

        let version2 = r#"{"version":2,"loadedPlugins":[["Old-id","Name","-6",null,[],false,"Mono",1,54,"",true,4]]}"#;
        let mut early = AudioGridder::new();
        assert!(early.load_state(version2.as_bytes()));
        let early_row = &early.loaded_plugins()[0];
        eprintln!(
            "v2 layout {} mono {} latency {} side {} os {}",
            early_row.layout, early_row.mono_channels, early_row.latency, early_row.sidechain_disabled, early_row.oversampling
        );
        assert_eq!(early_row.id, "Old-id");
        assert_eq!(early_row.settings, "-6");
        assert!(!early_row.bypassed);
        assert_eq!(early_row.layout, "Stereo");
        assert_eq!(early_row.mono_channels, 0);
        assert_eq!(early_row.latency, 0);
        assert!(!early_row.sidechain_disabled);
        assert_eq!(early_row.oversampling, 1);

        let version4 = r#"{"version":4,"loadedPlugins":[["Old-id","Name","-6",null,[],false,"Mono",1,54,"",true,4]]}"#;
        let mut mid = AudioGridder::new();
        assert!(mid.load_state(version4.as_bytes()));
        let mid_row = &mid.loaded_plugins()[0];
        assert_eq!(mid_row.layout, "Mono");
        assert_eq!(mid_row.mono_channels, 1);
        assert_eq!(mid_row.latency, 54);
        assert!(!mid_row.sidechain_disabled);
        assert_eq!(mid_row.oversampling, 1);

        let version6 = r#"{"version":6,"loadedPlugins":[["Old-id","Name","-6",null,[],false,"Mono",1,54,"",true,4]]}"#;
        let mut six = AudioGridder::new();
        assert!(six.load_state(version6.as_bytes()));
        let six_row = &six.loaded_plugins()[0];
        assert!(six_row.sidechain_disabled);
        assert_eq!(six_row.oversampling, 1);
    }

    #[test]
    fn state_remembers_the_active_server() {
        let mut plugin = AudioGridder::new();
        assert!(plugin.set_server("studio.example:55100"));
        let bytes = plugin.save_state().unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let saved = doc.get("activeServer").and_then(|value| value.as_str()).unwrap_or("");
        eprintln!("activeServer {saved}");
        assert_eq!(saved, "studio.example:55100");
        let mut restored = AudioGridder::new();
        assert_eq!(restored.active_server(), "127.0.0.1:55056");
        assert!(restored.load_state(&bytes));
        assert_eq!(restored.active_server(), "studio.example:55100");
        assert!(restored.discovered_servers().iter().any(|known| known == "studio.example:55100"));

        let older = br#"{"version":3,"activeServer":"10.0.0.8"}"#;
        let mut from_old = AudioGridder::new();
        assert!(from_old.load_state(older));
        eprintln!("restored {}", from_old.active_server());
        assert_eq!(from_old.active_server(), "10.0.0.8:55056");
        assert_eq!(from_old.latency(), 256, "loading a server does not change the profile");
    }

    #[test]
    fn state_version_is_7_and_versions_2_through_7_load() {
        let legacy = br#"{"version":7,"outputGain":6.0,"drive":2.5}"#;
        let mut restored = AudioGridder::new();
        let loaded = restored.load_state(legacy);
        eprintln!(
            "version7 loaded {loaded} output {:?} drive {:?}",
            restored.param("output_gain"),
            restored.param("drive")
        );
        assert!(loaded, "version 7 rejected");
        assert_eq!(restored.param("output_gain"), Some(6.0));
        assert_eq!(restored.param("drive"), Some(2.5));

        for version in 2..=7 {
            let blob = format!(r#"{{"version":{version},"outputGain":6.0,"drive":2.5}}"#);
            let mut older = AudioGridder::new();
            assert!(older.load_state(blob.as_bytes()), "version {version} rejected");
            assert_eq!(older.param("output_gain"), Some(6.0), "version {version} output");
            assert_eq!(older.param("drive"), Some(2.5), "version {version} drive");
        }

        let mut rejected = AudioGridder::new();
        assert!(!rejected.load_state(br#"{"version":1,"outputGain":6.0}"#), "version 1 loaded");
        assert_eq!(rejected.param("output_gain"), Some(0.0), "version 1 wrote the gain");
        assert!(!rejected.load_state(br#"{"version":8,"outputGain":6.0}"#), "version 8 loaded");
        assert_eq!(rejected.param("output_gain"), Some(0.0), "version 8 wrote the gain");
        assert!(!rejected.load_state(br#"{"outputGain":6.0}"#), "missing version loaded");

        let mut plugin = prepared();
        plugin.set_param("output_gain", 6.0);
        plugin.set_param("drive", 2.5);
        let bytes = plugin.save_state().unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        eprintln!("saved {text}");
        let doc: serde_json::Value = serde_json::from_slice(&bytes).expect("saved state is JSON");
        assert_eq!(doc.get("version").and_then(|value| value.as_i64()), Some(7));
        let mut from_save = AudioGridder::new();
        assert!(from_save.load_state(&bytes));
        assert_eq!(from_save.param("output_gain"), Some(6.0));
        assert_eq!(from_save.param("drive"), Some(2.5));
        assert_eq!(from_save.latency(), 128, "profile 0 stays 128");

        let ag1 = b"AG1 bypass=0 selection=0 profile=0 input=-12 mix=100 drive=2.5 output=6 character=1 quality=2";
        let mut compat = AudioGridder::new();
        assert!(compat.load_state(ag1), "AG1 text rejected");
        assert_eq!(compat.param("output_gain"), Some(6.0));
        assert_eq!(compat.param("input_gain"), Some(-12.0));
        assert_eq!(compat.latency(), 128);
    }

    fn chain_with(id: &str, params: Vec<(String, f32)>) -> ChainPlugin {
        ChainPlugin {
            id: id.into(),
            settings: String::new(),
            bypassed: false,
            layout: "Stereo".into(),
            mono_channels: 0,
            latency: 0,
            sidechain_disabled: false,
            oversampling: 1,
            params,
        }
    }

    fn editor_text(plugin: &AudioGridder) -> String {
        editor_labels(plugin).join("\n")
    }

    fn editor_labels(plugin: &AudioGridder) -> Vec<String> {
        let mut list = DrawList::new();
        plugin.render_editor(&mut list);
        list.commands.iter().filter_map(|cmd| match cmd {
            openplugin::dri::DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        }).collect()
    }

    #[test]
    fn editor_draws_generic_knobs_for_the_active_plugin() {
        let mut plugin = AudioGridder::new();
        plugin.note_loaded_plugin(chain_with("Filter", vec![("Cutoff".into(), 0.25)]));
        plugin.note_loaded_plugin(chain_with("Delay", vec![("Time".into(), 0.5)]));
        plugin.set_active_plugin(0);
        let first = editor_text(&plugin);
        eprintln!("active0 cutoff {} time {}", first.contains("Cutoff"), first.contains("Time"));
        assert!(first.contains("Cutoff"), "active plugin has no generic knob");
        assert!(!first.contains("Time"), "inactive plugin drew a knob");
        plugin.set_active_plugin(1);
        let second = editor_text(&plugin);
        eprintln!("active1 cutoff {} time {}", second.contains("Cutoff"), second.contains("Time"));
        assert!(second.contains("Time"), "the second plugin has no generic knob");
        assert!(!second.contains("Cutoff"), "the first plugin stayed on the generic row");
        plugin.set_active_plugin(-1);
        let none = editor_text(&plugin);
        assert!(!none.contains("Cutoff") && !none.contains("Time"), "a cleared selection still drew knobs");
        assert!(none.contains("INPUT") && none.contains("OUTPUT"), "console labels left");
    }

    #[test]
    fn status_bar_shows_the_server_and_the_worker_cpu() {
        use std::net::TcpListener;
        use std::thread;

        use ag_protocol::HandshakeRequest;
        use ag_server::serve_worker;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        let mut session = RemoteSession::connect(port, request).unwrap();
        let load = session.cpu_load().unwrap();
        let endpoint = format!("127.0.0.1:{port}");
        let shown = format!("{}%", load.round() as i32);

        let mut plugin = AudioGridder::new();
        assert!(plugin.set_server(&endpoint));
        plugin.note_server_cpu(load);
        let labels = editor_labels(&plugin);
        let has_server = labels.iter().any(|label| label == &endpoint);
        let has_cpu = labels.iter().any(|label| label == &shown);
        eprintln!("status load {load} shown {shown} server {has_server} cpu {has_cpu}");
        assert!(has_server, "status bar has no server name");
        assert!(has_cpu, "status bar has no cpu {shown}");
        assert!(!labels.iter().any(|label| label == "not connected"), "a stored reading still says not connected");
        let text = labels.join("\n");
        assert!(text.contains("INPUT") && text.contains("OUTPUT"), "console labels left");
        assert_eq!(plugin.latency(), 256, "status storage changed latency");

        let idle = editor_labels(&AudioGridder::new());
        assert!(idle.iter().any(|label| label == "not connected"), "disconnected status is missing");
        assert!(idle.iter().any(|label| label == "0%"), "disconnected cpu is missing");
        assert!(!idle.iter().any(|label| label.contains(&endpoint)), "disconnected status used another server");

        drop(session);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn stale_wet_tag_is_dropped() {
        let mut plugin = AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.prepare(48_000.0, 256, 1);
        assert_eq!(plugin.latency(), 128);
        plugin.push_tagged_wet(0, &[vec![0.25; 32]]);
        let mut io = vec![vec![1.0f32; 200]];
        plugin.process(&mut io, 200);
        eprintln!("fresh {}", io[0][128]);
        assert!((io[0][128] - 0.25).abs() < 1.0e-4, "fresh tag was not placed");
        plugin.push_tagged_wet(0, &[vec![0.25; 32]]);
        let mut io = vec![vec![0.5f32; 200]];
        plugin.process(&mut io, 200);
        let stale = io[0].iter().any(|sample| (sample - 0.25).abs() < 1.0e-4);
        eprintln!("stale heard {stale} sample128 {}", io[0][128]);
        assert!(!stale, "stale tag was played");
        assert_eq!(plugin.latency(), 128);
    }

    #[test]
    fn underruns_increment_only_when_the_ready_chain_misses_wet() {
        let mut plugin = AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.prepare(48_000.0, 256, 2);
        let mut io = vec![vec![1.0f32; 32], vec![1.0; 32]];
        plugin.process(&mut io, 32);
        eprintln!("local underruns {}", plugin.underruns());
        assert_eq!(plugin.underruns(), 0, "a local block counted as an underrun");
        assert_eq!(plugin.latency(), 128, "underrun tracking changed latency");

        plugin.note_chain_ready(true);
        plugin.process(&mut io, 0);
        assert_eq!(plugin.underruns(), 0, "an empty block counted as an underrun");
        plugin.process(&mut io, 32);
        eprintln!("missed underruns {}", plugin.underruns());
        assert_eq!(plugin.underruns(), 1, "a missing wet block did not count");
        plugin.process(&mut io, 32);
        assert_eq!(plugin.underruns(), 2, "the next missing block did not count");

        plugin.set_param("bypass", 1.0);
        plugin.process(&mut io, 32);
        assert_eq!(plugin.underruns(), 2, "bypass counted as an underrun");

        plugin.set_param("bypass", 0.0);
        plugin.set_offline(true);
        plugin.wet_handle().submit(vec![vec![0.25; 32], vec![0.25; 32]]);
        plugin.process(&mut io, 32);
        eprintln!("delivered underruns {}", plugin.underruns());
        assert_eq!(plugin.underruns(), 2, "a delivered wet block counted as an underrun");
        assert_eq!(plugin.latency(), 128, "the wet block changed latency");
    }

    #[test]
    fn prepare_rejects_a_rate_block_or_channel_count_outside_the_create_limits() {
        let coeff = 1.0f32 - (-1.0f32 / (0.010 * 48_000.0)).exp();
        let target = 10.0f32.powf(24.0 / 20.0);
        let mut ramp = 1.0f32;
        for _ in 0..=52 {
            ramp += coeff * (target - ramp);
        }

        let mut plugin = AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.prepare(48_000.0, 256, 2);
        plugin.set_param("input_gain", 24.0);
        plugin.prepare(1_000.0, 0, 8);
        plugin.prepare(f32::NAN, 256, 2);
        plugin.prepare(f32::INFINITY, 256, 2);
        plugin.prepare(768_001.0, 256, 2);
        plugin.prepare(48_000.0, 8193, 2);
        plugin.prepare(48_000.0, 256, 0);
        plugin.prepare(48_000.0, 256, 3);
        let mut io = vec![vec![1.0f32; 200], vec![1.0; 200]];
        plugin.process(&mut io, 200);
        eprintln!("rejected left {} right {} ramp {ramp}", io[0][180], io[1][180]);
        assert!((io[0][180] - ramp).abs() < 1.0e-3, "illegal prepare reset the ramp {}", io[0][180]);
        assert!((io[1][180] - io[0][180]).abs() < 1.0e-4, "illegal prepare changed the channel count");
        assert_eq!(plugin.latency(), 128, "illegal prepare changed latency");

        plugin.prepare(8_000.0, 1, 1);
        plugin.prepare(768_000.0, 8192, 2);
        let mut accepted = vec![vec![1.0f32; 200], vec![1.0; 200]];
        plugin.process(&mut accepted, 200);
        eprintln!("accepted {}", accepted[0][180]);
        assert!((accepted[0][180] - target).abs() < 1.0e-3, "a limit prepare was ignored {}", accepted[0][180]);
        assert_eq!(plugin.latency(), 128, "a limit prepare changed latency");
    }

    #[test]
    fn smoother_steps_once_per_sample_with_a_10_ms_pole() {
        let coeff = 1.0f32 - (-1.0f32 / (0.010 * 48_000.0)).exp();
        let target = 10.0f32.powf(24.0 / 20.0);
        let mut smoothed = 1.0f32;
        for _ in 0..=52 {
            smoothed += coeff * (target - smoothed);
        }

        let mut mono = AudioGridder::new();
        mono.set_param("latency_profile", 0.0);
        mono.prepare(48_000.0, 256, 1);
        mono.set_param("input_gain", 24.0);
        let mut mono_io = vec![vec![1.0f32; 200]];
        mono.process(&mut mono_io, 200);

        let mut stereo = AudioGridder::new();
        stereo.set_param("latency_profile", 0.0);
        stereo.prepare(48_000.0, 256, 2);
        stereo.set_param("input_gain", 24.0);
        let mut stereo_io = vec![vec![1.0f32; 200], vec![1.0; 200]];
        stereo.process(&mut stereo_io, 200);

        eprintln!(
            "smooth mono {} left {} right {} expected {smoothed}",
            mono_io[0][180], stereo_io[0][180], stereo_io[1][180]
        );
        assert!((stereo_io[0][180] - stereo_io[1][180]).abs() < 1.0e-4, "channels diverged");
        assert!((mono_io[0][180] - smoothed).abs() < 1.0e-4, "mono pole {}", mono_io[0][180]);
        assert!((stereo_io[0][180] - smoothed).abs() < 1.0e-4, "stereo pole {}", stereo_io[0][180]);
        assert_eq!(mono.latency(), 128, "the smoother changed latency");
        assert_eq!(stereo.latency(), 128, "stereo latency changed");
    }

    #[test]
    fn editor_is_720_by_420_and_draws_chain_buttons() {
        let mut plugin = AudioGridder::new();
        plugin.note_loaded_plugin(ChainPlugin {
            id: "Builtin-gain".into(),
            settings: "6".into(),
            bypassed: true,
            layout: "Mono".into(),
            mono_channels: 1,
            latency: 0,
            sidechain_disabled: false,
            oversampling: 4,
            params: Vec::new(),
        });
        let mut list = DrawList::new();
        plugin.render_editor(&mut list);
        let mut plate_w = 0.0f32;
        let mut plate_h = 0.0f32;
        let mut max_x = 0.0f32;
        for cmd in &list.commands {
            match cmd {
                openplugin::dri::DrawCmd::FillRect { x, y, w, h, .. }
                | openplugin::dri::DrawCmd::FillRoundedRect { x, y, w, h, .. } => {
                    max_x = max_x.max(x + w);
                    if x.abs() < 0.5 && y.abs() < 0.5 && *h >= plate_h {
                        plate_w = *w;
                        plate_h = *h;
                    }
                }
                openplugin::dri::DrawCmd::FillCircle { cx, radius, .. } => {
                    max_x = max_x.max(cx + radius);
                }
                openplugin::dri::DrawCmd::Text { x, .. } => {
                    max_x = max_x.max(*x);
                }
                _ => {}
            }
        }
        let text = list.commands.iter().filter_map(|cmd| match cmd {
            openplugin::dri::DrawCmd::Text { text, .. } => Some(text.as_str()),
            _ => None,
        }).collect::<Vec<_>>().join("\n");
        eprintln!("plate {plate_w}x{plate_h} max_x {max_x} slot {}", text.contains("SLOT Builtin-gain"));
        assert!((plate_w - 720.0).abs() < 0.5 && (plate_h - 420.0).abs() < 0.5, "editor is {plate_w} by {plate_h}");
        assert!(max_x <= 721.5, "a control draws past the 720 plate at {max_x}");
        assert!(text.contains("SLOT Builtin-gain"), "no chain button");
        assert!(text.contains("DEL"), "no delete button");
        for label in ["INPUT", "DRIVE", "OUTPUT", "MIX", "BYPASS", "VU"] {
            assert!(text.contains(label), "editor missing {label}");
        }
    }

    #[test]
    fn state_roundtrip_and_editor_draw() {
        let mut plugin = prepared();
        plugin.set_param("output_gain", 6.0);
        plugin.set_param("drive", 2.5);
        let bytes = plugin.save_state().unwrap();
        let mut restored = AudioGridder::new();
        assert!(restored.load_state(&bytes));
        assert_eq!(restored.param("output_gain"), Some(6.0));
        assert_eq!(restored.param("drive"), Some(2.5));
        let mut list = DrawList::new();
        restored.render_editor(&mut list);
        let text = list.commands.iter().filter_map(|cmd| match cmd {
            openplugin::dri::DrawCmd::Text { text, .. } => Some(text.as_str()),
            _ => None,
        }).collect::<Vec<_>>().join("\n");
        for label in ["INPUT", "DRIVE", "OUTPUT", "MIX", "BYPASS", "PLUGIN", "LATENCY", "CHARACTER", "QUALITY", "VU"] {
            assert!(text.contains(label), "editor missing {label}");
        }
    }

    #[test]
    fn insert_queues_256_midi_events_onto_the_audio_block() {
        let mut plugin = prepared();
        for n in 0..257 {
            let status = if n == 256 { 0x80 } else { 0x90 };
            plugin.midi(n, [status, (n & 0x7f) as u8, 100]);
        }
        let mut io = vec![vec![0.25f32; 64]];
        plugin.process(&mut io, 64);
        let events = plugin.block_midi();
        eprintln!("midi queued {}", events.len());
        assert_eq!(events.len(), 256, "queued {}", events.len());
        assert_eq!(events[0], QueuedMidi { sample_offset: 0, data: [0x90, 0, 100] });
        assert_eq!(events[255].data, [0x90, 127, 100]);
        assert_eq!(events[255].sample_offset, 255);
        plugin.process(&mut io, 64);
        assert_eq!(plugin.block_midi().len(), 0, "the next block repeated the queue");
    }

    #[test]
    fn f64_block_matches_the_f32_process() {
        let mut narrow = prepared();
        let mut wide = prepared();
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.017).sin()).collect();
        let rendered = render(&mut narrow, &input);
        let mut io = vec![input.iter().map(|sample| *sample as f64).collect::<Vec<_>>()];
        wide.process_f64(&mut io, input.len());
        let mut mismatches = 0usize;
        for (index, (got, expect)) in io[0].iter().zip(&rendered).enumerate() {
            if (*got - f64::from(*expect)).abs() > 1.0e-6 {
                if mismatches < 3 {
                    eprintln!("f64 {index} {got} vs {expect}");
                }
                mismatches += 1;
            }
        }
        eprintln!("f64 mismatches {mismatches}");
        assert_eq!(mismatches, 0);
    }

    #[test]
    fn drive_ramps_instead_of_jumping() {
        let mut moving = prepared();
        moving.set_param("drive", 8.0);
        let moving_out = render(&mut moving, &[0.8; 4096]);
        let mut snapped = prepared();
        snapped.set_param("drive", 8.0);
        snapped.prepare(48_000.0, 4096, 1);
        let snapped_out = render(&mut snapped, &[0.8; 4096]);
        let early = snapped_out[128] - moving_out[128];
        let late = (snapped_out[3000] - moving_out[3000]).abs();
        eprintln!("drive early gap {early} late gap {late} moving128 {} snap128 {}", moving_out[128], snapped_out[128]);
        assert!(early > 0.01, "drive jumped to the target on the first audible sample");
        assert!(late < early, "drive did not approach the snapped target");
    }

    #[test]
    fn adaa_differs_from_raw_saturation_and_is_skipped_when_drive_is_off() {
        let mut input = vec![0.0f32; 256];
        for (index, sample) in input.iter_mut().enumerate() {
            *sample = if index % 2 == 0 { 0.8 } else { -0.8 };
        }
        let mut off = prepared();
        let dry = render(&mut off, &input);
        assert_eq!(dry[127], 0.0);
        assert_eq!(dry[128], 0.8);
        assert_eq!(dry[129], -0.8);

        let mut on = prepared();
        on.set_param("drive", 8.0);
        on.set_param("drive_quality", 0.0);
        on.prepare(48_000.0, 4096, 1);
        let wet = render(&mut on, &input);
        let amount = 1.0 + 8.0 * 0.9;
        let mut differ = 0usize;
        for index in 128..256 {
            let plain = saturate_sample(input[index - 128], amount, SaturationMode::Tape);
            if (wet[index] - plain).abs() > 1.0e-3 {
                differ += 1;
            }
        }
        eprintln!("adaa differ {differ} wet200 {}", wet[200]);
        assert!(differ > 0, "quality 0 matched raw saturation");
    }

    #[test]
    fn oversampled_drive_keeps_the_reported_latency() {
        let mut peaks = [0usize; 3];
        let mut reported = [0usize; 3];
        for quality in 0..3 {
            let mut plugin = prepared();
            plugin.set_param("drive", 4.0);
            plugin.set_param("drive_quality", quality as f32);
            plugin.prepare(48_000.0, 4096, 1);
            reported[quality] = plugin.latency();
            let mut input = vec![0.0f32; 512];
            input[0] = 0.2;
            let output = render(&mut plugin, &input);
            peaks[quality] = output
                .iter()
                .enumerate()
                .max_by(|left, right| left.1.abs().partial_cmp(&right.1.abs()).unwrap())
                .unwrap()
                .0;
            eprintln!("quality {quality} latency {} peak {} value {}", reported[quality], peaks[quality], output[peaks[quality]]);
        }
        assert_eq!(reported, [128, 128, 128]);
        assert!(peaks[0].abs_diff(128) <= 1, "adaa peak {}", peaks[0]);
        assert_eq!(peaks[1], 128, "2x moved the impulse");
        assert_eq!(peaks[2], 128, "4x moved the impulse");
    }

    #[test]
    fn ab_toggle_restores_the_console_gain() {
        let mut plugin = prepared();
        plugin.set_param("input_gain", -12.0);
        plugin.prepare(48_000.0, 4096, 1);
        assert!(plugin.toggle_ab()); // A keeps -12 dB; B starts as a copy
        plugin.set_param("input_gain", 6.0);
        plugin.prepare(48_000.0, 4096, 1);
        assert!(plugin.toggle_ab()); // B keeps +6 dB, back on A
        assert!((plugin.param("input_gain").unwrap() - -12.0).abs() < 1.0e-4);
        assert!(plugin.toggle_ab());
        let on_b = render(&mut plugin, &[0.5; 256]);
        let plus = 0.5 * 10.0f32.powf(6.0 / 20.0);
        eprintln!("ab b {}", on_b[200]);
        assert!((on_b[200] - plus).abs() < 1.0e-4, "B {}", on_b[200]);
        assert!(plugin.toggle_ab());
        let on_a = render(&mut plugin, &[0.5; 256]);
        let minus = 0.5 * 10.0f32.powf(-12.0 / 20.0);
        eprintln!("ab a {}", on_a[200]);
        assert!((on_a[200] - minus).abs() < 1.0e-4, "A {}", on_a[200]);
        assert!((plugin.param("input_gain").unwrap() - -12.0).abs() < 1.0e-4);
        assert!(plugin.toggle_ab());
        let back = render(&mut plugin, &[0.5; 256]);
        eprintln!("ab b again {}", back[200]);
        assert!((back[200] - plus).abs() < 1.0e-4, "B again {}", back[200]);
        // An edit on a side that was already stored is kept too.
        plugin.set_param("input_gain", 3.0);
        assert!(plugin.toggle_ab());
        assert!(plugin.toggle_ab());
        assert!((plugin.param("input_gain").unwrap() - 3.0).abs() < 1.0e-4, "B edit lost");
    }

    #[test]
    fn unassigned_automation_slots_stay_zero() {
        let mut plugin = AudioGridder::new();
        let slots: Vec<_> = plugin.info().params.iter().filter(|param| param.id.starts_with("slot_")).map(|param| param.id).collect();
        eprintln!("automation slots {}", slots.len());
        assert_eq!(slots.len(), 16);
        assert_eq!(plugin.param("slot_0"), Some(0.0));
        assert_eq!(plugin.param("slot_15"), Some(0.0));
        assert!(plugin.set_param("slot_0", 0.75));
        assert_eq!(plugin.param("slot_0"), Some(0.0), "unassigned slot stored a write");
        assert!(plugin.assign_slot(0, 1, 2, 3));
        assert!(plugin.set_param("slot_0", 0.625));
        assert_eq!(plugin.param("slot_0"), Some(0.625));
        let writes = plugin.take_slot_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].plugin, 1);
        assert_eq!(writes[0].channel, 2);
        assert_eq!(writes[0].param, 3);
        assert!((writes[0].value - 0.625).abs() < 1.0e-6);
    }

    #[test]
    fn bypass_keeps_the_drive_delay_and_does_not_replay() {
        let mut plugin = prepared();
        plugin.set_param("drive_quality", 2.0);
        plugin.set_param("drive", 4.0);
        plugin.prepare(48_000.0, 4096, 1);
        assert_eq!(plugin.latency(), 128);

        // 100 samples: the impulse has left the 98-sample alignment and is inside
        // the 30-sample drive delay, and it has not reached the reported peak yet.
        let mut head = vec![0.0f32; 100];
        head[0] = 0.5;
        let _ = render(&mut plugin, &head);

        plugin.set_param("bypass", 1.0);
        let mut tail = vec![0.0f32; 160];
        tail[10] = 0.25;
        let out = render(&mut plugin, &tail);
        // Global sample 128 is tail index 28. Rewinding alignment replays 0.5 there.
        let replay = out[28];
        let peak = out
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.abs().partial_cmp(&right.1.abs()).unwrap())
            .unwrap()
            .0;
        eprintln!("bypass replay {replay} marker peak {peak} value {}", out[peak]);
        assert!(replay.abs() < 0.05, "bypass rewound alignment and replayed {replay}");
        assert_eq!(peak, 138, "marker moved to {peak}");
        assert!((out[138] - 0.25).abs() < 1.0e-4, "marker {}", out[138]);
        assert_eq!(plugin.latency(), 128);
    }

    #[test]
    fn non_finite_parameter_write_is_ignored() {
        let mut plugin = AudioGridder::new();
        plugin.set_param("latency_profile", 0.0);
        plugin.set_param("input_gain", -12.0);
        plugin.set_param("output_gain", 6.0);
        plugin.prepare(48_000.0, 4096, 1);
        assert_eq!(plugin.latency(), 128, "profile 0 left 128");
        assert!(plugin.set_param("input_gain", f32::NAN));
        assert!(plugin.set_param("output_gain", f32::INFINITY));
        let stored_in = plugin.param("input_gain").unwrap();
        let stored_out = plugin.param("output_gain").unwrap();
        let mut heard = 0.0f32;
        for _ in 0..24 {
            let output = render(&mut plugin, &[0.5; 4096]);
            heard = output[4095];
        }
        let minus = 0.5 * 10.0f32.powf(-12.0 / 20.0);
        let plus = 0.5 * 10.0f32.powf(6.0 / 20.0);
        eprintln!("nan stored {stored_in} inf stored {stored_out} heard {heard} kept {minus} replaced {plus}");
        assert!((stored_in - -12.0).abs() < 1.0e-6, "nan replaced input gain {stored_in}");
        assert!((stored_out - 6.0).abs() < 1.0e-6, "infinity replaced output gain {stored_out}");
        assert!((heard - minus * plus / 0.5).abs() < 1.0e-3, "non-finite write changed the audio {heard}");
        assert!(plugin.set_param("input_gain", 80.0));
        assert_eq!(plugin.param("input_gain"), Some(24.0), "a finite value still clamps");
        assert_eq!(plugin.latency(), 128, "ignoring a write changed latency");
    }

    #[test]
    fn hostile_parameter_is_clamped() {
        let mut plugin = AudioGridder::new();
        assert!(plugin.set_param("input_gain", f32::NAN));
        assert_eq!(plugin.param("input_gain"), Some(0.0));
        assert!(plugin.set_param("input_gain", 80.0));
        assert_eq!(plugin.param("input_gain"), Some(24.0));
        assert!(!plugin.set_param("nope", 1.0));
    }
}
