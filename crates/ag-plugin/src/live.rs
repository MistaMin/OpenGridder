//! A bounded audio handoff to a control/network thread. Packet storage is
//! allocated before activation. The realtime callback only moves ownership of
//! those packets, copies samples, and touches atomics; sockets and UI mutexes
//! belong to the worker. Missing/stale packets retain the existing dry fallback.
use crate::{
    session::{LoadedSlot, ScreenFrame},
    spsc::{self, Consumer, Producer},
    AudioGridder, QueuedMidi, RemoteSession,
};
use ag_protocol::{HandshakeRequest, PositionInfo};
use openplugin::prelude::Plugin;
use std::{
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
    pub catalog: Vec<String>,
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
}
pub enum LiveCommand {
    Connect {
        endpoint: String,
        request: HandshakeRequest,
        saved: Vec<LoadedSlot>,
    },
    Disconnect,
    Add {
        id: String,
        settings: String,
    },
    Delete(i32),
    Bypass(i32, bool),
    Move(i32, bool),
    Parameter(i32, i32, f32),
    Edit(i32),
    Hide,
    Oversampling(i32, i32),
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
        });
        let worker_shared = shared.clone();
        let handle = thread::Builder::new()
            .name("ag-network".into())
            .spawn(move || run(worker_shared, commands, receive, return_tx))
            .expect("network worker");
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
fn snapshot(shared: &Shared, session: &RemoteSession) {
    shared
        .latency
        .store(session.latency_samples().max(0) as usize, Ordering::Release);
    shared
        .ready
        .store(!session.loaded().is_empty(), Ordering::Release);
    let mut info = shared.info.lock().unwrap_or_else(|p| p.into_inner());
    info.connected = true;
    info.error = None;
    info.slots = session.loaded().to_vec();
}
fn disconnect(shared: &Shared, error: Option<String>) {
    shared.connected.store(false, Ordering::Release);
    shared.ready.store(false, Ordering::Release);
    shared.latency.store(0, Ordering::Release);
    shared.epoch.fetch_add(1, Ordering::AcqRel);
    shared
        .sockets
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear();
    let mut info = shared.info.lock().unwrap_or_else(|p| p.into_inner());
    info.connected = false;
    info.error = error;
    info.screen = None;
}
fn run(
    shared: Arc<Shared>,
    commands: mpsc::Receiver<LiveCommand>,
    mut requests: Consumer<Box<Packet>>,
    mut returns: Producer<Box<Packet>>,
) {
    let mut session: Option<RemoteSession> = None;
    let mut selected = 0;
    let mut last_cpu = Instant::now();
    while !shared.stop.load(Ordering::Acquire) {
        while let Ok(command) = commands.try_recv() {
            if let LiveCommand::Connect {
                endpoint,
                request,
                saved,
            } = command
            {
                disconnect(&shared, None);
                session = None;
                let (host, port) = crate::parse_server(&endpoint);
                shared
                    .info
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .endpoint = endpoint;
                match RemoteSession::connect_host(&host, port, request).and_then(|mut s| {
                    s.replay(&saved)?;
                    Ok(s)
                }) {
                    Ok(s) => {
                        *shared.sockets.lock().unwrap_or_else(|p| p.into_inner()) =
                            s.shutdown_handles().unwrap_or_default();
                        shared
                            .info
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .catalog = s.catalog_ids().to_vec();
                        snapshot(&shared, &s);
                        shared.connected.store(true, Ordering::Release);
                        session = Some(s);
                        selected = 0;
                    }
                    Err(e) => disconnect(&shared, Some(e.to_string())),
                }
                continue;
            }
            if matches!(command, LiveCommand::Disconnect) {
                session = None;
                disconnect(&shared, None);
                continue;
            }
            let Some(s) = session.as_mut() else {
                continue;
            };
            let result = match command {
                LiveCommand::Add { id, settings } => s.add_plugin(&id, &settings),
                LiveCommand::Delete(i) => s.unload_plugin(i),
                LiveCommand::Bypass(i, true) => s.bypass_plugin(i),
                LiveCommand::Bypass(i, false) => s.unbypass_plugin(i),
                LiveCommand::Move(i, true) => s.move_up(i),
                LiveCommand::Move(i, false) => s.move_down(i),
                LiveCommand::Parameter(i, p, v) => s.set_parameter(i, p, v),
                LiveCommand::Edit(i) => s.edit_plugin(i),
                LiveCommand::Hide => s.hide_plugin(),
                LiveCommand::Oversampling(i, n) => s.set_oversampling(i, n),
                _ => Ok(()),
            };
            if let Err(e) = result {
                shared.info.lock().unwrap_or_else(|p| p.into_inner()).error = Some(e.to_string());
            } else {
                snapshot(&shared, s);
            }
        }
        if let Some(s) = session.as_mut() {
            let next = shared.selection.load(Ordering::Acquire);
            if next > 0 && next != selected {
                if let Some(id) = s.catalog_ids().get((next - 1) as usize).cloned() {
                    match s.add_plugin(&id, "") {
                        Ok(()) => snapshot(&shared, s),
                        Err(e) => {
                            shared.info.lock().unwrap_or_else(|p| p.into_inner()).error =
                                Some(e.to_string())
                        }
                    }
                }
                selected = next;
            }
            if last_cpu.elapsed() > Duration::from_secs(1) {
                if let Ok(cpu) = s.cpu_load() {
                    shared.info.lock().unwrap_or_else(|p| p.into_inner()).cpu = Some(cpu);
                }
                last_cpu = Instant::now();
            }
        }
        let Some(mut packet) = requests.pop() else {
            thread::sleep(Duration::from_millis(1));
            continue;
        };
        if packet.epoch == shared.epoch.load(Ordering::Acquire) {
            if let Some(s) = session.as_mut() {
                let input: Vec<Vec<f32>> = packet.audio[..packet.channels]
                    .iter()
                    .map(|ch| ch[..packet.frames].to_vec())
                    .collect();
                match s.exchange_midi_at(
                    &input,
                    &packet.midi[..packet.midi_count],
                    packet.position.clone(),
                ) {
                    Ok((audio, _)) => {
                        for (ch, src) in audio.iter().take(packet.channels).enumerate() {
                            packet.audio[ch][..packet.frames]
                                .copy_from_slice(&src[..packet.frames]);
                        }
                        packet.valid = true;
                        shared
                            .latency
                            .store(s.latency_samples().max(0) as usize, Ordering::Release);
                    }
                    Err(e) => {
                        disconnect(&shared, Some(e.to_string()));
                        session = None;
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
        let server = thread::spawn(move || ag_server::serve_worker(listener));
        let (mut live, control) = LiveAudio::new();
        assert!(control.send(LiveCommand::Connect {
            endpoint: format!("127.0.0.1:{port}"),
            request: HandshakeRequest {
                samples_per_block: 64,
                ..HandshakeRequest::default()
            },
            saved: vec![LoadedSlot {
                id: crate::BUILTIN_GAIN_ID.into(),
                settings: "6".into(),
                layout: "Stereo".into(),
                mono: 0,
                bypassed: false,
                params: Vec::new(),
                presets: Vec::new()
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
