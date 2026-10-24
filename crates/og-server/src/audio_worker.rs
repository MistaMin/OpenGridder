//! Dedicated real-time audio worker thread (`AudioWorker` in `Server/Source/AudioWorker.hpp` & `.cpp`).

use crate::processor_chain::ProcessorChain;
use og_common::message::AudioMessage;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

pub struct AudioWorker {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl AudioWorker {
    pub fn spawn(
        mut stream: TcpStream,
        chain: Arc<ProcessorChain>,
        running: Arc<AtomicBool>,
    ) -> Self {
        let run_flag = running.clone();
        let handle = thread::Builder::new()
            .name("OG-AudioWorker".to_string())
            .spawn(move || {
                let mut msg = AudioMessage::new();
                let mut buf_f32: Vec<Vec<f32>> = Vec::new();
                let mut buf_f64: Vec<Vec<f64>> = Vec::new();
                let mut midi = Vec::new();
                let mut pos_info = og_common::message::PositionInfo::default();

                while run_flag.load(Ordering::Relaxed) {
                    match msg.read_from_client(
                        &mut stream,
                        &mut buf_f32,
                        &mut buf_f64,
                        &mut midi,
                        &mut pos_info,
                    ) {
                        Ok(()) => {
                            if msg.is_double() {
                                let lat =
                                    chain.process_block_f64(&mut buf_f64, &mut midi, &pos_info);
                                let ch_out = buf_f64.len();
                                if msg
                                    .send_to_client_f64(&mut stream, &buf_f64, &midi, lat, ch_out)
                                    .is_err()
                                {
                                    break;
                                }
                            } else {
                                let lat =
                                    chain.process_block_f32(&mut buf_f32, &mut midi, &pos_info);
                                let ch_out = buf_f32.len();
                                if msg
                                    .send_to_client_f32(&mut stream, &buf_f32, &midi, lat, ch_out)
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .expect("failed to spawn AudioWorker thread");

        Self {
            running,
            handle: Some(handle),
        }
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
