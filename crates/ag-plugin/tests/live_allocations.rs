//! Count Rust allocations/frees on the callback thread, including packet recycling.
use ag_plugin::{
    live::{LiveAudio, LiveCommand},
    AudioGridder,
};
use ag_protocol::{HandshakeRequest, PositionInfo};
use openplugin::prelude::Plugin;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant},
};
thread_local! {static COUNT:Cell<bool>=const{Cell::new(false)};}
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
struct Alloc;
unsafe impl GlobalAlloc for Alloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if COUNT.try_with(Cell::get).unwrap_or(false) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if COUNT.try_with(Cell::get).unwrap_or(false) {
            FREES.fetch_add(1, Ordering::Relaxed);
        }
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if COUNT.try_with(Cell::get).unwrap_or(false) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            FREES.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static A: Alloc = Alloc;
#[test]
fn live_callback_recycles_packets_without_rust_heap_activity() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || ag_server::serve_worker(listener));
    let (mut live, control) = LiveAudio::new();
    assert!(control.connect(
        &format!("127.0.0.1:{port}"),
        HandshakeRequest {
            samples_per_block: 64,
            ..HandshakeRequest::default()
        }
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !control.connected() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert!(control.connected());
    assert!(control.send(LiveCommand::Add {
        id: ag_plugin::BUILTIN_GAIN_ID.into(),
        settings: "6".into()
    }));
    let mut plugin = AudioGridder::new();
    plugin.set_param("latency_profile", 3.);
    plugin.prepare(48000., 64, 2);
    let mut io = vec![vec![0.25; 64]; 2];
    for _ in 0..100 {
        io.iter_mut().for_each(|ch| ch.fill(0.25));
        COUNT.with(|c| c.set(true));
        live.process(&mut plugin, &mut io, 64, PositionInfo::default(), false);
        COUNT.with(|c| c.set(false));
        assert!(io.iter().flatten().all(|x| x.is_finite()));
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        ALLOCS.load(Ordering::Relaxed),
        0,
        "callback Rust allocations"
    );
    assert_eq!(FREES.load(Ordering::Relaxed), 0, "callback Rust frees");
    assert_eq!(control.with_info(|i| i.slots.len()), 1);
    drop(control);
    drop(live);
    assert!(server.join().unwrap().is_ok());
}
