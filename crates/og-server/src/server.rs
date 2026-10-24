//! Main AudioGridder Server coordinator (`Server` in `Server/Source/Server.hpp` & `Server.cpp`).
//!
//! Includes:
//! - Configuration load/save (`~/.audiogridder/audiogridderserver.cfg`) with the `marcosfork`
//!   PR #1354 `save_config()` persistence fix
//! - AU, VST3, and LV2 plugin scanning and caching
//! - Handshake negotiation (`AG_PROTOCOL_VERSION = 13`) and multi-client `Worker` spawning

use crate::sandbox::SandboxMode;
use crate::worker::Worker;
use og_common::defaults::{get_config_file_name, ConfigFileKind, DEFAULT_SERVER_PORT};
use og_common::message::{
    read_value, write_value, HandshakeRequest, HandshakeResponse, AG_PROTOCOL_VERSION,
    HANDSHAKE_FLAGS_SANDBOX_ENABLED,
};
use og_common::server_plugin::ServerPlugin;
use og_common::utils::{json_load_from_file, json_save_to_file};
use og_hosting::PluginFormatManager;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ServerConfig {
    #[serde(rename = "ID", default)]
    pub id: i32,
    #[serde(default = "default_server_name")]
    pub name: String,
    #[serde(rename = "AU", default = "default_true")]
    pub au: bool,
    #[serde(rename = "VST3", default = "default_true")]
    pub vst3: bool,
    #[serde(rename = "LV2", default = "default_true")]
    pub lv2: bool,
    #[serde(rename = "VST3Folders", default)]
    pub vst3_folders: Vec<String>,
    #[serde(rename = "LV2Folders", default)]
    pub lv2_folders: Vec<String>,
    #[serde(default)]
    pub sandbox_mode: i32,
    #[serde(default = "default_true")]
    pub screen_diff_detection: bool,
    #[serde(default = "default_true")]
    pub scan_for_plugins: bool,
    #[serde(default)]
    pub crash_reporting: bool,
}

fn default_server_name() -> String {
    "OpenGridder Server".to_string()
}

fn default_true() -> bool {
    true
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            id: 0,
            name: default_server_name(),
            au: cfg!(target_os = "macos"),
            vst3: true,
            lv2: true,
            vst3_folders: Vec::new(),
            lv2_folders: Vec::new(),
            sandbox_mode: SandboxMode::None as i32,
            screen_diff_detection: true,
            scan_for_plugins: true,
            crash_reporting: false,
        }
    }
}

pub struct Server {
    config: Mutex<ServerConfig>,
    config_path: PathBuf,
    format_mgr: Arc<PluginFormatManager>,
    known_plugins: Arc<Mutex<Vec<ServerPlugin>>>,
    recents: Arc<Mutex<Vec<ServerPlugin>>>,
    running: Arc<AtomicBool>,
    bound_port: Mutex<u16>,
    accept_handle: Mutex<Option<JoinHandle<()>>>,
}

impl Default for Server {
    fn default() -> Self {
        Self::new(ServerConfig::default())
    }
}

impl Server {
    pub fn new(config: ServerConfig) -> Self {
        let cfg_path = get_config_file_name(ConfigFileKind::ConfigServer, &[]);
        let format_mgr = Arc::new(PluginFormatManager {
            enable_au: config.au,
            enable_vst3: config.vst3,
            enable_lv2: config.lv2,
            vst3_folders: config.vst3_folders.clone(),
            lv2_folders: config.lv2_folders.clone(),
        });
        Self {
            config: Mutex::new(config),
            config_path: cfg_path,
            format_mgr,
            known_plugins: Arc::new(Mutex::new(Vec::new())),
            recents: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
            bound_port: Mutex::new(DEFAULT_SERVER_PORT),
            accept_handle: Mutex::new(None),
        }
    }

    pub fn config(&self) -> ServerConfig {
        self.config.lock().clone()
    }

    /// Update server settings and immediately persist to disk (`save_config()`),
    /// matching the `marcosfork` PR #1354 fix in `ServerSettingsWindow.hpp`.
    pub fn apply_and_save_config(&self, new_cfg: ServerConfig) -> std::io::Result<()> {
        *self.config.lock() = new_cfg;
        self.save_config()
    }

    pub fn load_config(&self) -> Option<ServerConfig> {
        let loaded: Option<ServerConfig> = json_load_from_file(&self.config_path);
        if let Some(ref cfg) = loaded {
            *self.config.lock() = cfg.clone();
        }
        loaded
    }

