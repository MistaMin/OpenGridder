//! `og-tray`: Complete Rust rewrite of `AudioGridderPluginTray` (`PluginTray/Source/App.cpp`
//! and `PluginTray/Source/PluginMonitor.hpp`/`.cpp`).
//!
//! Coordinates active DAW plugin instances (`PluginStatus`), synchronizes recent plugins and
//! server lists across instances, and renders the `PluginMonitor` window using `GoodLookinUI`.

use og_common::defaults::DEFAULT_TRAY_PORT;
use og_common::message::{PluginTrayMessage, PluginTrayMessageType};
use og_goodlookinui::{draw_panel, Canvas, Color, Rect};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Status of a connected DAW plugin instance (`Monitor::Status` in `PluginTray/Source/App.hpp`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInstanceStatus {
    pub channel_name: String,
    pub channel_colour: u32,
    pub loaded_plugins: String,
    pub perf_95th_ms: f64,
    pub blocks: i32,
    pub rq_avg: usize,
    pub rq_95th: usize,
    pub wq_avg: usize,
    pub wq_95th: usize,
    pub connected: bool,
    #[serde(skip, default = "Instant::now")]
    pub last_updated: Instant,
}

impl Default for PluginInstanceStatus {
    fn default() -> Self {
        Self {
            channel_name: "Track 1".to_string(),
            channel_colour: 0xff3b5998,
            loaded_plugins: String::new(),
            perf_95th_ms: 0.0,
            blocks: 1,
            rq_avg: 0,
            rq_95th: 0,
            wq_avg: 0,
            wq_95th: 0,
            connected: false,
            last_updated: Instant::now(),
        }
    }
}

/// PluginTray coordinator server (`App` & `PluginMonitor` in `PluginTray/Source/*`).
pub struct PluginTrayServer {
    statuses: Arc<Mutex<HashMap<String, PluginInstanceStatus>>>,
    recents: Arc<Mutex<HashMap<String, String>>>,
    running: Arc<AtomicBool>,
    bound_port: Mutex<u16>,
    accept_handle: Mutex<Option<JoinHandle<()>>>,
}

impl Default for PluginTrayServer {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginTrayServer {
    pub fn new() -> Self {
        Self {
            statuses: Arc::new(Mutex::new(HashMap::new())),
            recents: Arc::new(Mutex::new(HashMap::new())),
            running: Arc::new(AtomicBool::new(false)),
            bound_port: Mutex::new(DEFAULT_TRAY_PORT),
            accept_handle: Mutex::new(None),
        }
    }

    pub fn statuses(&self) -> Vec<PluginInstanceStatus> {
        self.statuses.lock().values().cloned().collect()
    }

    pub fn start(&self, host: &str, port: u16) -> Result<u16, String> {
        self.stop();
        let listener = TcpListener::bind((host, port))
            .map_err(|e| format!("failed to bind tray {}:{}: {}", host, port, e))?;
        let actual_port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let _ = listener.set_nonblocking(true);
        *self.bound_port.lock() = actual_port;
        self.running.store(true, Ordering::SeqCst);

        let run_flag = self.running.clone();
        let statuses = self.statuses.clone();
        let recents = self.recents.clone();

        let handle = thread::Builder::new()
            .name("OG-TrayServer".to_string())
            .spawn(move || {
                while run_flag.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_nodelay(true);
                            let peer_key = peer.to_string();
                            let st = statuses.clone();
                            let rc = recents.clone();
                            let rf = run_flag.clone();
                            thread::spawn(move || {
                                Self::handle_connection(stream, peer_key, st, rc, rf);
                            });
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        *self.accept_handle.lock() = Some(handle);
        Ok(actual_port)
    }

    fn handle_connection(
        mut stream: TcpStream,
        peer_key: String,
        statuses: Arc<Mutex<HashMap<String, PluginInstanceStatus>>>,
        recents: Arc<Mutex<HashMap<String, String>>>,
        running: Arc<AtomicBool>,
    ) {
        while running.load(Ordering::Relaxed) {
            let Ok(msg) = PluginTrayMessage::read_from(&mut stream) else {
                break;
            };
            match msg.msg_type {
                PluginTrayMessageType::Status => {
                    let st = PluginInstanceStatus {
                        channel_name: msg
                            .data
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Track")
                            .to_string(),
                        channel_colour: msg
                            .data
                            .get("colour")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0xff3b5998) as u32,
                        loaded_plugins: msg
                            .data
                            .get("loadedPlugins")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        perf_95th_ms: msg
                            .data
                            .get("perf")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                        blocks: msg.data.get("blocks").and_then(|v| v.as_i64()).unwrap_or(1)
                            as i32,
                        rq_avg: msg
                            .data
                            .get("rq_avg")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize,
                        rq_95th: msg
                            .data
                            .get("rq_95th")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize,
                        wq_avg: msg
                            .data
                            .get("wq_avg")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize,
                        wq_95th: msg
                            .data
                            .get("wq_95th")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize,
                        connected: msg
                            .data
                            .get("ok")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        last_updated: Instant::now(),
                    };
                    statuses.lock().insert(peer_key.clone(), st);
                }
                PluginTrayMessageType::UpdateRecents => {
                    if let (Some(srv), Some(rec)) = (
                        msg.data.get("server").and_then(|v| v.as_str()),
                        msg.data.get("recents").and_then(|v| v.as_str()),
                    ) {
                        recents.lock().insert(srv.to_string(), rec.to_string());
                    }
                }
                PluginTrayMessageType::GetRecents => {
                    let srv = msg
                        .data
                        .get("server")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let rec = recents.lock().get(srv).cloned().unwrap_or_default();
                    let reply = PluginTrayMessage::new(
                        PluginTrayMessageType::GetRecents,
                        serde_json::json!({ "server": srv, "recents": rec }),
                    );
                    let _ = reply.send_to(&mut stream);
                }
                PluginTrayMessageType::Stop => break,
                _ => {}
            }
        }
        statuses.lock().remove(&peer_key);
    }

    /// Render the `PluginMonitor` window onto a `GoodLookinUI` `Canvas`.
    pub fn render_monitor(&self, width: u32, height: u32) -> Canvas {
        let mut canvas = Canvas::new(width, height);
        draw_panel(
            &mut canvas,
            Rect::new(0.0, 0.0, width as f32, height as f32),
        );
        let header = Rect::new(24.0, 18.0, (width as f32) - 48.0, 22.0);
        canvas.draw_text_left(
            "OPENGRIDDER PLUGIN MONITOR",
            header,
            1,
            Color::from_argb(0xffffca28),
        );

        let list = self.statuses();
        for (i, st) in list.iter().enumerate() {
            let row = Rect::new(
                28.0,
                48.0 + (i as f32) * 26.0,
                (width as f32) - 56.0,
                22.0,
            );
            let line = format!(
                "{}  |  {}  |  {:.2} ms  |  {}",
                st.channel_name,
                st.loaded_plugins,
                st.perf_95th_ms,
                if st.connected { "OK" } else { "OFFLINE" }
            );
            canvas.draw_text_left(&line, row, 1, Color::from_argb(0xffece3d3));
        }
        canvas
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.accept_handle.lock().take() {
            let _ = h.join();
        }
    }
}

impl Drop for PluginTrayServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tray_status_and_recents_ipc() {
        let tray = PluginTrayServer::new();
        let port = tray.start("127.0.0.1", 0).expect("bind tray");

        let mut client = TcpStream::connect(("127.0.0.1", port)).expect("connect tray");
        let status_msg = PluginTrayMessage::new(
            PluginTrayMessageType::Status,
            serde_json::json!({
                "name": "Lead Vocal",
                "colour": 0xff00ff00u32,
                "loadedPlugins": "Pro-Q 3 -> 1176",
                "perf": 1.25,
                "blocks": 4,
                "ok": true
            }),
        );
        status_msg.send_to(&mut client).expect("send status");

        let rec_msg = PluginTrayMessage::new(
            PluginTrayMessageType::UpdateRecents,
            serde_json::json!({
                "server": "127.0.0.1:55056",
                "recents": "VST3-ProQ3\n"
            }),
        );
        rec_msg.send_to(&mut client).expect("send recents");

        let get_rec = PluginTrayMessage::new(
            PluginTrayMessageType::GetRecents,
            serde_json::json!({ "server": "127.0.0.1:55056" }),
        );
        get_rec.send_to(&mut client).expect("request recents");
        let reply = PluginTrayMessage::read_from(&mut client).expect("read recents reply");
        assert_eq!(
            reply.data.get("recents").and_then(|v| v.as_str()),
            Some("VST3-ProQ3\n")
        );

        let statuses = tray.statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].channel_name, "Lead Vocal");
        assert!(statuses[0].connected);

        let monitor_canvas = tray.render_monitor(480, 200);
        assert!(monitor_canvas.pixels.iter().any(|&b| b > 0));
    }
}
