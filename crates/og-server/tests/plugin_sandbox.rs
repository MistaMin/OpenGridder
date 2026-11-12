//! Plugin isolation: the worker port stays in the master, the sandbox flag stays
//! clear, and a dead plugin child is reloaded with the last settings.
//! This test drives the binary.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use og_protocol::command::{self, ADD_PLUGIN, ADD_PLUGIN_RESULT, DEL_PLUGIN, PARAMETERS, PLUGIN_LIST, PLUGIN_STATUS, PRESETS, RESP_LOCAL_MODE, RESP_SANDBOX, RESULT};
use og_protocol::{read_audio_response, write_audio_request, HandshakeRequest, HandshakeResponse, PositionInfo, PROTOCOL_VERSION};

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
fn plugin_sandbox_keeps_the_listener_in_the_master_and_reloads_after_the_child_dies() {
    let bin = env!("CARGO_BIN_EXE_og-server");
    let mut child = Command::new(bin)
        .args(["--sandbox", "plugin", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("og-server");
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
    assert!(line.starts_with("master "), "expected a plugin master port, got {line:?}");
    let master_port: u16 = line["master ".len()..].trim().parse().expect("master port");
    eprintln!("plugin master {master_port} pid {master_pid}");

    let answer = handshake(master_port);
    eprintln!("plugin answer version {} flags {} port {}", answer.version, answer.flags, answer.port);
    assert_eq!(answer.version, PROTOCOL_VERSION);
    assert_eq!(answer.flags & RESP_SANDBOX, 0, "plugin isolation set the sandbox flag");
    assert_eq!(answer.flags & RESP_LOCAL_MODE, 0, "local mode was set");
    let worker = u16::try_from(answer.port).expect("worker port");
    assert_ne!(worker, master_port, "handshake returned the master port");

    let listener = listener_pid(worker);
    eprintln!("worker {worker} listener {listener}");
    assert_eq!(listener, master_pid, "the worker port is not the master process");

    let mut sockets = Vec::new();
    for _ in 0..4 {
        let sock = TcpStream::connect(("127.0.0.1", worker)).expect("worker accept");
        sock.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(30))).unwrap();
        sockets.push(sock);
    }
    let mut command = sockets.remove(0);
    let mut inbound = sockets.remove(0);
    let mut audio = sockets.remove(0);
    let _screen = sockets.remove(0);

    let (kind, body) = read_cmd(&mut command);
    eprintln!("plugin list type {kind} bytes {}", body.len());
    assert_eq!(kind, PLUGIN_LIST, "master did not send the connect-time plugin list");
    assert!(!body.is_empty(), "plugin list was empty");

    let add = command::json_payload(&serde_json::json!({"id": "Builtin-gain", "settings": "6"}));
    write_cmd(&mut command, ADD_PLUGIN, &add);
    let (kind, body) = read_cmd(&mut command);
    assert_eq!(kind, ADD_PLUGIN_RESULT);
    let result = command::read_json_payload(&body).expect("add result");
    assert_eq!(result["success"], true, "add failed: {result}");
    let (kind, _) = read_cmd(&mut command);
    assert_eq!(kind, PRESETS);
    let (kind, _) = read_cmd(&mut command);
    assert_eq!(kind, PARAMETERS);
    let (kind, body) = read_cmd(&mut inbound);
    assert_eq!(kind, PLUGIN_STATUS, "ready status was not pushed");
    let ready = command::read_json_payload(&body).expect("ready status");
    assert_eq!(ready["ok"], true, "plugin was not ready: {ready}");

    let gained = first_sample(&mut audio);
    eprintln!("plugin audio before {gained}");
    assert!((gained - 1.9952624).abs() < 1.0e-5, "hosted gain was not +6 dB, got {gained}");

    let children = plugin_children(master_pid);
    eprintln!("plugin children {children:?}");
    assert_eq!(children.len(), 1, "expected one plugin child");
    let child_pid = children[0];
    assert_ne!(child_pid, master_pid);
    assert_eq!(parent_pid(child_pid), master_pid, "plugin child is not parented by the master");
    let child_port = listen_port(child_pid);
    eprintln!("plugin child {child_pid} port {child_port} established {}", established_count(child_pid));
    assert_ne!(child_port, worker, "the child is listening on the client worker port");
    assert_eq!(listener_pid(child_port), child_pid, "the child port is not the child process");
    assert!(established_count(child_pid) >= 3, "the plugin child does not have three sockets");

    let inbound_reader = inbound.try_clone().expect("inbound clone");
    let (status_send, status_recv) = mpsc::channel();
    thread::spawn(move || {
        let mut sock = inbound_reader;
        let mut got = Vec::new();
        for _ in 0..2 {
            let (kind, body) = read_cmd(&mut sock);
            got.push((kind, body));
        }
        let _ = status_send.send(got);
    });
    let _ = Command::new("kill").args(["-9", &child_pid.to_string()]).status();

    let recovered = first_sample(&mut audio);
    eprintln!("plugin audio after {recovered}");
    let statuses = status_recv.recv_timeout(Duration::from_secs(20)).expect("reload did not push two statuses");
    assert_eq!(statuses.len(), 2);
    let down = command::read_json_payload(&statuses[0].1).expect("down status");
    let up = command::read_json_payload(&statuses[1].1).expect("up status");
    eprintln!("plugin status down {down} up {up}");
    assert_eq!(statuses[0].0, PLUGIN_STATUS);
    assert_eq!(statuses[1].0, PLUGIN_STATUS);
    assert_eq!(down["ok"], false, "death did not mark the plugin down");
    assert_eq!(up["ok"], true, "reload did not mark the plugin ready");
    assert!((recovered - 1.9952624).abs() < 1.0e-5, "reload did not restore +6 dB, got {recovered}");

    let again = plugin_children(master_pid);
    eprintln!("plugin children after reload {again:?}");
    assert_eq!(again.len(), 1, "reload did not leave one plugin child");
    assert_ne!(again[0], child_pid, "reload kept the dead child");
    assert_eq!(parent_pid(again[0]), master_pid);

    write_cmd(&mut command, DEL_PLUGIN, &command::number(0));
    let (kind, _) = read_cmd(&mut command);
    assert_eq!(kind, RESULT, "unload did not reply");
    let dry = first_sample(&mut audio);
    eprintln!("plugin audio unloaded {dry}");
    assert!((dry - 1.0).abs() < 1.0e-5, "unload did not remove the gain, got {dry}");

    let leftover = plugin_children(master_pid);
    for pid in leftover {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
    let after_unload = first_sample(&mut audio);
    eprintln!("plugin audio after unloaded child died {after_unload}");
    assert!((after_unload - 1.0).abs() < 1.0e-5, "an unloaded plugin was reloaded, got {after_unload}");
    inbound.set_read_timeout(Some(Duration::from_millis(400))).unwrap();
    let mut header = [0u8; command::CMD_HEADER];
    let reread = inbound.read(&mut header);
    assert!(reread.is_err() || matches!(reread, Ok(0)), "unload pushed another status: {reread:?} {header:?}");
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

fn write_cmd(sock: &mut TcpStream, message_type: i32, body: &[u8]) {
    sock.write_all(&command::encode(message_type, body)).expect("write command");
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

fn first_sample(audio: &mut TcpStream) -> f32 {
    let block = write_audio_request(&[vec![1.0; 32], vec![1.0; 32]], &PositionInfo::default()).unwrap();
    audio.write_all(&block).expect("audio request");
    let mut response = vec![0u8; 16 + 2 * 32 * 4];
    audio.read_exact(&mut response).expect("audio response");
    let (_header, samples) = read_audio_response(&response).expect("audio body");
    samples[0][0]
}

fn listener_pid(port: u16) -> u32 {
    let output = Command::new("lsof").args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-Fp"]).output().expect("lsof");
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

fn plugin_children(master: u32) -> Vec<u32> {
    let output = Command::new("ps").args(["-axo", "pid=,ppid=,command="]).output().expect("ps");
    let text = String::from_utf8_lossy(&output.stdout);
    let mut pids = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.contains("--sandbox-plugin-child") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(pid) = parts.next().and_then(|text| text.parse::<u32>().ok()) else { continue };
        let Some(ppid) = parts.next().and_then(|text| text.parse::<u32>().ok()) else { continue };
        if ppid == master {
            pids.push(pid);
        }
    }
    pids
}

fn listen_port(pid: u32) -> u16 {
    let output = Command::new("lsof").args(["-nP", "-a", "-p", &pid.to_string(), "-iTCP", "-sTCP:LISTEN", "-Fn"]).output().expect("lsof");
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('n') {
            if let Some(port) = rest.rsplit(':').next().and_then(|text| text.parse().ok()) {
                return port;
            }
        }
    }
    panic!("plugin child {pid} is not listening: {text}");
}

fn established_count(pid: u32) -> usize {
    let output = Command::new("lsof").args(["-nP", "-a", "-p", &pid.to_string(), "-iTCP", "-sTCP:ESTABLISHED", "-Fn"]).output().expect("lsof");
    String::from_utf8_lossy(&output.stdout).lines().filter(|line| line.starts_with('n')).count()
}
