//! SDK-independent native FX runtime. All network operations and chain changes
//! run on a worker. The host callback only transfers preallocated audio slots
//! and reads/writes a timestamped output timeline; it never waits for the worker.
use crate::spsc::{self, Consumer, Producer};
use crate::{PluginMode, PluginProcessor};
use og_common::message::PositionInfo;
use std::cell::UnsafeCell;
use std::ffi::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const POOL: usize = 8;
const MAX_PLUGIN_LATENCY: usize = 65536;
const MAX_TRANSPORT_LATENCY: usize = 8192;
const TRANSPORT_BITS: u32 = 14;
const TRANSPORT_MASK: u32 = (1 << TRANSPORT_BITS) - 1;
const EMPTY: u64 = u64::MAX;

// One atomic word publishes a coherent transport/plugin latency pair. Use a
// 32-bit atomic so the same synchronization design is suitable for legacy x86.
fn latency_pair(transport: u32, plugin: u32) -> u32 {
    (plugin << TRANSPORT_BITS) | transport
}
fn transport_part(pair: u32) -> u32 {
    pair & TRANSPORT_MASK
}
fn plugin_part(pair: u32) -> u32 {
    pair >> TRANSPORT_BITS
}
fn total_latency(pair: u32) -> u32 {
    transport_part(pair) + plugin_part(pair)
}

struct Chunk {
    start: u64,
    frames: usize,
    audio: Vec<Vec<f32>>,
    valid: bool,
    selection: u32,
    latency: u32,
}

struct Shared {
    stop: AtomicBool,
    selected: AtomicU32,
    bypass: AtomicBool,
    active_latency: AtomicU32,
    requested_latency: AtomicU32,
    processed: AtomicU32,
    status: AtomicI32,
    underruns: AtomicU32,
    names: Mutex<Vec<String>>,
}

impl Shared {
    // Worker-only update. Preserve any transport-budget request from the host callback.
    fn request_plugin_latency(&self, plugin: u32) {
        let _ =
            self.requested_latency
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    Some(latency_pair(
                        transport_part(current),
                        plugin.min(MAX_PLUGIN_LATENCY as u32),
                    ))
                });
    }
}

struct Callback {
    send: Producer<Box<Chunk>>,
    receive: Consumer<Box<Chunk>>,
    free: Vec<Box<Chunk>>,
    clock: u64,
    dry: Vec<Vec<f32>>,
    wet: Vec<Vec<f32>>,
    dry_tags: Vec<u64>,
    wet_tags: Vec<u64>,
    selection: u32,
    plugin_latency: usize,
    transport_latency: usize,
}

/// Opaque handle. Only one audio thread may process it at a time; control
/// functions only access atomics/metadata. Destruction requires processing to stop.
pub struct NativePlugin {
    shared: Arc<Shared>,
    audio: UnsafeCell<Callback>,
    worker: Option<JoinHandle<()>>,
    channels: usize,
    max_frames: usize,
}

