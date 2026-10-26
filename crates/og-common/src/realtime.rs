//! Scheduling hints for audio-critical threads.
//!
//! Streaming threads that run at default priority are delayed by unrelated work
//! and miss their deadline, which is heard as crackles.

/// Ask the scheduler to treat the calling thread as interactive, latency-critical work.
pub fn prioritize_audio_thread() {
    #[cfg(target_os = "macos")]
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0);
    }
}
