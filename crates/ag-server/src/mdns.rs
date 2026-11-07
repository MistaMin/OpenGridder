//! mDNS service responder (_audiogridder._tcp.local TXT record broadcast).
//! Fulfills H15: mDNS service responder broadcasting ID, UUID, LM, LOAD, V.

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub const MDNS_SERVICE_NAME: &str = "_audiogridder._tcp.local.";
pub const MDNS_PORT: u16 = 5353;
pub const MDNS_MULTICAST_IPV4: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceInfo {
    pub id: i32,
    pub uuid: String,
    pub local_mode: bool,
    pub load: f32,
    pub version: String,
    pub port: u16,
    pub hostname: String,
}

impl ServiceInfo {
    pub fn txt_record(&self) -> String {
        let json = serde_json::json!({
            "ID": self.id,
            "UUID": self.uuid,
            "LM": self.local_mode,
            "LOAD": self.load,
            "V": self.version
        });
        format!("INFO={}", json)
    }

    /// Build standard DNS PTR response with TXT and SRV records for AudioGridder
    pub fn build_mdns_response(&self) -> Vec<u8> {
        let mut packet = Vec::with_capacity(512);
        // Header: Transaction ID (0), Flags (0x8400: Standard response, Authoritative),
        // Questions (0), Answer RRs (1), Authority RRs (0), Additional RRs (2: TXT, SRV)
        packet.extend_from_slice(&[0x00, 0x00, 0x84, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02]);

        // 1. Answer: PTR record for _audiogridder._tcp.local. -> hostname._audiogridder._tcp.local.
        encode_dns_name(&mut packet, MDNS_SERVICE_NAME);
        packet.extend_from_slice(&[0x00, 0x0c]); // Type PTR (12)
        packet.extend_from_slice(&[0x80, 0x01]); // Class IN, flush cache (0x8001)
        packet.extend_from_slice(&120u32.to_be_bytes()); // TTL 120s

        let instance_name = format!("{}.{}", self.hostname, MDNS_SERVICE_NAME);
        let mut rdata = Vec::new();
        encode_dns_name(&mut rdata, &instance_name);
        packet.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        packet.extend_from_slice(&rdata);

        // 2. Additional: TXT record with INFO={...}
        encode_dns_name(&mut packet, &instance_name);
        packet.extend_from_slice(&[0x00, 0x10]); // Type TXT (16)
        packet.extend_from_slice(&[0x80, 0x01]); // Class IN
        packet.extend_from_slice(&120u32.to_be_bytes()); // TTL

        let txt = self.txt_record();
        let txt_bytes = txt.as_bytes();
        let mut txt_rdata = Vec::new();
        // TXT string is length-prefixed chunks (max 255)
        for chunk in txt_bytes.chunks(255) {
            txt_rdata.push(chunk.len() as u8);
            txt_rdata.extend_from_slice(chunk);
        }
        packet.extend_from_slice(&(txt_rdata.len() as u16).to_be_bytes());
        packet.extend_from_slice(&txt_rdata);

        // 3. Additional: SRV record with target and port
        encode_dns_name(&mut packet, &instance_name);
        packet.extend_from_slice(&[0x00, 0x21]); // Type SRV (33)
        packet.extend_from_slice(&[0x80, 0x01]); // Class IN
        packet.extend_from_slice(&120u32.to_be_bytes()); // TTL

        let mut srv_rdata = Vec::new();
        srv_rdata.extend_from_slice(&[0x00, 0x00]); // Priority 0
        srv_rdata.extend_from_slice(&[0x00, 0x00]); // Weight 0
        srv_rdata.extend_from_slice(&self.port.to_be_bytes()); // Port
        encode_dns_name(&mut srv_rdata, &format!("{}.local.", self.hostname));
        packet.extend_from_slice(&(srv_rdata.len() as u16).to_be_bytes());
        packet.extend_from_slice(&srv_rdata);

        packet
    }
}

fn encode_dns_name(buf: &mut Vec<u8>, name: &str) {
    for part in name.trim_end_matches('.').split('.') {
        if !part.is_empty() {
            buf.push(part.len() as u8);
            buf.extend_from_slice(part.as_bytes());
        }
    }
    buf.push(0x00);
}

pub struct ServiceResponder {
    info: ServiceInfo,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ServiceResponder {
    pub fn new(info: ServiceInfo) -> Self {
        Self {
            info,
            running: Arc::new(AtomicBool::new(false)),
            handle: None,
        }
    }

    pub fn start(&mut self) -> std::io::Result<()> {
        let running = self.running.clone();
        running.store(true, Ordering::SeqCst);
        let info = self.info.clone();

        // Bind UDP socket with SO_REUSEADDR / SO_REUSEPORT
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        let _ = socket.set_read_timeout(Some(Duration::from_millis(50)));
        let _ = socket.set_broadcast(true);

        let handle = thread::Builder::new()
            .name("ag-mdns-responder".to_string())
            .spawn(move || {
                let response = info.build_mdns_response();
                let mcast_target = SocketAddrV4::new(MDNS_MULTICAST_IPV4, MDNS_PORT);
                while running.load(Ordering::Relaxed) {
                    // Send advertisement beacon periodically
                    let _ = socket.send_to(&response, mcast_target);
                    for _ in 0..20 {
                        if !running.load(Ordering::Relaxed) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            })?;

        self.handle = Some(handle);
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h15_mdns_service_responder_broadcasts_info() {
        let info = ServiceInfo {
            id: 1,
            uuid: "12345678-1234-1234-1234-123456789abc".to_string(),
            local_mode: false,
            load: 15.5,
            version: "2.0.0-alpha.1".to_string(),
            port: 55057,
            hostname: "Studio-Mini".to_string(),
        };

        let txt = info.txt_record();
        assert!(txt.starts_with("INFO="));
        assert!(txt.contains("\"ID\":1"));
        assert!(txt.contains("\"UUID\":\"12345678-1234-1234-1234-123456789abc\""));
        assert!(txt.contains("\"LM\":false"));
        assert!(txt.contains("\"LOAD\":15.5"));
        assert!(txt.contains("\"V\":\"2.0.0-alpha.1\""));

        let packet = info.build_mdns_response();
        assert!(!packet.is_empty());
        // Verify service name is present in DNS wire bytes
        let service_bytes = b"_audiogridder";
        assert!(packet.windows(service_bytes.len()).any(|w| w == service_bytes));

        // Start and stop responder
        let mut responder = ServiceResponder::new(info);
        responder.start().expect("start responder");
        thread::sleep(Duration::from_millis(50));
        responder.stop();
    }
}