    pub fn save_config(&self) -> std::io::Result<()> {
        let cfg = self.config.lock().clone();
        json_save_to_file(&self.config_path, &cfg)
    }

    pub fn set_config_path(&mut self, path: PathBuf) {
        self.config_path = path;
    }

    pub fn add_known_plugin(&self, plugin: ServerPlugin) {
        self.known_plugins.lock().push(plugin);
    }

    pub fn known_plugins(&self) -> Vec<ServerPlugin> {
        self.known_plugins.lock().clone()
    }

    pub fn scan_plugins(&self) -> usize {
        let discovered = self.format_mgr.scan_all();
        let mut guard = self.known_plugins.lock();
        for p in discovered {
            if !guard.iter().any(|existing| existing.id == p.id) {
                guard.push(p);
            }
        }
        guard.len()
    }

    pub fn bound_port(&self) -> u16 {
        *self.bound_port.lock()
    }

    /// Start listening on `host:port` (pass `port = 0` for OS-assigned ephemeral port in tests).
    pub fn start(&self, host: &str, port: u16) -> Result<u16, String> {
        self.stop();
        let listener = TcpListener::bind((host, port))
            .map_err(|e| format!("failed to bind {}:{}: {}", host, port, e))?;
        let actual_port = listener
            .local_addr()
            .map_err(|e| e.to_string())?
            .port();
        let _ = listener.set_nonblocking(true);

        *self.bound_port.lock() = actual_port;
        self.running.store(true, Ordering::SeqCst);

        let run_flag = self.running.clone();
        let fmt_mgr = self.format_mgr.clone();
        let known = self.known_plugins.clone();
        let recents = self.recents.clone();
        let cfg = self.config.lock().clone();

        let handle = thread::Builder::new()
            .name("OG-ServerAccept".to_string())
            .spawn(move || {
                while run_flag.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((master_stream, _peer)) => {
                            let _ = master_stream.set_nonblocking(false);
                            let _ = master_stream.set_nodelay(true);
                            Self::handle_new_client(
                                &listener,
                                master_stream,
                                fmt_mgr.clone(),
                                known.clone(),
                                recents.clone(),
                                cfg.clone(),
                            );
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        *self.accept_handle.lock() = Some(handle);
        Ok(actual_port)
    }

    fn handle_new_client(
        listener: &TcpListener,
        mut master_stream: TcpStream,
        fmt_mgr: Arc<PluginFormatManager>,
        known: Arc<Mutex<Vec<ServerPlugin>>>,
        recents: Arc<Mutex<Vec<ServerPlugin>>>,
        cfg: ServerConfig,
    ) {
        let Ok(req_bytes) = read_value::<_, { HandshakeRequest::SIZE }>(&mut master_stream) else {
            return;
        };
        let Ok(req) = HandshakeRequest::from_bytes(&req_bytes) else {
            return;
        };

        let local_port = listener.local_addr().map(|a| a.port()).unwrap_or(DEFAULT_SERVER_PORT);
        let mut resp = HandshakeResponse {
            version: AG_PROTOCOL_VERSION,
            flags: 0,
            port: local_port as i32,
            unused: [0; 6],
        };
        if SandboxMode::from_i32(cfg.sandbox_mode) != SandboxMode::None {
            resp.set_flag(HANDSHAKE_FLAGS_SANDBOX_ENABLED);
        }

        if write_value(&mut master_stream, &resp.to_bytes()).is_err() {
            return;
        }

        // Accept the 3 remaining worker connections (`cmdIn`, `audio`, `screen`)
        let Some(cmd_in_stream) = Self::accept_blocking(listener) else {
            return;
        };
        let Some(audio_stream) = Self::accept_blocking(listener) else {
            return;
        };
        let Some(screen_stream) = Self::accept_blocking(listener) else {
            return;
        };

        thread::Builder::new()
            .name("OG-Worker".to_string())
            .spawn(move || {
                let worker = Worker::new(req, fmt_mgr, known, recents);
                worker.run(
                    master_stream,
                    cmd_in_stream,
                    audio_stream,
                    screen_stream,
                    cfg.screen_diff_detection,
                );
            })
            .ok();
    }

    fn accept_blocking(listener: &TcpListener) -> Option<TcpStream> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_nodelay(true);
                    return Some(stream);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return None,
            }
        }
        None
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.accept_handle.lock().take() {
            let _ = h.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}
