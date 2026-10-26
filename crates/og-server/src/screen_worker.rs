//! Dedicated remote UI screen capture worker thread (`ScreenWorker` in `Server/Source/ScreenWorker.hpp` & `.cpp`).

use crate::processor_chain::ProcessorChain;
use og_screen::ScreenEncoder;
use parking_lot::Mutex;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct ScreenWorker {
    running: Arc<AtomicBool>,
    active_editor_idx: Arc<AtomicI32>,
    handle: Option<JoinHandle<()>>,
}

impl ScreenWorker {
    pub fn spawn(
        stream: TcpStream,
        chain: Arc<ProcessorChain>,
        running: Arc<AtomicBool>,
        active_editor_idx: Arc<AtomicI32>,
        diff_detection: bool,
        capturing_off: bool,
    ) -> Self {
        let run_flag = running.clone();
        let editor_idx = active_editor_idx.clone();
        let stream_arc = Arc::new(Mutex::new(stream));

        let handle = thread::Builder::new()
            .name("OG-ScreenWorker".to_string())
            .spawn(move || {
                let mut encoder = ScreenEncoder::new();
                encoder.enable_diff_detection = diff_detection;

                while run_flag.load(Ordering::Relaxed) {
                    if capturing_off {
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    let idx = editor_idx.load(Ordering::Relaxed);
                    if idx >= 0 {
                        if let Some(proc_arc) = chain.get_processor(idx as usize) {
                            let rendered = proc_arc.lock().render_editor_rgba();
                            if let Some((w, h, rgba)) = rendered {
                                if let Some(msg) = encoder.encode_frame(w, h, 1.0, &rgba) {
                                    let mut sock = stream_arc.lock();
                                    if msg.send(&mut *sock).is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                    } else {
                        encoder.reset();
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            })
            .expect("failed to spawn ScreenWorker thread");

        Self {
            running,
            active_editor_idx,
            handle: Some(handle),
        }
    }

    pub fn set_active_editor(&self, idx: i32) {
        self.active_editor_idx.store(idx, Ordering::Relaxed);
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
