//! One native plugin chain per child process. A plugin fault closes only that
//! client's sockets; the parent retains the catalog and accepts new sessions.
#![cfg(unix)]
use crate::Worker;
use og_common::{message::HandshakeRequest, server_plugin::ServerPlugin};
use og_hosting::PluginFormatManager;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    net::TcpStream,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    path::Path,
    process::{Child, Command, Stdio},
    sync::Arc,
};

#[derive(Serialize, Deserialize)]
struct Payload {
    handshake: Vec<u8>,
    plugins: Vec<ServerPlugin>,
    diff_detection: bool,
    screen_local_mode: bool,
    screen_capturing_off: bool,
    plugin_windows_on_top: bool,
}

pub fn spawn(
    executable: &Path,
    streams: [TcpStream; 4],
    handshake: HandshakeRequest,
    plugins: Vec<ServerPlugin>,
    diff_detection: bool,
    screen_local_mode: bool,
    screen_capturing_off: bool,
    plugin_windows_on_top: bool,
) -> Result<Child, String> {
    let payload = serde_json::to_vec(&Payload {
        handshake: handshake.to_bytes().to_vec(),
        plugins,
        diff_detection,
        screen_local_mode,
        screen_capturing_off,
        plugin_windows_on_top,
    })
    .map_err(|e| e.to_string())?;
    if payload.len() > 8 * 1024 * 1024 {
        return Err("Worker catalog exceeds size limit".into());
    }
    // Duplicate above the target range so dup2 cannot clobber another source.
    // CLOEXEC on these temporary parent descriptors prevents unrelated child leaks.
    let mut descriptors = Vec::new();
    for stream in &streams {
        let fd = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 100) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        descriptors.push(unsafe { OwnedFd::from_raw_fd(fd) });
    }
    let source: Vec<i32> = descriptors.iter().map(AsRawFd::as_raw_fd).collect();
    let mut command = Command::new(executable);
    command
        .arg("--chain-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    // Only async-signal-safe syscalls execute between fork and exec.
    unsafe {
        command.pre_exec(move || {
            for (index, fd) in source.iter().enumerate() {
                if libc::dup2(*fd, 3 + index as i32) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Could not spawn chain worker: {e}"))?;
    let write_result = child
        .stdin
        .take()
        .ok_or("Missing worker input")?
        .write_all(&payload);
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.to_string());
    }
    Ok(child)
}

/// Internal standalone entry point. Descriptors 3–6 are inherited only by this worker.
pub fn run_worker() -> Result<(), String> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Worker input exceeds size limit".into());
    }
    let payload: Payload = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let handshake = HandshakeRequest::from_bytes(&payload.handshake).map_err(|e| e.to_string())?;
    // Mark inherited sockets CLOEXEC again before plugins can spawn other programs.
    for fd in 3..=6 {
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err("Invalid inherited worker socket".into());
        }
    }
    let [master, cmd_in, audio, screen] =
        [3, 4, 5, 6].map(|fd| unsafe { TcpStream::from_raw_fd(fd) });
    let worker = Worker::new(
        handshake,
        Arc::new(PluginFormatManager::new_all_enabled()),
        Arc::new(Mutex::new(payload.plugins)),
        Arc::new(Mutex::new(Vec::new())),
        payload.screen_local_mode,
        payload.screen_capturing_off,
        payload.plugin_windows_on_top,
    );

    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn og_run_main_loop();
            fn og_stop_main_loop();
        }
        let diff_detect = payload.diff_detection;
        let worker_thread = std::thread::Builder::new()
            .name("OG-ChainWorker".into())
            .spawn(move || {
                worker.run(master, cmd_in, audio, screen, diff_detect);
                unsafe { og_stop_main_loop(); }
            })
            .map_err(|e| e.to_string())?;

        unsafe { og_run_main_loop(); }
        let _ = worker_thread.join();
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        worker.run(master, cmd_in, audio, screen, payload.diff_detection);
        Ok(())
    }
}