impl NativePlugin {
    fn new(
        rate: f64,
        max_frames: usize,
        channels: usize,
        latency: usize,
        endpoint: Option<(String, u16)>,
    ) -> Self {
        let (send, mut incoming) = spsc::channel::<Box<Chunk>>(POOL);
        let (mut outgoing, receive) = spsc::channel::<Box<Chunk>>(POOL);
        let free = (0..POOL)
            .map(|_| {
                Box::new(Chunk {
                    start: 0,
                    frames: 0,
                    audio: vec![vec![0.0; max_frames]; channels],
                    valid: false,
                    selection: 0,
                    latency: 0,
                })
            })
            .collect();
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            selected: AtomicU32::new(0),
            bypass: AtomicBool::new(false),
            active_latency: AtomicU32::new(latency_pair(latency as u32, 0)),
            requested_latency: AtomicU32::new(latency_pair(latency as u32, 0)),
            processed: AtomicU32::new(0),
            status: AtomicI32::new(0),
            underruns: AtomicU32::new(0),
            names: Mutex::new(Vec::new()),
        });
        let capacity = MAX_PLUGIN_LATENCY + MAX_TRANSPORT_LATENCY + max_frames + 1;
        let mut this = Self {
            shared: shared.clone(),
            channels,
            max_frames,
            audio: UnsafeCell::new(Callback {
                send,
                receive,
                free,
                clock: 0,
                dry: vec![vec![0.0; capacity]; channels],
                wet: vec![vec![0.0; capacity]; channels],
                dry_tags: vec![EMPTY; capacity],
                wet_tags: vec![EMPTY; capacity],
                selection: 0,
                plugin_latency: 0,
                transport_latency: latency,
            }),
            worker: None,
        };
        if let Some((host, port)) = endpoint {
            this.worker = Some(
                thread::Builder::new()
                    .name("OpenGridder native IO".into())
                    .spawn(move || {
                        let mut processor = PluginProcessor::new(PluginMode::Fx);
                        processor.prepare_to_play(rate, max_frames);
                        let mut cfg = processor.client.config();
                        cfg.channels_in = channels as i32;
                        cfg.channels_out = channels as i32;
                        cfg.active_channels = (1 << channels) - 1;
                        processor.client.set_config(cfg);
                        let mut retry = Instant::now();
                        let mut applied = u32::MAX;
                        while !shared.stop.load(Ordering::Acquire) {
                            if !processor.client.is_ready() && Instant::now() >= retry {
                                shared.status.store(1, Ordering::Release);
                                if processor.connect_to_server(&host, port).is_ok() {
                                    *shared.names.lock().unwrap_or_else(|e| e.into_inner()) =
                                        processor
                                            .client
                                            .available_plugins()
                                            .iter()
                                            .map(|p| p.name.clone())
                                            .collect();
                                    applied = u32::MAX;
                                    shared.status.store(2, Ordering::Release);
                                } else {
                                    shared.status.store(0, Ordering::Release);
                                }
                                retry = Instant::now() + Duration::from_secs(1);
                            }
                            let selected = shared.selected.load(Ordering::Acquire);
                            if processor.client.is_ready() && applied != selected {
                                while !processor.loaded_plugins.is_empty() {
                                    if processor.unload_plugin(0).is_err() {
                                        break;
                                    }
                                }
                                let list = processor.client.available_plugins();
                                let loaded = if selected == 0 {
                                    true
                                } else if let Some(desc) = list.get(selected as usize - 1) {
                                    processor
                                        .load_plugin(
                                            desc,
                                            if channels == 1 { "Mono" } else { "Stereo" },
                                        )
                                        .is_ok()
                                } else {
                                    false
                                };
                                shared.status.store(
                                    if loaded {
                                        if selected == 0 {
                                            2
                                        } else {
                                            3
                                        }
                                    } else {
                                        -1
                                    },
                                    Ordering::Release,
                                );
                                shared.request_plugin_latency(
                                        processor.latency_samples.max(0) as u32
                                    );
                                applied = selected;
                            }
                            if let Some(mut chunk) = incoming.pop() {
                                chunk.valid = false;
                                if processor.client.is_ready()
                                    && shared.status.load(Ordering::Acquire) == 3
                                    && chunk.selection == applied
                                {
                                    for channel in &mut chunk.audio {
                                        channel.truncate(chunk.frames);
                                    }
                                    let mut midi = Vec::new();
                                    match processor.client.process_audio_f32(
                                        &mut chunk.audio,
                                        &mut midi,
                                        &PositionInfo::default(),
                                    ) {
                                        Ok(delay)
                                            if delay >= 0
                                                && (delay as usize) <= MAX_PLUGIN_LATENCY =>
                                        {
                                            shared.request_plugin_latency(delay as u32);
                                            let active =
                                                shared.active_latency.load(Ordering::Acquire);
                                            chunk.valid = plugin_part(active) == delay as u32
                                                && chunk.latency == active;
                                            shared.processed.fetch_add(1, Ordering::Relaxed);
                                        }
                                        _ => {
                                            processor.client.disconnect();
                                            shared.status.store(0, Ordering::Release);
                                        }
                                    }
                                    for channel in &mut chunk.audio {
                                        channel.resize(max_frames, 0.0);
                                    }
                                }
                                // A pool slot is always in exactly one of the queues or endpoint
                                // owners. Therefore a POOL-sized return queue has room here.
                                if outgoing.push(chunk).is_err() {
                                    break;
                                }
                            } else {
                                thread::sleep(Duration::from_micros(200));
                            }
                        }
                        processor.client.disconnect();
                    })
                    .expect("native worker creation failed"),
            );
        }
        this
    }

    unsafe fn process(
        &self,
        inputs: *const *const f32,
        outputs: *const *mut f32,
        frames: usize,
    ) -> i32 {
        if frames == 0 {
            return 0;
        }
        if frames > self.max_frames || inputs.is_null() || outputs.is_null() {
            return -1;
        }
        let input = std::slice::from_raw_parts(inputs, self.channels);
        let output = std::slice::from_raw_parts(outputs, self.channels);
        if input.iter().any(|p| p.is_null()) || output.iter().any(|p| p.is_null()) {
            return -1;
        }
        // SAFETY: the adapter guarantees exactly one callback at a time.
        let cb = &mut *self.audio.get();
        let size = cb.dry_tags.len();
        let selection = self.shared.selected.load(Ordering::Acquire);
        let requested = self.shared.requested_latency.load(Ordering::Acquire);
        if frames > transport_part(requested) as usize {
            let expanded = frames.next_power_of_two().min(MAX_TRANSPORT_LATENCY) as u32;
            // At most one CAS attempt per callback: a racing worker update can
            // defer the request to the next callback, never make this callback wait.
            let _ = self.shared.requested_latency.compare_exchange(
                requested,
                latency_pair(expanded, plugin_part(requested)),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        let active = self.shared.active_latency.load(Ordering::Acquire);
        let plugin_delay = plugin_part(active) as usize;
        let transport_delay = transport_part(active) as usize;
        if cb.selection != selection
            || cb.plugin_latency != plugin_delay
            || cb.transport_latency != transport_delay
        {
            cb.wet_tags.fill(EMPTY);
            cb.dry_tags.fill(EMPTY);
            cb.selection = selection;
            cb.plugin_latency = plugin_delay;
            cb.transport_latency = transport_delay;
        }
        for _ in 0..POOL {
            let Some(chunk) = cb.receive.pop() else {
                break;
            };
            if chunk.valid && chunk.selection == selection && chunk.latency == active {
                for n in 0..chunk.frames {
                    let target = chunk.start + n as u64 + transport_delay as u64;
                    if target >= cb.clock && target < cb.clock + size as u64 {
                        let index = target as usize % size;
                        for ch in 0..self.channels {
                            cb.wet[ch][index] = chunk.audio[ch][n];
                        }
                        cb.wet_tags[index] = target;
                    }
                }
            }
            cb.free.push(chunk);
        }
        if let Some(mut chunk) = cb.free.pop() {
            chunk.start = cb.clock;
            chunk.frames = frames;
            chunk.selection = selection;
            chunk.latency = active;
            for ch in 0..self.channels {
                std::ptr::copy_nonoverlapping(input[ch], chunk.audio[ch].as_mut_ptr(), frames);
            }
            if let Err(chunk) = cb.send.push(chunk) {
                cb.free.push(chunk);
            }
        }
        let bypass = self.shared.bypass.load(Ordering::Acquire);
        let mut missed = false;
        for n in 0..frames {
            let now = cb.clock + n as u64;
            let target = now + (transport_delay + plugin_delay) as u64;
            let write = target as usize % size;
            let read = now as usize % size;
            for ch in 0..self.channels {
                cb.dry[ch][write] = *input[ch].add(n);
            }
            cb.dry_tags[write] = target;
            let wet = !bypass && cb.wet_tags[read] == now;
            let dry = cb.dry_tags[read] == now;
            for ch in 0..self.channels {
                *output[ch].add(n) = if wet {
                    cb.wet[ch][read]
                } else if dry {
                    cb.dry[ch][read]
                } else {
                    0.0
                };
            }
            missed |= !wet && !bypass;
            cb.wet_tags[read] = EMPTY;
            cb.dry_tags[read] = EMPTY;
        }
        cb.clock += frames as u64;
        if missed && self.shared.status.load(Ordering::Acquire) == 3 {
            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
        0
    }
}

impl Drop for NativePlugin {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Read only on a control thread. A file lets GUI-launched DAWs configure the
/// remote endpoint without inheriting shell environment variables.
#[derive(serde::Deserialize)]
#[serde(default)]
struct NativeConfiguration {
    server: String,
    transport_samples: u32,
}
impl Default for NativeConfiguration {
    fn default() -> Self {
        Self {
            server: "127.0.0.1:55056".into(),
            transport_samples: 256,
        }
    }
}
fn native_configuration() -> NativeConfiguration {
    let mut config = NativeConfiguration::default();
    let path = std::env::var_os("OPENGRIDDER_CLIENT_CONFIG")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| {
                std::path::PathBuf::from(home)
                    .join("Library/Application Support/OpenGridder/client.json")
            })
        });
    if let Some(path) = path {
        if path.metadata().map(|m| m.len() <= 65536).unwrap_or(false) {
            if let Ok(bytes) = std::fs::read(&path) {
                match serde_json::from_slice(&bytes) {
                    Ok(value) => config = value,
                    Err(error) => log::warn!("Invalid OpenGridder client configuration: {error}"),
                }
            }
        }
    }
    if let Ok(endpoint) = std::env::var("OPENGRIDDER_SERVER") {
        config.server = endpoint;
    }
    if let Ok(samples) = std::env::var("OPENGRIDDER_TRANSPORT_SAMPLES") {
        if let Ok(samples) = samples.parse() {
            config.transport_samples = samples;
        }
    }
    if !(32..=8192).contains(&config.transport_samples) {
        config.transport_samples = 256;
    }
    config
}
/// Preferred initial transport budget. Call on a control thread before creation.
#[unsafe(no_mangle)]
pub extern "C" fn og_native_default_latency() -> u32 {
    native_configuration().transport_samples
}

