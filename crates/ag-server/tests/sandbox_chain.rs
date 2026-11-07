//! Chain sandbox: the handshake port is a child process, and the sandbox flag is set.
//! Disabled mode stays an in-process worker with the flag clear. This test drives the binary.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ag_protocol::command::{self, PLUGIN_LIST, RESP_LOCAL_MODE, RESP_SANDBOX};
use ag_protocol::{HandshakeRequest, HandshakeResponse, PROTOCOL_VERSION};

struct Kill(Option<Child>);

impl Drop for Kill {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn chain_sandbox_handshake_port_belongs_to_the_child() {
    let bin = env!("CARGO_BIN_EXE_ag-server");
    let mut child = Command::new(bin)
        .args(["--sandbox", "chain", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("ag-server");
    let master_pid = child.id();
    let stdout = child.stdout.take().expect("stdout");
    let kill = Kill(Some(child));
    let (send, recv) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        let _ = std::io::BufRead::read_line(&mut reader, &mut line);
        let _ = send.send(line);
    });
    let line = recv.recv_timeout(Duration::from_secs(3)).unwrap_or_else(|_| panic!("master did not announce a port"));
    assert!(line.starts_with("master "), "expected a chain master port, got {line:?}");
    let master_port: u16 = line["master ".len()..].trim().parse().expect("master port");
    eprintln!("chain master {master_port} pid {master_pid}");

    let answer = handshake(master_port);
    eprintln!("chain answer version {} flags {} port {}", answer.version, answer.flags, answer.port);
    assert_eq!(answer.version, PROTOCOL_VERSION);
    assert_eq!(answer.flags & RESP_SANDBOX, RESP_SANDBOX, "sandbox flag was clear");
    assert_eq!(answer.flags & RESP_LOCAL_MODE, 0, "local mode was set");
    assert_ne!(answer.port, i32::from(master_port), "handshake returned the master port");
    let worker = u16::try_from(answer.port).expect("worker port");

    let listener = listener_pid(worker);
    let parent = parent_pid(listener);
    eprintln!("worker {worker} listener {listener} parent {parent}");
    assert_ne!(listener, master_pid, "the worker port is the master process");
    assert_eq!(parent, master_pid, "the listener is not a child of the master");

    let mut sockets = Vec::new();
    for _ in 0..4 {
        let sock = TcpStream::connect(("127.0.0.1", worker)).expect("worker accept");
        sock.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        sockets.push(sock);
    }
    let (kind, body) = read_cmd(&mut sockets[0]);
    eprintln!("child plugin list type {kind} bytes {}", body.len());
    assert_eq!(kind, PLUGIN_LIST, "child did not send the connect-time plugin list");
    assert!(!body.is_empty(), "child plugin list was empty");
    drop(sockets);
    drop(kill);
}

fn handshake(port: u16) -> HandshakeResponse {
    let mut sock = TcpStream::connect(("127.0.0.1", port)).expect("master connect");
    sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    sock.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
    sock.write_all(&request.to_bytes()).unwrap();
    let mut raw = [0u8; HandshakeResponse::WIRE_SIZE];
    sock.read_exact(&mut raw).expect("handshake answer");
    HandshakeResponse::from_bytes(&raw).expect("handshake bytes")
}

fn read_cmd(sock: &mut TcpStream) -> (i32, Vec<u8>) {
    let mut header = [0u8; command::CMD_HEADER];
    sock.read_exact(&mut header).expect("command header");
    let kind = i32::from_le_bytes(header[0..4].try_into().unwrap());
    let size = i32::from_le_bytes(header[4..8].try_into().unwrap());
    assert!(size >= 0, "command size {size}");
    let mut body = vec![0u8; size as usize];
    if size > 0 {
        sock.read_exact(&mut body).expect("command body");
    }
    (kind, body)
}

fn listener_pid(port: u16) -> u32 {
    let output = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-Fp"])
        .output()
        .expect("lsof");
    let text = String::from_utf8_lossy(&output.stdout);
    let mut pids = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            if let Ok(pid) = rest.parse() {
                if !pids.contains(&pid) {
                    pids.push(pid);
                }
            }
        }
    }
    assert_eq!(pids.len(), 1, "listeners on {port}: {text}");
    pids[0]
}

fn parent_pid(pid: u32) -> u32 {
    let output = Command::new("ps").args(["-o", "ppid=", "-p", &pid.to_string()]).output().expect("ps");
    String::from_utf8_lossy(&output.stdout).trim().parse().expect("ppid")
}
