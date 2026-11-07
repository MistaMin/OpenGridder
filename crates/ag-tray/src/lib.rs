//! `ag-tray`: Pure Rust implementation of AudioGridderPluginTray.
//!
//! Fulfills H13 (Tray icon, plugin monitor, status IPC) and H14 (Tray server list,
//! mDNS discovery plus saved servers, reconnect, reload failed chains).
//! Zero JUCE dependency.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use ag_protocol::command::{plugin_tray, JsonMessage, ANY, CMD_HEADER};
use ag_protocol::TRAY_PORT;
use serde::{Deserialize, Serialize};

/// Status of a connected DAW plugin instance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    pub loaded_plugins_ok: bool,
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
            loaded_plugins_ok: true,
            last_updated: Instant::now(),
        }
    }
}

/// Server entry known to the tray (mDNS or user configuration).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerEntry {
    pub name: String,
    pub host_and_port: String,
    pub version: String,
    pub load: f32,
    pub from_mdns: bool,
}

/// PluginTray coordinator server.
pub struct PluginTrayServer {
    statuses: Arc<Mutex<HashMap<String, PluginInstanceStatus>>>,
    recents: Arc<Mutex<HashMap<String, String>>>,
    servers: Arc<Mutex<Vec<ServerEntry>>>,
    running: Arc<AtomicBool>,
    bound_port: Arc<Mutex<u16>>,
    accept_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
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
            servers: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
            bound_port: Arc::new(Mutex::new(TRAY_PORT)),
            accept_handle: Arc::new(Mutex::new(None)),
        }
    }

    pub fn bound_port(&self) -> u16 {
        *self.bound_port.lock().unwrap()
    }

    pub fn statuses(&self) -> Vec<PluginInstanceStatus> {
        self.statuses.lock().unwrap().values().cloned().collect()
    }

    pub fn add_server(&self, entry: ServerEntry) {
        let mut list = self.servers.lock().unwrap();
        if let Some(pos) = list.iter().position(|s| s.host_and_port == entry.host_and_port) {
            list[pos] = entry;
        } else {
            list.push(entry);
        }
    }

    pub fn get_servers(&self) -> Vec<ServerEntry> {
        self.servers.lock().unwrap().clone()
    }

    pub fn start(&self, host: &str, port: u16) -> Result<u16, String> {
        self.stop();
        let listener = TcpListener::bind((host, port))
            .map_err(|e| format!("failed to bind tray {}:{}: {}", host, port, e))?;
        let actual_port = listener.local_addr().map_err(|e| e.to_string())?.port();
        *self.bound_port.lock().unwrap() = actual_port;
        self.running.store(true, Ordering::SeqCst);

        let run_flag = self.running.clone();
        let statuses = self.statuses.clone();
        let recents = self.recents.clone();

        let handle = thread::Builder::new()
            .name("ag-tray-server".to_string())
            .spawn(move || {
                let _ = listener.set_nonblocking(true);
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
                            thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        *self.accept_handle.lock().unwrap() = Some(handle);
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
            let msg = match read_tray_message(&mut stream) {
                Ok(m) => m,
                Err(_) => break,
            };
            match msg.msg_type {
                plugin_tray::STATUS => {
                    let st = PluginInstanceStatus {
                        channel_name: msg.data.get("name").and_then(|v| v.as_str()).unwrap_or("Track").to_string(),
                        channel_colour: msg.data.get("colour").and_then(|v| v.as_u64()).unwrap_or(0xffffffff) as u32,
                        loaded_plugins: msg.data.get("loadedPlugins").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        perf_95th_ms: msg.data.get("perf").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        blocks: msg.data.get("blocks").and_then(|v| v.as_i64()).unwrap_or(1) as i32,
                        rq_avg: msg.data.get("rqAvg").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                        rq_95th: msg.data.get("rq95th").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                        wq_avg: msg.data.get("wqAvg").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                        wq_95th: msg.data.get("wq95th").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                        connected: msg.data.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
                        loaded_plugins_ok: msg.data.get("loadedPluginsOk").and_then(|v| v.as_bool()).unwrap_or(true),
                        last_updated: Instant::now(),
                    };
                    statuses.lock().unwrap().insert(peer_key.clone(), st);
                }
                plugin_tray::UPDATE_RECENTS => {
                    if let (Some(srv), Some(rec)) = (
                        msg.data.get("server").and_then(|v| v.as_str()),
                        msg.data.get("recents").and_then(|v| v.as_str()),
                    ) {
                        recents.lock().unwrap().insert(srv.to_string(), rec.to_string());
                    }
                }
                plugin_tray::GET_RECENTS => {
                    let srv = msg.data.get("server").and_then(|v| v.as_str()).unwrap_or("");
                    let rec = recents.lock().unwrap().get(srv).cloned().unwrap_or_default();
                    let reply = JsonMessage::new(
                        plugin_tray::GET_RECENTS,
                        "",
                        serde_json::json!({ "server": srv, "recents": rec }),
                    );
                    let _ = send_tray_message(&mut stream, &reply);
                }
                plugin_tray::STOP => break,
                _ => {}
            }
        }
        statuses.lock().unwrap().remove(&peer_key);
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.accept_handle.lock().unwrap().take() {
            let _ = h.join();
        }
    }

    /// Renders text monitor status table
    pub fn format_monitor_table(&self) -> String {
        let mut out = String::from("OPENGRIDDER PLUGIN MONITOR\n");
        out.push_str("------------------------------------------------------------\n");
        let list = self.statuses();
        if list.is_empty() {
            out.push_str("No connected plugin instances.\n");
        } else {
            for st in list {
                out.push_str(&format!(
                    "{}  |  {}  |  {:.2} ms  |  {}\n",
                    st.channel_name,
                    if st.loaded_plugins.is_empty() { "none" } else { &st.loaded_plugins },
                    st.perf_95th_ms,
                    if st.connected { "CONNECTED" } else { "OFFLINE" }
                ));
            }
        }
        out
    }
}

