//! A bounded audio handoff to a control/network thread. Packet storage is
//! allocated before activation. The realtime callback only moves ownership of
//! those packets, copies samples, and touches atomics; sockets and UI mutexes
//! belong to the worker. Missing/stale packets retain the existing dry fallback.
use crate::{
    session::{AudioLink, CatalogItem, LoadedSlot, ScreenFrame},
    spsc::{self, Consumer, Producer},
    AudioGridder, QueuedMidi, RemoteSession,
};
use og_protocol::{HandshakeRequest, PositionInfo};
use openplugin::prelude::Plugin;
use std::{
    collections::VecDeque,
    net::{Shutdown, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const POOL: usize = 8;
const FRAMES: usize = 8192;

struct Packet {
    audio: Vec<Vec<f32>>,
    frames: usize,
    channels: usize,
    start: u64,
    epoch: u64,
    valid: bool,
    midi: [QueuedMidi; 256],
    midi_count: usize,
    position: PositionInfo,
}
impl Packet {
    fn new() -> Self {
        Self {
            audio: vec![vec![0.; FRAMES]; 2],
            frames: 0,
            channels: 0,
            start: 0,
            epoch: 0,
            valid: false,
            midi: [QueuedMidi {
                sample_offset: 0,
                data: [0; 3],
            }; 256],
            midi_count: 0,
            position: PositionInfo::default(),
        }
    }
}
#[derive(Default)]
pub struct LiveInfo {
    pub connected: bool,
    pub endpoint: String,
    pub catalog: Vec<CatalogItem>,
    pub slots: Vec<LoadedSlot>,
    pub error: Option<String>,
    pub cpu: Option<f32>,
    pub screen: Option<ScreenFrame>,
}
struct Shared {
    stop: AtomicBool,
    connected: AtomicBool,
    ready: AtomicBool,
    epoch: AtomicU64,
    latency: AtomicUsize,
    selection: AtomicI32,
    info: Mutex<LiveInfo>,
    sockets: Mutex<Vec<TcpStream>>,
    /// Editor frames from the screen socket, oldest first. They are PNG deltas over the
    /// previous frame (AudioGridder's diff detection), so they are applied in order.
    screen: Mutex<VecDeque<ScreenFrame>>,
    /// A remote editor is open: set on edit, cleared on hide, a server-side close, or disconnect.
    editor_open: AtomicBool,
    /// The connected session's audio socket, used only by the audio thread, tagged with a
    /// generation so a late failure of an old link cannot drop a newer session.
    audio_link: Mutex<Option<(u64, AudioLink)>>,
    audio_generation: AtomicU64,
    /// Generation of a link the audio thread lost (0: none); the network thread reconnects.
    audio_lost: AtomicU64,
    /// Block period for the audio thread's real-time policy (f64 bits, ms).
    audio_period: AtomicU64,
}
/// Frames kept for the UI. When it falls behind, the oldest go; a full frame follows
/// within a second (every 20th capture), so the picture recovers.
const SCREEN_QUEUE: usize = 64;
pub enum LiveCommand {
    Connect {
        endpoint: String,
        request: HandshakeRequest,
        saved: Vec<LoadedSlot>,
    },
    Disconnect,
    /// Chain restored from the DAW state; it is (re)built on the next connect.
    Restore(Vec<LoadedSlot>),
    Add {
        id: String,
        settings: String,
    },
    Delete(i32),
    Bypass(i32, bool),
    Move(i32, bool),
    Parameter(i32, i32, f32),
    Edit(i32),
    /// Edit with the window's top-left in top-left-origin screen coordinates.
    EditAt(i32, i32, i32),
    Hide,
    Mouse {
        kind: u8,
        x: f32,
        y: f32,
        shift: bool,
        ctrl: bool,
        alt: bool,
        dx: f32,
        dy: f32,
    },
    Key(Vec<u16>),
    /// Fetch the plugin list again (the plugin browser is opening).
    RefreshCatalog,
    /// Read every slot's settings (with the plugin's state) now; reply when done.
    SyncSettings(SyncSender<()>),
    Oversampling(i32, i32),
    Preset(i32, i32),
    /// Apply saved settings (plugin state and values) to a running slot (A/B switch).
    ApplySettings(i32, String),
}
struct Owner {
    shared: Arc<Shared>,
    sender: SyncSender<LiveCommand>,
    thread: Mutex<Option<JoinHandle<()>>>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Ok(sockets) = self.shared.sockets.lock() {
            for s in sockets.iter() {
                let _ = s.shutdown(Shutdown::Both);
            }
        }
        if let Ok(mut handle) = self.thread.lock() {
            if let Some(t) = handle.take() {
                let _ = t.join();
            }
        }
    }
}
#[derive(Clone)]
pub struct LiveController(Arc<Owner>);
impl LiveController {
    /// Bounded control queue; the UI can report backpressure instead of blocking.
    pub fn send(&self, command: LiveCommand) -> bool {
        self.0.sender.try_send(command).is_ok()
    }
    pub fn connect(&self, endpoint: &str, request: HandshakeRequest) -> bool {
        self.send(LiveCommand::Connect {
            endpoint: endpoint.to_owned(),
            request,
            saved: Vec::new(),
        })
    }
    /// Rebuild `saved` on the server, now if connected (by reconnecting) or once a server answers.
    pub fn restore(&self, saved: Vec<LoadedSlot>) -> bool {
        self.send(LiveCommand::Restore(saved))
    }
    pub fn with_info<R>(&self, f: impl FnOnce(&LiveInfo) -> R) -> R {
        let info = self.0.shared.info.lock().unwrap_or_else(|p| p.into_inner());
        f(&info)
    }
    pub fn latency(&self) -> usize {
        self.0.shared.latency.load(Ordering::Acquire)
    }
    pub fn connected(&self) -> bool {
        self.0.shared.connected.load(Ordering::Acquire)
    }
    /// Read every slot's settings from the server now (the DAW is saving). Waits up to
    /// `timeout`; on timeout the last periodic read is saved (AudioGridder blocks the same way).
    pub fn sync_settings(&self, timeout: Duration) -> bool {
        if !self.connected() {
            return false;
        }
        let (done, wait) = mpsc::sync_channel(1);
        if !self.send(LiveCommand::SyncSettings(done)) {
            return false;
        }
        wait.recv_timeout(timeout).is_ok()
    }

    /// The oldest editor frame not yet shown.
    pub fn pop_screen(&self) -> Option<ScreenFrame> {
        self.0.shared.screen.lock().unwrap_or_else(|p| p.into_inner()).pop_front()
    }
    pub fn editor_open(&self) -> bool {
        self.0.shared.editor_open.load(Ordering::Acquire)
    }
    /// Mark the editor open (or closed) now, before the queued command reaches the worker.
    pub fn set_editor_open(&self, open: bool) {
        self.0.shared.editor_open.store(open, Ordering::Release);
    }
}
pub struct LiveAudio {
    shared: Arc<Shared>,
    requests: Producer<Box<Packet>>,
    returns: Consumer<Box<Packet>>,
    free: Vec<Box<Packet>>,
    epoch: u64,
}
impl LiveAudio {
    pub fn new() -> (Self, LiveController) {
        let (requests, receive) = spsc::channel(POOL);
        let (return_tx, returns) = spsc::channel(POOL);
        let (sender, commands) = mpsc::sync_channel(32);
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            connected: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            epoch: AtomicU64::new(1),
            latency: AtomicUsize::new(0),
            selection: AtomicI32::new(0),
            info: Mutex::new(LiveInfo::default()),
            sockets: Mutex::new(Vec::new()),
            screen: Mutex::new(VecDeque::new()),
            editor_open: AtomicBool::new(false),
            audio_link: Mutex::new(None),
            audio_generation: AtomicU64::new(0),
            audio_lost: AtomicU64::new(0),
            audio_period: AtomicU64::new(10.0f64.to_bits()),
        });
        let worker_shared = shared.clone();
        let handle = thread::Builder::new()
            .name("ag-network".into())
            .spawn(move || run(worker_shared, commands))
            .expect("network worker");
        // Audio has its own (real-time) thread, as AudioGridder's AudioStreamer: a slow
        // command (loading a big plugin can take seconds) never holds up the audio.
        let audio_shared = shared.clone();
        thread::Builder::new()
            .name("ag-audio".into())
            .spawn(move || audio_thread(audio_shared, receive, return_tx))
            .expect("audio worker");
        let control = LiveController(Arc::new(Owner {
            shared: shared.clone(),
            sender,
            thread: Mutex::new(Some(handle)),
        }));
        (
            Self {
                shared,
                requests,
                returns,
                free: (0..POOL).map(|_| Box::new(Packet::new())).collect(),
                epoch: 0,
            },
            control,
        )
    }
    fn receive(&mut self, plugin: &mut AudioGridder, wanted: Option<u64>) -> bool {
        let mut found = false;
        while let Some(packet) = self.returns.pop() {
            if packet.valid && packet.epoch == self.epoch {
                plugin.push_tagged_wet_frames(
                    packet.start,
                    &packet.audio[..packet.channels],
                    packet.frames,
                );
                found |= wanted == Some(packet.start);
            }
            self.free.push(packet);
        }
        found
    }
    fn before(&mut self, plugin: &mut AudioGridder) {
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        if epoch != self.epoch {
            self.epoch = epoch;
            plugin.clear_wet_ring();
        }
        plugin.note_chain_ready(self.shared.ready.load(Ordering::Acquire));
        plugin.note_remote_latency(self.shared.latency.load(Ordering::Acquire));
        self.receive(plugin, None);
    }
    fn after(&mut self, plugin: &AudioGridder, start: u64, position: PositionInfo) -> Option<u64> {
        self.shared
            .selection
            .store(plugin.values[1].round() as i32, Ordering::Release);
        if !self.shared.connected.load(Ordering::Acquire) || plugin.outbound_frames == 0 {
            return None;
        }
        let Some(mut packet) = self.free.pop() else {
            return None;
        };
        packet.frames = plugin.outbound_frames.min(FRAMES);
        packet.channels = plugin.outbound_channels.min(2);
        packet.start = start;
        packet.position = position;
        packet.epoch = self.epoch;
        packet.valid = false;
        for ch in 0..packet.channels {
            packet.audio[ch][..packet.frames]
                .copy_from_slice(&plugin.outbound[ch][..packet.frames]);
        }
        let midi = plugin.block_midi();
        packet.midi_count = midi.len().min(256);
        packet.midi[..packet.midi_count].copy_from_slice(&midi[..packet.midi_count]);
        match self.requests.push(packet) {
            Ok(()) => Some(start),
            Err(packet) => {
                self.free.push(packet);
                None
            }
        }
    }
    pub fn process(
        &mut self,
        plugin: &mut AudioGridder,
        io: &mut [Vec<f32>],
        frames: usize,
        position: PositionInfo,
        offline: bool,
    ) {
        self.before(plugin);
        let start = plugin.clock;
        plugin.set_offline(false);
        // Waiting belongs to this tagged transport, not the older untagged offline mailbox.
        plugin.process(io, frames);
        let sent = self.after(plugin, start, position);
        self.offline_wait(plugin, sent, offline);
    }
    pub fn process_f64(
        &mut self,
        plugin: &mut AudioGridder,
        io: &mut [Vec<f64>],
        frames: usize,
        position: PositionInfo,
        offline: bool,
    ) {
        self.before(plugin);
        let start = plugin.clock;
        plugin.set_offline(false);
        plugin.process_f64(io, frames);
        let sent = self.after(plugin, start, position);
        self.offline_wait(plugin, sent, offline);
    }
    fn offline_wait(&mut self, plugin: &mut AudioGridder, sent: Option<u64>, offline: bool) {
        if !offline || sent.is_none() {
            return;
        }
        let deadline = Instant::now() + Duration::from_millis(250);
        while !self.receive(plugin, sent)
            && Instant::now() < deadline
            && self.shared.connected.load(Ordering::Acquire)
        {
            thread::sleep(Duration::from_micros(200));
        }
    }
}
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
/// How often loaded plugins' settings are re-read for state saves and reconnect replays.
// The plugin's full state comes with the settings (it can be large): every 10 s, plus a
// fresh read whenever the DAW saves (`LiveController::sync_settings`).
const SETTINGS_INTERVAL: Duration = Duration::from_secs(10);
struct Target {
    endpoint: String,
    request: HandshakeRequest,
    saved: Vec<LoadedSlot>,
}
/// Keep the chain of a session that just died so the reconnect restores the same plugins.
fn remember_chain(target: &mut Option<Target>, session: &RemoteSession) {
    if let Some(t) = target.as_mut() {
        t.saved = session.loaded().to_vec();
    }
}
/// Socket errors that mean the server went away (restart, rescan, crash), not a refused command.
/// The command channel can no longer be trusted (connection gone, timeout, garbled or late
/// reply). Only a server refusal leaves it in step.
fn breaks_session(e: &std::io::Error) -> bool {
    is_connection_lost(e) || !crate::session::is_refusal(e)
}