/// Create on a control thread. Supports mono/stereo FX and up to 8192 frames.
#[unsafe(no_mangle)]
pub extern "C" fn og_native_create(
    rate: f64,
    max_frames: u32,
    channels: u32,
    latency: u32,
) -> *mut NativePlugin {
    if !rate.is_finite()
        || rate < 8000.0
        || rate > 768000.0
        || !(1..=8192).contains(&max_frames)
        || !(1..=2).contains(&channels)
        || !(32..=8192).contains(&latency)
    {
        return std::ptr::null_mut();
    }
    let endpoint = native_configuration().server;
    let endpoint = endpoint
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().ok().map(|p| (host.to_owned(), p)))
        .unwrap_or_else(|| ("127.0.0.1".into(), 55056));
    catch_unwind(|| {
        Box::into_raw(Box::new(NativePlugin::new(
            rate,
            max_frames as usize,
            channels as usize,
            latency as usize,
            Some(endpoint),
        )))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// Handle must be live and not concurrently processed or destroyed. Buffer arrays
/// contain the configured number of valid planar channels, each at least frames long.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_process(
    handle: *mut NativePlugin,
    inputs: *const *const f32,
    outputs: *const *mut f32,
    frames: u32,
) -> i32 {
    let Some(plugin) = handle.as_ref() else {
        return -1;
    };
    catch_unwind(AssertUnwindSafe(|| {
        plugin.process(inputs, outputs, frames as usize)
    }))
    .unwrap_or(-2)
}

/// # Safety
/// Valid uniquely-owned handle; all callbacks/control users must have stopped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_destroy(handle: *mut NativePlugin) {
    if !handle.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(Box::from_raw(handle))));
    }
}

