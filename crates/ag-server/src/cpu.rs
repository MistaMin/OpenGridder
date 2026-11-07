//! Host CPU percent, sampled the way AudioGridder's `CPUInfo` does.
//!
//! One background thread reads `host_processor_info` (`PROCESSOR_CPU_LOAD_INFO`),
//! sleeps one second, reads again, and stores the five-sample average of
//! busy ticks over total ticks, times 100. The command reply is that cached
//! percent. A failed mach call leaves the previous value, which starts at 0.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Once;
use std::thread;
use std::time::Duration;

static USAGE_BITS: AtomicU32 = AtomicU32::new(0.0f32.to_bits());

pub fn start() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        thread::spawn(sample_loop);
    });
}

pub fn usage() -> f32 {
    f32::from_bits(USAGE_BITS.load(Ordering::Relaxed))
}

fn sample_loop() {
    let mut window = [0.0f32; 5];
    let mut slot = 0usize;
    loop {
        let Some(start) = load_ticks() else {
            thread::sleep(Duration::from_secs(1));
            continue;
        };
        thread::sleep(Duration::from_secs(1));
        let Some(end) = load_ticks() else {
            continue;
        };
        let Some(sample) = percent_busy(&start, &end) else {
            continue;
        };
        window[slot % window.len()] = sample;
        slot += 1;
        let average = window.iter().sum::<f32>() / window.len() as f32;
        if average.is_finite() {
            USAGE_BITS.store(average.clamp(0.0, 100.0).to_bits(), Ordering::Relaxed);
        }
    }
}

fn percent_busy(start: &[[u32; 4]], end: &[[u32; 4]]) -> Option<f32> {
    if start.len() != end.len() || start.is_empty() {
        return None;
    }
    let mut busy: u64 = 0;
    let mut idle: u64 = 0;
    for (before, after) in start.iter().zip(end.iter()) {
        busy += tick_delta(after[0], before[0]);
        busy += tick_delta(after[1], before[1]);
        busy += tick_delta(after[3], before[3]);
        idle += tick_delta(after[2], before[2]);
    }
    let total = busy + idle;
    if total == 0 {
        return None;
    }
    let usage = (busy as f64) / (total as f64) * 100.0;
    usage.is_finite().then_some(usage as f32)
}

fn tick_delta(end: u32, start: u32) -> u64 {
    u64::from(end.wrapping_sub(start))
}

#[cfg(target_os = "macos")]
fn load_ticks() -> Option<Vec<[u32; 4]>> {
    const PROCESSOR_CPU_LOAD_INFO: i32 = 2;
    const KERN_SUCCESS: i32 = 0;
    unsafe {
        let mut processors: u32 = 0;
        let mut info: *mut i32 = std::ptr::null_mut();
        let mut info_count: u32 = 0;
        let rc = host_processor_info(mach_host_self(), PROCESSOR_CPU_LOAD_INFO, &mut processors, &mut info, &mut info_count);
        if rc != KERN_SUCCESS || info.is_null() || processors == 0 {
            deallocate(info, info_count);
            return None;
        }
        let words = info as *const u32;
        let mut ticks = Vec::with_capacity(processors as usize);
        for index in 0..processors as usize {
            let mut row = [0u32; 4];
            for state in 0..4 {
                row[state] = *words.add(index * 4 + state);
            }
            ticks.push(row);
        }
        deallocate(info, info_count);
        Some(ticks)
    }
}

#[cfg(target_os = "macos")]
unsafe fn deallocate(info: *mut i32, info_count: u32) {
    if info.is_null() || info_count == 0 {
        return;
    }
    let size = (info_count as usize).saturating_mul(std::mem::size_of::<i32>());
    let _ = vm_deallocate(mach_task_self(), info as usize, size);
}

#[cfg(not(target_os = "macos"))]
fn load_ticks() -> Option<Vec<[u32; 4]>> {
    None
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn mach_host_self() -> u32;
    fn mach_task_self() -> u32;
    fn host_processor_info(host: u32, flavor: i32, out_processor_count: *mut u32, out_processor_info: *mut *mut i32, out_processor_info_count: *mut u32) -> i32;
    fn vm_deallocate(target: u32, address: usize, size: usize) -> i32;
}
