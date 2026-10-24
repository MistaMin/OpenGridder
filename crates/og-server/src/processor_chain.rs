//! Ordered processor chain (`ProcessorChain` in `Server/Source/ProcessorChain.hpp` & `.cpp`).

use crate::processor::Processor;
use og_common::message::{MidiEvent, PositionInfo};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

pub struct ProcessorChain {
    processors: Mutex<Vec<Arc<Mutex<Processor>>>>,
    sample_rate: Mutex<f64>,
    max_block_size: Mutex<usize>,
    latency_samples: AtomicI32,
}

impl Default for ProcessorChain {
    fn default() -> Self {
        Self::new(48000.0, 512)
    }
}

impl ProcessorChain {
    pub fn new(sample_rate: f64, max_block_size: usize) -> Self {
        Self {
            processors: Mutex::new(Vec::new()),
            sample_rate: Mutex::new(sample_rate),
            max_block_size: Mutex::new(max_block_size),
            latency_samples: AtomicI32::new(0),
        }
    }

    pub fn prepare_to_play(&self, sample_rate: f64, max_block_size: usize) {
        *self.sample_rate.lock() = sample_rate;
        *self.max_block_size.lock() = max_block_size;
        for proc in self.processors.lock().iter() {
            proc.lock().prepare_to_play(sample_rate, max_block_size);
        }
        self.update_latency();
    }

    pub fn release_resources(&self) {
        for proc in self.processors.lock().iter() {
            proc.lock().release_resources();
        }
    }

    pub fn len(&self) -> usize {
        self.processors.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.processors.lock().is_empty()
    }

    pub fn clear(&self) {
        self.processors.lock().clear();
        self.latency_samples.store(0, Ordering::Relaxed);
    }

    pub fn add_processor(&self, proc: Arc<Mutex<Processor>>) -> usize {
        let mut guard = self.processors.lock();
        guard.push(proc);
        let idx = guard.len() - 1;
        drop(guard);
        self.update_latency();
        idx
    }

    pub fn del_processor(&self, idx: usize) -> bool {
        let mut guard = self.processors.lock();
        if idx < guard.len() {
            guard.remove(idx);
            drop(guard);
            self.update_latency();
            true
        } else {
            false
        }
    }

    pub fn exchange_processors(&self, idx_a: usize, idx_b: usize) -> bool {
        let mut guard = self.processors.lock();
        if idx_a < guard.len() && idx_b < guard.len() {
            guard.swap(idx_a, idx_b);
            true
        } else {
            false
        }
    }

    pub fn get_processor(&self, idx: usize) -> Option<Arc<Mutex<Processor>>> {
        self.processors.lock().get(idx).cloned()
    }

    pub fn update_latency(&self) -> i32 {
        let total: i32 = self
            .processors
            .lock()
            .iter()
            .map(|p| p.lock().latency_samples())
            .sum();
        self.latency_samples.store(total, Ordering::Relaxed);
        total
    }

    pub fn latency_samples(&self) -> i32 {
        self.latency_samples.load(Ordering::Relaxed)
    }

    pub fn process_block_f32(
        &self,
        audio: &mut [Vec<f32>],
        midi: &mut Vec<MidiEvent>,
        pos: &PositionInfo,
    ) -> i32 {
        let procs: Vec<Arc<Mutex<Processor>>> = self.processors.lock().clone();
        let mut total_lat = 0;
        for proc_arc in procs {
            let mut proc = proc_arc.lock();
            proc.process_block_f32(audio, midi, pos);
            total_lat += proc.latency_samples();
        }
        self.latency_samples.store(total_lat, Ordering::Relaxed);
        total_lat
    }

    pub fn process_block_f64(
        &self,
        audio: &mut [Vec<f64>],
        midi: &mut Vec<MidiEvent>,
        pos: &PositionInfo,
    ) -> i32 {
        let procs: Vec<Arc<Mutex<Processor>>> = self.processors.lock().clone();
        let mut total_lat = 0;
        for proc_arc in procs {
            let mut proc = proc_arc.lock();
            proc.process_block_f64(audio, midi, pos);
            total_lat += proc.latency_samples();
        }
        self.latency_samples.store(total_lat, Ordering::Relaxed);
        total_lat
    }
}