/// # Safety
/// Live handle, not concurrently destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_select(handle: *mut NativePlugin, index: u32) {
    if let Some(p) = handle.as_ref() {
        p.shared.selected.store(index.min(128), Ordering::Release);
    }
}
/// # Safety
/// Live handle, not concurrently destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_bypass(handle: *mut NativePlugin, bypass: bool) {
    if let Some(p) = handle.as_ref() {
        p.shared.bypass.store(bypass, Ordering::Release);
    }
}
/// # Safety
/// Live handle, not concurrently destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_latency(handle: *mut NativePlugin) -> u32 {
    handle
        .as_ref()
        .map(|p| total_latency(p.shared.requested_latency.load(Ordering::Acquire)))
        .unwrap_or(0)
}
/// Apply a latency only after the format's host has acknowledged it.
/// # Safety
/// Live handle, not concurrently destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_accept_latency(handle: *mut NativePlugin, total: u32) {
    if let Some(p) = handle.as_ref() {
        let requested = p.shared.requested_latency.load(Ordering::Acquire);
        if total == total_latency(requested) {
            p.shared.active_latency.store(requested, Ordering::Release);
        }
    }
}
/// # Safety
/// Live handle, not concurrently destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_status(handle: *mut NativePlugin) -> i32 {
    handle
        .as_ref()
        .map(|p| p.shared.status.load(Ordering::Acquire))
        .unwrap_or(-1)
}
/// Copy a discovered plugin name. Control-thread only, never call from audio.
/// # Safety
/// Live handle and writable buffer of capacity bytes. No concurrent destruction.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn og_native_name(
    handle: *mut NativePlugin,
    index: u32,
    buffer: *mut c_char,
    capacity: u32,
) {
    if buffer.is_null() || capacity == 0 {
        return;
    }
    *buffer = 0;
    if let Some(p) = handle.as_ref() {
        let names = p.shared.names.lock().unwrap_or_else(|e| e.into_inner());
        let name = if index == 0 {
            "None"
        } else {
            names
                .get(index as usize - 1)
                .map(String::as_str)
                .unwrap_or("Unavailable")
        };
        let count = name.len().min(capacity as usize - 1);
        std::ptr::copy_nonoverlapping(name.as_ptr(), buffer.cast(), count);
        *buffer.add(count) = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    thread_local! { static HEAP_OPS: Cell<Option<usize>> = const { Cell::new(None) }; }
    struct CountHeap;
    fn heap_op() {
        let _ = HEAP_OPS.try_with(|count| {
            if let Some(n) = count.get() {
                count.set(Some(n + 1));
            }
        });
    }
    unsafe impl GlobalAlloc for CountHeap {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            heap_op();
            System.alloc(layout)
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            heap_op();
            System.dealloc(ptr, layout);
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            heap_op();
            System.realloc(ptr, layout, size)
        }
    }
    #[global_allocator]
    static ALLOCATOR: CountHeap = CountHeap;

    #[test]
    fn callback_does_not_allocate_or_free_when_pool_is_exhausted() {
        let plugin = NativePlugin::new(48000.0, 64, 2, 32, None);
        let input = [0.25f32; 64];
        let mut left = [0.0f32; 64];
        let mut right = [0.0f32; 64];
        let ins = [input.as_ptr(), input.as_ptr()];
        let outs = [left.as_mut_ptr(), right.as_mut_ptr()];
        HEAP_OPS.with(|n| n.set(Some(0)));
        for _ in 0..1000 {
            assert_eq!(
                unsafe { plugin.process(ins.as_ptr(), outs.as_ptr(), 64) },
                0
            );
        }
        let count = HEAP_OPS.with(|n| n.replace(None)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn late_wet_audio_is_never_played_at_a_new_time() {
        let mut plugin = NativePlugin::new(48000.0, 64, 1, 32, None);
        let (mut returned, receive) = spsc::channel(POOL);
        let cb = plugin.audio.get_mut();
        cb.receive = receive;
        cb.clock = 128;
        let mut late = cb.free.pop().unwrap();
        late.start = 0;
        late.frames = 16;
        late.valid = true;
        late.latency = latency_pair(32, 0);
        late.audio[0].fill(0.9);
        assert!(returned.push(late).is_ok());
        let input = [0.0f32; 64];
        let mut output = [0.0f32; 64];
        assert_eq!(
            unsafe {
                plugin.process(
                    [input.as_ptr()].as_ptr(),
                    [output.as_mut_ptr()].as_ptr(),
                    64,
                )
            },
            0
        );
        assert!(output.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn latency_changes_wait_for_host_acknowledgement() {
        let mut plugin = NativePlugin::new(48000.0, 64, 1, 32, None);
        plugin.shared.request_plugin_latency(18);
        let p = &mut plugin as *mut NativePlugin;
        assert_eq!(unsafe { og_native_latency(p) }, 50);
        assert_eq!(
            plugin_part(plugin.shared.active_latency.load(Ordering::Acquire)),
            0
        );
        unsafe {
            og_native_accept_latency(p, 49);
        }
        assert_eq!(
            plugin_part(plugin.shared.active_latency.load(Ordering::Acquire)),
            0
        );
        unsafe {
            og_native_accept_latency(p, 50);
        }
        assert_eq!(
            plugin_part(plugin.shared.active_latency.load(Ordering::Acquire)),
            18
        );
    }

    #[test]
    fn larger_host_blocks_request_a_coherent_larger_budget() {
        let mut plugin = NativePlugin::new(48000.0, 1024, 1, 256, None);
        plugin.shared.request_plugin_latency(18);
        let input = [0.25f32; 1024];
        let mut output = [0.0f32; 1024];
        assert_eq!(
            unsafe {
                plugin.process(
                    [input.as_ptr()].as_ptr(),
                    [output.as_mut_ptr()].as_ptr(),
                    1024,
                )
            },
            0
        );
        let handle = &mut plugin as *mut NativePlugin;
        assert_eq!(unsafe { og_native_latency(handle) }, 1042);
        assert_eq!(
            plugin.shared.active_latency.load(Ordering::Acquire),
            latency_pair(256, 0)
        );
        unsafe {
            og_native_accept_latency(handle, 1042);
        }
        assert_eq!(
            plugin.shared.active_latency.load(Ordering::Acquire),
            latency_pair(1024, 18)
        );
    }

    #[test]
    fn worker_processes_remote_audio_without_blocking_callback() {
        use og_common::server_plugin::ServerPlugin;
        use og_hosting::vst3::default_plugin_parameters;
        use og_server::{Server, ServerConfig};
        let server = Server::new(ServerConfig::default());
        let mut desc = ServerPlugin::new(
            "Native Test Gain",
            "OpenGridder",
            "OpenGridder:Gain",
            "OpenGridder:Gain",
            "OpenGridder",
            "Fx",
            false,
            vec!["Stereo".into()],
        );
        desc.parameters = default_plugin_parameters();
        desc.parameters[0].current_value = 1.0;
        desc.parameters[0].default_value = 1.0;
        server.add_known_plugin(desc);
        let port = server.start("127.0.0.1", 0).unwrap();
        let mut plugin = NativePlugin::new(48000.0, 1024, 2, 256, Some(("127.0.0.1".into(), port)));
        plugin.shared.selected.store(1, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(3);
        while plugin.shared.status.load(Ordering::Acquire) != 3 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(plugin.shared.status.load(Ordering::Acquire), 3);
        let input = [0.25f32; 32];
        let mut left = [0.0f32; 32];
        let mut right = [0.0f32; 32];
        let mut heard_remote = false;
        for _ in 0..100 {
            let ins = [input.as_ptr(), input.as_ptr()];
            let outs = [left.as_mut_ptr(), right.as_mut_ptr()];
            HEAP_OPS.with(|n| n.set(Some(0)));
            let result = unsafe { plugin.process(ins.as_ptr(), outs.as_ptr(), 32) };
            let heap_ops = HEAP_OPS.with(|n| n.replace(None)).unwrap();
            assert_eq!(result, 0);
            assert_eq!(
                heap_ops, 0,
                "live worker returns must not allocate/free on the callback"
            );
            heard_remote |= left.iter().any(|&v| (v - 0.5).abs() < 1e-5);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(plugin.shared.processed.load(Ordering::Acquire) > 0);
        assert!(
            heard_remote,
            "expected 2x gain from the server, not delayed dry fallback"
        );
        let input = [0.25f32; 1024];
        let mut left = [0.0f32; 1024];
        let mut right = [0.0f32; 1024];
        let mut heard_large_block = false;
        for _ in 0..30 {
            let ins = [input.as_ptr(), input.as_ptr()];
            let outs = [left.as_mut_ptr(), right.as_mut_ptr()];
            assert_eq!(
                unsafe { plugin.process(ins.as_ptr(), outs.as_ptr(), 1024) },
                0
            );
            let handle = &mut plugin as *mut NativePlugin;
            unsafe {
                og_native_accept_latency(handle, og_native_latency(handle));
            }
            heard_large_block |= left.iter().any(|&v| (v - 0.5).abs() < 1e-5);
            thread::sleep(Duration::from_millis(4));
        }
        assert!(
            heard_large_block,
            "large blocks must not be permanently late/dry"
        );
        drop(plugin);
        server.stop();
    }
    #[test]
    fn disconnected_callback_has_exact_delay_across_variable_blocks() {
        let plugin = NativePlugin::new(48000.0, 64, 2, 32, None);
        let mut timeline = Vec::new();
        let mut position = 0;
        for frames in [1, 7, 16, 3, 64, 32, 64] {
            let input: Vec<f32> = (position..position + frames)
                .map(|n| if n == 0 { 1.0 } else { 0.0 })
                .collect();
            let mut left = vec![0.0; frames];
            let mut right = vec![0.0; frames];
            let ins = [input.as_ptr(), input.as_ptr()];
            let outs = [left.as_mut_ptr(), right.as_mut_ptr()];
            assert_eq!(
                unsafe { plugin.process(ins.as_ptr(), outs.as_ptr(), frames) },
                0
            );
            assert_eq!(left, right);
            timeline.extend(left);
            position += frames;
        }
        assert_eq!(timeline[32], 1.0);
        assert_eq!(timeline.iter().filter(|&&v| v != 0.0).count(), 1);
    }
    #[test]
    fn invalid_configuration_is_rejected() {
        assert!(og_native_create(f64::NAN, 64, 2, 32).is_null());
        assert!(og_native_create(48000.0, 0, 2, 32).is_null());
        assert!(og_native_create(48000.0, 64, 3, 32).is_null());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_callback_receives_real_audio_unit_processing() {
        use og_common::server_plugin::ServerPlugin;
        use og_server::{Server, ServerConfig};
        let server = Server::new(ServerConfig::default());
        server.add_known_plugin(ServerPlugin::new(
            "AULowpass",
            "Apple",
            "AudioUnit:aufx:lpas:appl",
            "AULowpass",
            "AudioUnit",
            "Filter",
            false,
            vec!["Stereo".into()],
        ));
        let port = server.start("127.0.0.1", 0).unwrap();
        let plugin = NativePlugin::new(48000., 256, 2, 256, Some(("127.0.0.1".into(), port)));
        plugin.shared.selected.store(1, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(5);
        while plugin.shared.status.load(Ordering::Acquire) != 3 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(plugin.shared.status.load(Ordering::Acquire), 3);
        let mut wet_energy = 0.;
        let mut input = [0.0f32; 64];
        let mut left = [0.0f32; 64];
        let mut right = [0.0f32; 64];
        for block in 0..150 {
            for (index, value) in input.iter_mut().enumerate() {
                *value = (std::f64::consts::TAU * 16000. * (block * 64 + index) as f64 / 48000.)
                    .sin() as f32
                    * 0.25;
            }
            HEAP_OPS.with(|n| n.set(Some(0)));
            let result = unsafe {
                plugin.process(
                    [input.as_ptr(), input.as_ptr()].as_ptr(),
                    [left.as_mut_ptr(), right.as_mut_ptr()].as_ptr(),
                    64,
                )
            };
            assert_eq!(HEAP_OPS.with(|n| n.replace(None)), Some(0));
            assert_eq!(result, 0);
            assert_eq!(left, right);
            if block >= 100 {
                wet_energy += left.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
            }
            thread::sleep(Duration::from_millis(2));
        }
        let rms = (wet_energy / (50 * 64) as f64).sqrt();
        assert!(
            rms > 0.00001 && rms < 0.1,
            "Native callback must receive real lowpass output, not dry fallback (RMS={rms})"
        );
        assert!(plugin.shared.processed.load(Ordering::Acquire) > 100);
        drop(plugin);
        server.stop();
    }
}