fn is_connection_lost(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(e.kind(), BrokenPipe | ConnectionReset | ConnectionAborted | NotConnected | UnexpectedEof)
}
fn snapshot(shared: &Shared, session: &RemoteSession) {
    shared
        .latency
        .store(session.latency_samples().max(0) as usize, Ordering::Release);
    shared
        .ready
        .store(!session.loaded().is_empty(), Ordering::Release);
    let mut info = shared.info.lock().unwrap_or_else(|p| p.into_inner());
    info.connected = true;
    // The error is cleared where things recover (connect, a successful add), not here:
    // this runs after a refused command too, and would hide its message.
    info.slots = session.loaded().to_vec();
}
/// Drain command-in pushes and screen frames on their own threads (blocking reads), so a
/// busy UI or worker never leaves the server's writes stuck. Both end when the sockets
/// are shut down at disconnect.
fn spawn_readers(shared: &Arc<Shared>, mut inbound: TcpStream, mut screen: TcpStream) {
    let screen_shared = Arc::clone(shared);
    let _ = thread::Builder::new().name("ag-screen".into()).spawn(move || loop {
        match crate::session::read_pending(&mut screen) {
            Ok(Some((og_protocol::command::SCREEN_CAPTURE, body))) => {
                if let Some(frame) = crate::session::screen_frame(&body) {
                    let mut queue = screen_shared.screen.lock().unwrap_or_else(|p| p.into_inner());
                    if queue.len() >= SCREEN_QUEUE {
                        queue.pop_front();
                    }
                    queue.push_back(frame);
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    });
    let inbound_shared = Arc::clone(shared);
    let _ = thread::Builder::new().name("ag-command-in".into()).spawn(move || loop {
        match crate::session::read_pending(&mut inbound) {
            // The editor was closed on the server (`Worker::sendHideEditor`).
            Ok(Some((og_protocol::command::HIDE_PLUGIN, _))) => {
                inbound_shared.editor_open.store(false, Ordering::Release);
            }
            Ok(_) => {}
            Err(_) => break,
        }
    });
}

fn disconnect(shared: &Shared, error: Option<String>) {
    shared.connected.store(false, Ordering::Release);
    shared.ready.store(false, Ordering::Release);
    shared.latency.store(0, Ordering::Release);
    shared.epoch.fetch_add(1, Ordering::AcqRel);
    // Shut the sockets down (not just drop these clones) so the reader threads' clones
    // stop blocking and the server sees the session end.
    for socket in shared.sockets.lock().unwrap_or_else(|p| p.into_inner()).drain(..) {
        let _ = socket.shutdown(Shutdown::Both);
    }
    *shared.audio_link.lock().unwrap_or_else(|p| p.into_inner()) = None;
    shared.screen.lock().unwrap_or_else(|p| p.into_inner()).clear();
    shared.editor_open.store(false, Ordering::Release);
    let mut info = shared.info.lock().unwrap_or_else(|p| p.into_inner());
    info.connected = false;
    info.error = error;
    info.screen = None;
}
fn run(shared: Arc<Shared>, commands: mpsc::Receiver<LiveCommand>) {
    let mut session: Option<RemoteSession> = None;
    let mut selected = 0;
    let mut last_cpu = Instant::now();
    // The server this insert should be connected to. While there is no session the worker
    // retries it, so an insert created before the server is up (or one whose server restarted,
    // e.g. for a rescan) connects on its own. `saved` is the chain to restore on reconnect.
    let mut target: Option<Target> = None;
    let mut next_retry = Instant::now();
    let mut last_settings = Instant::now();
    while !shared.stop.load(Ordering::Acquire) {
        while let Ok(command) = commands.try_recv() {
            if let LiveCommand::Connect {
                endpoint,
                request,
                saved,
            } = command
            {
                // Switching or re-announcing the server keeps the chain: the live one if there
                // is a session, otherwise the one waiting to be restored.
                let saved = if !saved.is_empty() {
                    saved
                } else if let Some(s) = session.as_ref() {
                    s.loaded().to_vec()
                } else {
                    target.take().map(|t| t.saved).unwrap_or_default()
                };
                disconnect(&shared, None);
                session = None;
                target = Some(Target { endpoint, request, saved });
                next_retry = Instant::now();
                continue;
            }
            if let LiveCommand::Restore(saved) = command {
                match target.as_mut() {
                    Some(t) => t.saved = saved,
                    None => {
                        target = Some(Target {
                            endpoint: format!("127.0.0.1:{}", og_protocol::SERVER_PORT),
                            request: HandshakeRequest::default(),
                            saved,
                        })
                    }
                }
                if session.take().is_some() {
                    disconnect(&shared, None);
                }
                next_retry = Instant::now();
                continue;
            }
            if matches!(command, LiveCommand::Disconnect) {
                session = None;
                target = None;
                disconnect(&shared, None);
                continue;
            }
            let Some(s) = session.as_mut() else {
                continue;
            };
            let result = match command {
                LiveCommand::Add { id, settings } => s.add_plugin(&id, &settings).map(|()| {
                    shared.info.lock().unwrap_or_else(|p| p.into_inner()).error = None;
                }),
                LiveCommand::Delete(i) => s.unload_plugin(i),
                LiveCommand::Bypass(i, true) => s.bypass_plugin(i),
                LiveCommand::Bypass(i, false) => s.unbypass_plugin(i),
                LiveCommand::Move(i, true) => s.move_up(i),
                LiveCommand::Move(i, false) => s.move_down(i),
                LiveCommand::Parameter(i, p, v) => s.set_parameter(i, p, v),
                LiveCommand::Edit(i) => {
                    shared.screen.lock().unwrap_or_else(|p| p.into_inner()).clear();
                    shared.editor_open.store(true, Ordering::Release);
                    s.edit_plugin(i)
                }
                LiveCommand::EditAt(i, x, y) => {
                    shared.screen.lock().unwrap_or_else(|p| p.into_inner()).clear();
                    shared.editor_open.store(true, Ordering::Release);
                    s.edit_plugin_at(i, x, y)
                }
                LiveCommand::Hide => {
                    shared.editor_open.store(false, Ordering::Release);
                    s.hide_plugin()
                }
                LiveCommand::Mouse { kind, x, y, shift, ctrl, alt, dx, dy } => {
                    s.send_mouse_event(kind, x, y, shift, ctrl, alt, dx, dy)
                }
                LiveCommand::Key(codes) => s.send_key(&codes),
                LiveCommand::SyncSettings(done) => {
                    let result = s.refresh_settings();
                    if result.is_ok() {
                        snapshot(&shared, s); // the saving thread reads these slots next
                    }
                    let _ = done.try_send(());
                    result
                }
                LiveCommand::RefreshCatalog => s.refresh_catalog().map(|()| {
                    shared.info.lock().unwrap_or_else(|p| p.into_inner()).catalog = s.catalog_items().to_vec();
                }),
                LiveCommand::Oversampling(i, n) => s.set_oversampling(i, n),
                LiveCommand::Preset(i, p) => s.set_preset(i, p, 0),
                LiveCommand::ApplySettings(i, settings) => s.apply_settings(i, &settings),
                _ => Ok(()),
            };
            match result {
                // A timeout or a garbled reply leaves a reply in flight: every later command
                // would read the previous one's answer. Reconnect (the chain is restored).
                Err(e) if breaks_session(&e) => {
                    remember_chain(&mut target, s);
                    disconnect(&shared, Some(e.to_string()));
                    session = None;
                    next_retry = Instant::now() + RETRY_INTERVAL;
                }
                Err(e) => {
                    shared.info.lock().unwrap_or_else(|p| p.into_inner()).error = Some(e.to_string());
                    snapshot(&shared, s);
                }
                Ok(()) => snapshot(&shared, s),
            }
        }
        if session.is_none() && Instant::now() >= next_retry {
            if let Some(t) = target.as_ref() {
                shared.info.lock().unwrap_or_else(|p| p.into_inner()).endpoint = t.endpoint.clone();
                let (host, port) = crate::parse_server(&t.endpoint);
                match RemoteSession::connect_host(&host, port, t.request.clone()).and_then(|mut s| {
                    let skipped = s.replay(&t.saved)?;
                    Ok((s, skipped))
                }) {
                    Ok((s, skipped)) => {
                        shared.info.lock().unwrap_or_else(|p| p.into_inner()).error =
                            (!skipped.is_empty()).then(|| format!("Not on this server: {}", skipped.join(", ")));
                        *shared.sockets.lock().unwrap_or_else(|p| p.into_inner()) =
                            s.shutdown_handles().unwrap_or_default();
                        if let Ok(link) = s.audio_link() {
                            let generation = shared.audio_generation.fetch_add(1, Ordering::AcqRel) + 1;
                            let period = og_protocol::realtime::block_period_ms(t.request.samples_per_block, t.request.sample_rate);
                            shared.audio_period.store(period.to_bits(), Ordering::Release);
                            *shared.audio_link.lock().unwrap_or_else(|p| p.into_inner()) = Some((generation, link));
                        }
                        if let Ok((inbound, screen)) = s.reader_sockets() {
                            spawn_readers(&shared, inbound, screen);
                        }
                        shared
                            .info
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .catalog = s.catalog_items().to_vec();
                        snapshot(&shared, &s);
                        shared.connected.store(true, Ordering::Release);
                        session = Some(s);
                        // A replayed chain already holds the selected plugin; do not add it again.
                        selected = if t.saved.is_empty() { 0 } else { shared.selection.load(Ordering::Acquire) };
                        last_settings = Instant::now();
                    }
                    Err(e) => {
                        disconnect(&shared, Some(e.to_string()));
                        next_retry = Instant::now() + RETRY_INTERVAL;
                    }
                }
            }
        }
        if let Some(s) = session.as_mut() {
            let next = shared.selection.load(Ordering::Acquire);
            if next > 0 && next != selected {
                if let Some(id) = s.catalog_ids().get((next - 1) as usize).cloned() {
                    match s.add_plugin(&id, "") {
                        Ok(()) => snapshot(&shared, s),
                        Err(e) if breaks_session(&e) => {
                            remember_chain(&mut target, s);
                            disconnect(&shared, Some(e.to_string()));
                            session = None;
                            next_retry = Instant::now() + RETRY_INTERVAL;
                            selected = next;
                            continue;
                        }
                        Err(e) => {
                            shared.info.lock().unwrap_or_else(|p| p.into_inner()).error =
                                Some(e.to_string())
                        }
                    }
                }
                selected = next;
            }
            if last_cpu.elapsed() > Duration::from_secs(1) {
                last_cpu = Instant::now();
                match s.cpu_load() {
                    Ok(cpu) => shared.info.lock().unwrap_or_else(|p| p.into_inner()).cpu = Some(cpu),
                    Err(e) if breaks_session(&e) => {
                        remember_chain(&mut target, s);
                        disconnect(&shared, Some(e.to_string()));
                        session = None;
                        next_retry = Instant::now() + RETRY_INTERVAL;
                        continue;
                    }
                    Err(_) => {}
                }
            }
            if !s.loaded().is_empty() && last_settings.elapsed() > SETTINGS_INTERVAL {
                last_settings = Instant::now();
                match s.refresh_settings() {
                    Ok(()) => snapshot(&shared, s),
                    Err(e) if breaks_session(&e) => {
                        remember_chain(&mut target, s);
                        disconnect(&shared, Some(e.to_string()));
                        session = None;
                        next_retry = Instant::now() + RETRY_INTERVAL;
                    }
                    Err(_) => {}
                }
            }
        }
        // The audio thread lost this session's audio socket: reconnect.
        let lost = shared.audio_lost.swap(0, Ordering::AcqRel);
        if lost != 0 && lost == shared.audio_generation.load(Ordering::Acquire) {
            if let Some(s) = session.as_ref() {
                remember_chain(&mut target, s);
                disconnect(&shared, Some("audio connection lost".into()));
                session = None;
                next_retry = Instant::now() + RETRY_INTERVAL;
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
}

/// Exchanges the host's audio blocks with the server, on a real-time thread with the block
/// period (AudioGridder's AudioStreamer, `startRealtimeThread`).
fn audio_thread(shared: Arc<Shared>, mut requests: Consumer<Box<Packet>>, mut returns: Producer<Box<Packet>>) {
    let mut applied_period = 0.0f64;
    while !shared.stop.load(Ordering::Acquire) {
        let Some(mut packet) = requests.pop() else {
            thread::sleep(Duration::from_micros(500));
            continue;
        };
        let period = f64::from_bits(shared.audio_period.load(Ordering::Acquire));
        if period != applied_period {
            og_protocol::realtime::make_current_thread_realtime(period);
            applied_period = period;
        }
        if packet.epoch == shared.epoch.load(Ordering::Acquire) {
            let mut link = shared.audio_link.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((generation, audio)) = link.as_mut() {
                let input: Vec<Vec<f32>> = packet.audio[..packet.channels].iter().map(|ch| ch[..packet.frames].to_vec()).collect();
                match audio.exchange(&input, &packet.midi[..packet.midi_count], packet.position.clone()) {
                    Ok(Some(output)) => {
                        for (ch, src) in output.iter().take(packet.channels).enumerate() {
                            packet.audio[ch][..packet.frames].copy_from_slice(&src[..packet.frames]);
                        }
                        packet.valid = true;
                        shared.latency.store(audio.latency_samples().max(0) as usize, Ordering::Release);
                    }
                    // The server is late (overloaded): this block stays dry, the link stays.
                    Ok(None) => {}
                    Err(_) => {
                        shared.audio_lost.store(*generation, Ordering::Release);
                        *link = None;
                    }
                }
            }
        }
        while let Err(back) = returns.push(packet) {
            packet = back;
            if shared.stop.load(Ordering::Acquire) {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_worker_streams_gain_offline_without_socket_calls_on_the_callback() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || og_server::serve_worker(listener));
        let (mut live, control) = LiveAudio::new();
        assert!(control.send(LiveCommand::Connect {
            endpoint: format!("127.0.0.1:{port}"),
            request: HandshakeRequest {
                samples_per_block: 64,
                ..HandshakeRequest::default()
            },
            saved: vec![LoadedSlot {
                name: "OpenGridder Host Gain".into(),
                id: crate::BUILTIN_GAIN_ID.into(),
                settings: "6".into(),
                layout: "Stereo".into(),
                mono: 0,
                bypassed: false,
                oversampling: 1,
                params: Vec::new(),
                presets: Vec::new(),
            }]
        }));
        let limit = Instant::now() + Duration::from_secs(5);
        while !control.connected() && Instant::now() < limit {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            control.connected(),
            "{:?}",
            control.with_info(|i| i.error.clone())
        );
        let mut plugin = AudioGridder::new();
        plugin.set_param("latency_profile", 3.);
        plugin.prepare(48000., 64, 2);
        let mut io = vec![vec![1.; 64]; 2];
        for _ in 0..24 {
            io.iter_mut().for_each(|ch| ch.fill(1.));
            live.process(&mut plugin, &mut io, 64, PositionInfo::default(), true);
        }
        let expected = 10f32.powf(6. / 20.);
        for value in io.iter().flatten() {
            assert!(
                (value - expected).abs() < 1e-5,
                "live sample {value}, expected {expected}"
            );
        }
        assert_eq!(plugin.latency(), 1024);
        assert_eq!(control.with_info(|i| i.slots.len()), 1);
        drop(control);
        drop(live);
        assert!(server.join().unwrap().is_ok());
    }
}