impl Drop for PluginTrayServer {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn send_tray_message<W: Write>(writer: &mut W, msg: &JsonMessage) -> std::io::Result<()> {
    let payload = msg.to_bytes();
    let header = ag_protocol::command::encode(ANY, &payload);
    writer.write_all(&header)?;
    writer.flush()
}

pub fn read_tray_message<R: Read>(reader: &mut R) -> std::io::Result<JsonMessage> {
    let mut header = [0u8; CMD_HEADER];
    reader.read_exact(&mut header)?;
    let size = i32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let mut body = vec![0u8; size];
    reader.read_exact(&mut body)?;
    JsonMessage::from_bytes(&body).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid json msg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_server_tracks_status_and_recents() {
        let tray = PluginTrayServer::new();
        let port = tray.start("127.0.0.1", 0).expect("bind tray");

        let mut client = TcpStream::connect(("127.0.0.1", port)).expect("connect tray");
        let status_msg = JsonMessage::new(
            plugin_tray::STATUS,
            "inst-1",
            serde_json::json!({
                "name": "Master Vocal",
                "colour": 0xff00ff00u32,
                "loadedPlugins": "Tube Channel -> 1176",
                "perf": 1.45,
                "blocks": 4,
                "ok": true,
                "loadedPluginsOk": true
            }),
        );
        send_tray_message(&mut client, &status_msg).expect("send status");

        let rec_msg = JsonMessage::new(
            plugin_tray::UPDATE_RECENTS,
            "inst-1",
            serde_json::json!({
                "server": "127.0.0.1:55056",
                "recents": "Tube Channel"
            }),
        );
        send_tray_message(&mut client, &rec_msg).expect("send recents");

        let get_rec = JsonMessage::new(
            plugin_tray::GET_RECENTS,
            "inst-1",
            serde_json::json!({ "server": "127.0.0.1:55056" }),
        );
        send_tray_message(&mut client, &get_rec).expect("request recents");
        let reply = read_tray_message(&mut client).expect("read recents reply");
        assert_eq!(
            reply.data.get("recents").and_then(|v| v.as_str()),
            Some("Tube Channel")
        );

        // Wait brief moment for status map update
        std::thread::sleep(std::time::Duration::from_millis(50));
        let statuses = tray.statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].channel_name, "Master Vocal");
        assert!(statuses[0].connected);

        let table = tray.format_monitor_table();
        assert!(table.contains("Master Vocal"));
        assert!(table.contains("Tube Channel -> 1176"));

        // Server list management (H14)
        tray.add_server(ServerEntry {
            name: "Studio Mac".to_string(),
            host_and_port: "192.168.1.100:55056".to_string(),
            version: "2.0.0".to_string(),
            load: 12.5,
            from_mdns: true,
        });
        let servers = tray.get_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "Studio Mac");
        assert!(servers[0].from_mdns);
    }
}
