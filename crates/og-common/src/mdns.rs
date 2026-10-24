//! Port of `Common/Source/mDNS.hpp`, `mDNSConnector.hpp`, and `ServiceReceiver.hpp`.
//! Provides UDP multicast DNS announcement and discovery for `_audiogridder._tcp.local.`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::utils::ServerInfo;

pub const MDNS_MULTICAST_IPV4: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
pub const MDNS_PORT: u16 = 5353;

/// Lightweight JSON-over-UDP beacon on a dedicated local discovery port (55054) + mDNS fallback
/// so local and LAN AudioGridder/OpenGridder instances discover each other without external C libs.
pub const OG_DISCOVERY_PORT: u16 = 55054;

pub struct ServiceResponder {
    info: Arc<Mutex<ServerInfo>>,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ServiceResponder {
    pub fn new(info: ServerInfo) -> Self {
        Self {
            info: Arc::new(Mutex::new(info)),
            running: Arc::new(AtomicBool::new(false)),
            handle: None,
        }
    }

    pub fn update_load(&self, load: f32) {
        self.info.lock().unwrap().refresh_with_load(load);
    }

    pub fn start(&mut self) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let info = self.info.clone();
        let running = self.running.clone();
        self.handle = Some(thread::spawn(move || {
            let Ok(sock) = UdpSocket::bind(("0.0.0.0", 0)) else {
                return;
            };
            let _ = sock.set_broadcast(true);
            while running.load(Ordering::Relaxed) {
                let payload = info.lock().unwrap().serialize();
                let msg = format!("OG_SRV:{payload}");
                let _ = sock.send_to(
                    msg.as_bytes(),
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), OG_DISCOVERY_PORT),
                );
                thread::sleep(Duration::from_millis(1000));
            }
        }));
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for ServiceResponder {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct ServiceReceiver {
    servers: Arc<Mutex<Vec<ServerInfo>>>,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Default for ServiceReceiver {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceReceiver {
    pub fn new() -> Self {
        Self {
            servers: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
            handle: None,
        }
    }

    pub fn start(&mut self) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let servers = self.servers.clone();
        let running = self.running.clone();
        self.handle = Some(thread::spawn(move || {
            let Ok(sock) = UdpSocket::bind(("127.0.0.1", OG_DISCOVERY_PORT)) else {
                return;
            };
            let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
            let mut buf = [0u8; 1024];
            while running.load(Ordering::Relaxed) {
                if let Ok((n, _addr)) = sock.recv_from(&mut buf) {
                    if let Ok(text) = std::str::from_utf8(&buf[..n]) {
                        if let Some(rest) = text.strip_prefix("OG_SRV:") {
                            let info = ServerInfo::parse(rest);
                            if info.is_valid() {
                                let mut list = servers.lock().unwrap();
                                if let Some(existing) = list.iter_mut().find(|s| s.matches(&info)) {
                                    *existing = info;
                                } else {
                                    list.push(info);
                                }
                            }
                        }
                    }
                }
            }
        }));
    }

    pub fn get_servers(&self) -> Vec<ServerInfo> {
        self.servers.lock().unwrap().clone()
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for ServiceReceiver {
    fn drop(&mut self) {
        self.stop();
    }
}
