use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use og_protocol::command::{RESP_LOCAL_MODE, RESP_SANDBOX};
use og_protocol::{HandshakeRequest, HandshakeResponse, PROTOCOL_VERSION};

struct Kill(Option<Child>);

impl Drop for Kill {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_server(args: &[&str]) -> (Child, u16) {
    let bin = env!("CARGO_BIN_EXE_og-server");
    let mut child = Command::new(bin)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("og-server");
    let stdout = child.stdout.take().expect("stdout");
    let (send, recv) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        let _ = std::io::BufRead::read_line(&mut reader, &mut line);
        let _ = send.send(line);
    });
    let line = recv
        .recv_timeout(Duration::from_secs(3))
        .unwrap_or_else(|_| panic!("master did not announce a port for args {:?}", args));
    let port_str = line
        .trim()
        .split_whitespace()
        .last()
        .unwrap_or_else(|| panic!("invalid announcement line: {:?}", line));
    let port: u16 = port_str.parse().expect("parsed port");
    (child, port)
}

fn send_handshake(port: u16) -> HandshakeResponse {
    let mut sock = TcpStream::connect(("127.0.0.1", port)).expect("master connect");
    sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    sock.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
    sock.write_all(&request.to_bytes()).unwrap();
    let mut raw = [0u8; HandshakeResponse::WIRE_SIZE];
    sock.read_exact(&mut raw).expect("handshake answer");
    HandshakeResponse::from_bytes(&raw).expect("handshake bytes")
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

#[test]
fn sandbox_modes_assert_disabled_chain_and_plugin_isolation() {
    // 1. Mode: Chain isolation (--sandbox chain)
    // - handshake response has flags RESP_SANDBOX set
    // - handshake worker port belongs to a child process (listener != master, ppid == master)
    {
        let (child, master_port) = spawn_server(&["--sandbox", "chain", "--port", "0"]);
        let master_pid = child.id();
        let _kill = Kill(Some(child));

        let answer = send_handshake(master_port);
        assert_eq!(answer.version, PROTOCOL_VERSION);
        assert_eq!(answer.flags & RESP_SANDBOX, RESP_SANDBOX, "chain sandbox flag was not set");
        assert_eq!(answer.flags & RESP_LOCAL_MODE, 0, "local mode was set");
        let worker_port = u16::try_from(answer.port).expect("worker port");
        assert_ne!(worker_port, master_port, "chain handshake returned master port");

        let listener = listener_pid(worker_port);
        let parent = parent_pid(listener);
        assert_ne!(listener, master_pid, "chain worker listener is in master process, expected child process");
        assert_eq!(parent, master_pid, "chain worker listener parent is not master");
    }

    // 2. Mode: Plugin isolation (--sandbox plugin)
    // - handshake response has flags 0 (RESP_SANDBOX clear)
    // - handshake worker port belongs to master process (listener == master)
    {
        let (child, master_port) = spawn_server(&["--sandbox", "plugin", "--port", "0"]);
        let master_pid = child.id();
        let _kill = Kill(Some(child));

        let answer = send_handshake(master_port);
        assert_eq!(answer.version, PROTOCOL_VERSION);
        assert_eq!(answer.flags & RESP_SANDBOX, 0, "plugin isolation set sandbox flag");
        assert_eq!(answer.flags & RESP_LOCAL_MODE, 0, "local mode was set");
        let worker_port = u16::try_from(answer.port).expect("worker port");
        assert_ne!(worker_port, master_port, "plugin handshake returned master port");

        let listener = listener_pid(worker_port);
        assert_eq!(listener, master_pid, "plugin worker listener is not in master process");
    }

    // 3. Mode: Disabled isolation (default / serve_clients)
    // - flags 0 (RESP_SANDBOX clear)
    // - handshake worker port belongs to master process (listener == master)
    // Test in-process using an ephemeral listener with worker::serve_clients
    {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let master_port = listener.local_addr().unwrap().port();
        let master_pid = std::process::id();
        let _server = thread::spawn(move || og_server::serve_clients(listener));

        let answer = send_handshake(master_port);
        assert_eq!(answer.version, PROTOCOL_VERSION);
        assert_eq!(answer.flags & RESP_SANDBOX, 0, "disabled sandbox set sandbox flag");
        assert_eq!(answer.flags & RESP_LOCAL_MODE, 0, "local mode was set");
        let worker_port = u16::try_from(answer.port).expect("worker port");
        assert_ne!(worker_port, master_port, "disabled handshake returned master port");

        let listener = listener_pid(worker_port);
        assert_eq!(listener, master_pid, "disabled worker listener is not in master process");
    }
}
