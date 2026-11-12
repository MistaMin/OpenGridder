//! Real-time scheduling for audio threads (AudioGridder starts its server audio worker and
//! client audio streamer with JUCE's `startRealtimeThread`, i.e. the Mach time-constraint
//! policy). Without it, a heavy plugin UI, a plugin load or the scanner preempts the audio
//! thread and blocks arrive late.

/// Give the calling thread a time-constraint policy for audio blocks of `period_ms`.
/// Returns false when the system refused (the thread keeps its normal priority).
#[cfg(target_os = "macos")]
pub fn make_current_thread_realtime(period_ms: f64) -> bool {
    #[repr(C)]
    struct TimebaseInfo {
        numer: u32,
        denom: u32,
    }
    #[repr(C)]
    struct TimeConstraintPolicy {
        period: u32,
        computation: u32,
        constraint: u32,
        preemptible: i32,
    }
    extern "C" {
        fn mach_thread_self() -> u32;
        fn mach_task_self() -> u32;
        fn mach_port_deallocate(task: u32, name: u32) -> i32;
        fn mach_timebase_info(info: *mut TimebaseInfo) -> i32;
        fn thread_policy_set(thread: u32, flavor: u32, policy: *const TimeConstraintPolicy, count: u32) -> i32;
    }
    const THREAD_TIME_CONSTRAINT_POLICY: u32 = 2;
    const THREAD_TIME_CONSTRAINT_POLICY_COUNT: u32 = 4;

    let period_ms = if period_ms.is_finite() { period_ms.clamp(1.0, 100.0) } else { 10.0 };
    let mut timebase = TimebaseInfo { numer: 0, denom: 0 };
    if unsafe { mach_timebase_info(&mut timebase) } != 0 || timebase.numer == 0 {
        return false;
    }
    let to_abs = |ms: f64| -> u32 { (ms * 1_000_000.0 * f64::from(timebase.denom) / f64::from(timebase.numer)) as u32 };
    let policy = TimeConstraintPolicy {
        period: to_abs(period_ms),
        // Most of a block is spent waiting on the network or the plugins; ask for half of
        // it as computation and the whole period as the deadline.
        computation: to_abs((period_ms * 0.5).max(0.5)),
        constraint: to_abs(period_ms),
        preemptible: 1,
    };
    unsafe {
        let thread = mach_thread_self();
        let result = thread_policy_set(thread, THREAD_TIME_CONSTRAINT_POLICY, &policy, THREAD_TIME_CONSTRAINT_POLICY_COUNT);
        mach_port_deallocate(mach_task_self(), thread);
        result == 0
    }
}

#[cfg(not(target_os = "macos"))]
pub fn make_current_thread_realtime(_period_ms: f64) -> bool {
    false
}

/// Block duration in milliseconds, AudioGridder's `agRealtimeOptions` period
/// (one block, minus a millisecond, at least 1 ms).
pub fn block_period_ms(samples_per_block: i32, sample_rate: f64) -> f64 {
    if samples_per_block <= 0 || sample_rate <= 0.0 {
        return 10.0;
    }
    (f64::from(samples_per_block) / sample_rate * 1000.0 - 1.0).round().max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_thread_gets_the_time_constraint_policy() {
        assert_eq!(block_period_ms(512, 48_000.0), 10.0);
        assert_eq!(block_period_ms(32, 48_000.0), 1.0);
        let granted = std::thread::spawn(|| make_current_thread_realtime(block_period_ms(256, 48_000.0))).join().unwrap();
        assert!(granted, "thread_policy_set refused the time-constraint policy");
    }
}
