//! AudioGridder's four-socket worker.
//!
//! The client closes the handshake socket, then connects four times to the
//! port in the handshake answer: command-out, command-in, audio, screen.
//! Request replies are written back on command-out, matching
//! `Worker::handleMessage` for plugin list, add, and parameters.
//! Command-in carries server pushes: KEY for an applied arrow, ParameterGesture
//! around a mouse edit, ParameterValue when that edit changes a parameter,
//! PluginStatus when a plugin is ready, and HidePlugin when the editor's close
//! box is pressed. After the four sockets connect, the worker writes the plugin
//! list on the command socket before it reads. The audio thread applies the
//! block's playhead before it processes.
//! `Rescan` and `Restart` close this worker. `Quit` returns Ok. `CPULoad`
//! replies with the host percent on the command socket.
//! Screen bytes are a PNG of the generic editor, not a desktop capture.
//! Diff detection is on, so an unchanged capture is not sent, every 20th
//! capture is a full frame, and a mostly black or white capture resets the
//! editor instead of being sent. JPEG quality is not applied.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ag_protocol::command::{self, CLIPBOARD, QUIT, RESCAN, RESP_SANDBOX, RESTART, SCREEN_CAPTURE};
use ag_protocol::{
    audio_shape_ok, write_audio_response, AudioRequestHeader, AudioResponseHeader, HandshakeRequest, HandshakeResponse,
    PositionInfo, PROTOCOL_VERSION,
};

use crate::chain::PluginChain;
use crate::validate_handshake;

const IO_TIMEOUT: Duration = Duration::from_secs(2);

/// AudioGridder `EXIT_RESTART`: `run_session` returns `Interrupted` with one of
/// `restart`, `restart-rescan`, or `restart-rescan-wipe`. The worker thread
/// exits the process with code 1. `serve_worker` returns the same error to
/// the caller instead of exiting.
pub fn restart_requested(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::Interrupted
        && matches!(err.to_string().as_str(), "restart" | "restart-rescan" | "restart-rescan-wipe")
}

struct AcceptedClient {
    worker: TcpListener,
    request: HandshakeRequest,
}

/// Read one handshake on `listener`, bind that client's worker port, and
/// answer. `Ok(None)` is a client that left or sent a rejected handshake.
/// The server port stays open for the next client.
fn accept_client(listener: &TcpListener) -> std::io::Result<Option<AcceptedClient>> {
    let (mut handshake, _) = listener.accept()?;
    handshake.set_read_timeout(Some(IO_TIMEOUT))?;
    handshake.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut raw = [0u8; HandshakeRequest::WIRE_SIZE];
    if let Err(err) = handshake.read_exact(&mut raw) {
        if is_wait(err.kind()) || err.kind() == std::io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(err);
    }
    let req = match HandshakeRequest::from_bytes(&raw) {
        Some(req) => req,
        None => return Ok(None),
    };
    if validate_handshake(&req).is_err() {
        return Ok(None);
    }
    let worker = TcpListener::bind((handshake.local_addr()?.ip(), 0))?;
    let port = worker.local_addr()?.port();
    let answer = HandshakeResponse { version: PROTOCOL_VERSION, flags: 0, port: i32::from(port) };
    if let Err(err) = handshake.write_all(&answer.to_bytes()) {
        if err.kind() == std::io::ErrorKind::UnexpectedEof || is_wait(err.kind()) {
            return Ok(None);
        }
        return Err(err);
    }
    drop(handshake);
    Ok(Some(AcceptedClient { worker, request: req }))
}

/// Chain sandbox. Each handshake spawns this executable as `--sandbox-chain-child`.
/// The handshake answer's port is the child's listener, and `SANDBOX_ENABLED` is set.
/// Disabled mode stays `serve_clients`: flags 0, and the worker is a thread.
pub fn serve_chain(listener: TcpListener) -> std::io::Result<()> {
    crate::cpu::start();
    listener.set_nonblocking(false)?;
    let mut children = Vec::new();
    loop {
        let (mut handshake, _) = match listener.accept() {
            Ok(accepted) => accepted,
            Err(err) => return Err(err),
        };
        handshake.set_read_timeout(Some(IO_TIMEOUT))?;
        handshake.set_write_timeout(Some(IO_TIMEOUT))?;
        let mut raw = [0u8; HandshakeRequest::WIRE_SIZE];
        if handshake.read_exact(&mut raw).is_err() {
            continue;
        }
        let Some(req) = HandshakeRequest::from_bytes(&raw) else {
            continue;
        };
        if validate_handshake(&req).is_err() {
            continue;
        }
        let mut child = match spawn_chain_child(&req) {
            Ok(child) => child,
            Err(err) => {
                eprintln!("ag-server: {err}");
                continue;
            }
        };
        let port = match read_child_port(&mut child) {
            Ok(port) => port,
            Err(err) => {
                eprintln!("ag-server: {err}");
                drop(child);
                continue;
            }
        };
        let answer = HandshakeResponse { version: PROTOCOL_VERSION, flags: RESP_SANDBOX, port };
        if handshake.write_all(&answer.to_bytes()).is_err() {
            drop(child);
            continue;
        }
        children.push(child);
    }
}

/// Entry point for `--sandbox-chain-child`. The parent writes one handshake on
/// stdin and keeps that pipe open. This process binds a port, writes it to
/// stdout, and hosts the four-socket worker. Stdin closing ends the process.
pub fn sandbox_chain_child() -> std::io::Result<()> {
    crate::cpu::start();
    let mut raw = [0u8; HandshakeRequest::WIRE_SIZE];
    std::io::stdin().read_exact(&mut raw)?;
    let req = HandshakeRequest::from_bytes(&raw).ok_or_else(|| invalid("sandbox handshake"))?;
    validate_handshake(&req).map_err(|_| invalid("sandbox handshake rejected"))?;
    let worker = TcpListener::bind(("127.0.0.1", 0))?;
    let port = i32::from(worker.local_addr()?.port());
    let mut out = std::io::stdout();
    out.write_all(&port.to_le_bytes())?;
    out.flush()?;
    thread::spawn(|| {
        let mut byte = [0u8; 1];
        let _ = std::io::stdin().read(&mut byte);
        std::process::exit(0);
    });
    let result = run_session(AcceptedClient { worker, request: req });
    if let Err(err) = &result {
        if restart_requested(err) {
            eprintln!("ag-server: {err}");
            std::process::exit(1);
        }
        eprintln!("ag-server: {err}");
    }
    result
}

struct SandboxChild(Option<std::process::Child>);

impl Drop for SandboxChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_chain_child(request: &HandshakeRequest) -> std::io::Result<SandboxChild> {
    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe)
        .arg("--sandbox-chain-child")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let stdin = child.stdin.as_mut().ok_or_else(|| invalid("sandbox stdin"))?;
    stdin.write_all(&request.to_bytes())?;
    stdin.flush()?;
    Ok(SandboxChild(Some(child)))
}

fn read_child_port(child: &mut SandboxChild) -> std::io::Result<i32> {
    let proc = child.0.as_mut().ok_or_else(|| invalid("sandbox child"))?;
    let stdout = proc.stdout.as_mut().ok_or_else(|| invalid("sandbox stdout"))?;
    let mut raw = [0u8; 4];
    stdout.read_exact(&mut raw)?;
    let port = i32::from_le_bytes(raw);
    if port <= 0 || port > i32::from(u16::MAX) {
        return Err(invalid("sandbox port"));
    }
    Ok(port)
}

/// Plugin isolation. The handshake answer keeps `SANDBOX_ENABLED` clear and the
/// worker port belongs to this process. Each client chain runs in
/// `--sandbox-plugin-child`: command-out, command-in, and audio. No screen
/// socket and no connect-time plugin list on that child. A dead child is
/// started again and the last successful add is replayed. An unload clears
/// that memory first, so the plugin is not brought back.
pub fn serve_plugin(listener: TcpListener) -> std::io::Result<()> {
    crate::cpu::start();
    listener.set_nonblocking(false)?;
    loop {
        match accept_client(&listener) {
            Ok(Some(accepted)) => {
                thread::spawn(move || {
                    if let Err(err) = run_session_isolated(accepted) {
                        if restart_requested(&err) {
                            eprintln!("ag-server: {err}");
                            std::process::exit(1);
                        }
                        eprintln!("ag-server: {err}");
                    }
                });
            }
            Ok(None) => continue,
            Err(err) => return Err(err),
        }
    }
}

/// Entry point for `--sandbox-plugin-child`. The parent writes one handshake
/// on stdin and keeps that pipe open. This process binds a port, writes it
/// to stdout, accepts three sockets, and hosts the chain.
pub fn sandbox_plugin_child() -> std::io::Result<()> {
    crate::cpu::start();
    let mut raw = [0u8; HandshakeRequest::WIRE_SIZE];
    std::io::stdin().read_exact(&mut raw)?;
    let req = HandshakeRequest::from_bytes(&raw).ok_or_else(|| invalid("sandbox handshake"))?;
    validate_handshake(&req).map_err(|_| invalid("sandbox handshake rejected"))?;
    let worker = TcpListener::bind(("127.0.0.1", 0))?;
    let port = i32::from(worker.local_addr()?.port());
    let mut out = std::io::stdout();
    out.write_all(&port.to_le_bytes())?;
    out.flush()?;
    thread::spawn(|| {
        let mut byte = [0u8; 1];
        let _ = std::io::stdin().read(&mut byte);
        std::process::exit(0);
    });
    run_plugin_child_session(worker, req)
}

fn run_plugin_child_session(worker: TcpListener, req: HandshakeRequest) -> std::io::Result<()> {
    worker.set_nonblocking(false)?;
    let (mut command, _) = accept_timeout(&worker)?;
    let (mut inbound, _) = accept_timeout(&worker)?;
    let (audio, _) = accept_timeout(&worker)?;
    for sock in [&command, &inbound, &audio] {
        sock.set_read_timeout(Some(IO_TIMEOUT))?;
        sock.set_write_timeout(Some(IO_TIMEOUT))?;
    }
    let chain = Arc::new(Mutex::new(PluginChain::new(req.channels_out)));
    {
        let mut chain = chain.lock().expect("chain");
        chain.set_route(req.channels_in, req.channels_sc, req.active_channels);
        chain.set_plugin_list_filter(req.flags);
        chain.prepare(req.sample_rate as f32, req.samples_per_block as usize, req.channels_in as usize);
    }
    let audio_chain = Arc::clone(&chain);
    let audio_shutdown = audio.try_clone()?;
    let audio_thread = thread::spawn(move || audio_loop(audio, audio_chain));
    loop {
        let (message_type, body) = match read_cmd(&mut command) {
            Ok(message) => message,
            Err(err) if is_wait(err.kind()) => continue,
            Err(err) if client_disconnected(&err) => break,
            Err(err) => return Err(err),
        };
        if message_type == QUIT {
            let _ = write_i32(&mut command, 0);
            let _ = write_i32(&mut inbound, 0);
            break;
        }
        let (replies, pushes) = {
            let mut chain = chain.lock().expect("chain");
            let replies = chain.handle(message_type, &body);
            let pushes = chain.take_outbound();
            let _ = chain.take_screen();
            (replies, pushes)
        };
        if write_burst(&mut command, &replies).is_err() || write_burst(&mut inbound, &pushes).is_err() {
            break;
        }
    }
    let _ = audio_shutdown.shutdown(Shutdown::Both);
    let _ = audio_thread.join();
    Ok(())
}

struct PluginLink {
    child: SandboxChild,
    command: TcpStream,
    inbound: TcpStream,
    audio: TcpStream,
}

struct PluginBridge {
    child: SandboxChild,
    command: TcpStream,
    inbound: TcpStream,
    audio: TcpStream,
    request: HandshakeRequest,
    loaded: bool,
    add: Vec<u8>,
    follow: Vec<(i32, Vec<u8>)>,
    generation: u64,
}

fn run_session_isolated(accepted: AcceptedClient) -> std::io::Result<()> {
    let AcceptedClient { worker, request: req } = accepted;
    worker.set_nonblocking(false)?;
    let mut sockets = Vec::with_capacity(4);
    for _ in 0..4 {
        let (sock, _) = accept_timeout(&worker)?;
        sock.set_read_timeout(Some(IO_TIMEOUT))?;
        sock.set_write_timeout(Some(IO_TIMEOUT))?;
        sockets.push(sock);
    }
    let mut command = sockets.remove(0);
    let client_in = Arc::new(Mutex::new(sockets.remove(0)));
    let audio = sockets.remove(0);
    let _screen = sockets.remove(0);
    let audio_shutdown = audio.try_clone()?;

    let client_host = command.peer_addr().map(|addr| addr.ip().to_string()).unwrap_or_default();
    let list = {
        let mut catalog = PluginChain::new(req.channels_out);
        catalog.set_route(req.channels_in, req.channels_sc, req.active_channels);
        catalog.set_plugin_list_filter(req.flags);
        catalog.prepare(req.sample_rate as f32, req.samples_per_block as usize, req.channels_in as usize);
        catalog.set_client_host(client_host);
        catalog.handle(command::PLUGIN_LIST, &[])
    };
    for (reply_type, reply_body) in list {
        if !write_until_disconnect(&mut command, reply_type, &reply_body)? {
            return Ok(());
        }
    }

    let opened = open_plugin_child(&req)?;
    let bridge = Arc::new(Mutex::new(PluginBridge {
        child: opened.child,
        command: opened.command,
        inbound: opened.inbound,
        audio: opened.audio,
        request: req,
        loaded: false,
        add: Vec::new(),
        follow: Vec::new(),
        generation: 0,
    }));
    let audio_bridge = Arc::clone(&bridge);
    let audio_in = Arc::clone(&client_in);
    let audio_thread = thread::spawn(move || isolated_audio(audio, audio_bridge, audio_in));

    let mut quit = false;
    let mut restart: Option<&'static str> = None;
    let mut fatal: Option<std::io::Error> = None;
    while !quit && restart.is_none() && fatal.is_none() {
        let (message_type, body) = match read_cmd(&mut command) {
            Ok(message) => message,
            Err(err) if is_wait(err.kind()) => continue,
            Err(err) if client_disconnected(&err) => break,
            Err(err) => {
                fatal = Some(err);
                break;
            }
        };
        if message_type == RESCAN {
            let wipe = command::read_number(&body) == Some(1);
            restart = Some(crate::host::rescan(wipe));
            break;
        }
        if message_type == RESTART {
            restart = Some("restart");
            break;
        }
        quit = message_type == QUIT;
        let mut bridge = bridge.lock().expect("bridge");
        if message_type == command::DEL_PLUGIN {
            bridge.loaded = false;
            bridge.add.clear();
            bridge.follow.clear();
        }
        let generation = bridge.generation;
        let exchanged = match exchange(&mut bridge, message_type, &body) {
            Ok(messages) => Ok(messages),
            Err(err) if child_lost(&err) || child_is_dead(&mut bridge.child) => {
                if let Err(err) = reload_plugin_child(&mut bridge, generation, &client_in) {
                    Err(err)
                } else {
                    exchange(&mut bridge, message_type, &body)
                }
            }
            Err(err) => Err(err),
        };
        let (replies, pushes) = match exchanged {
            Ok(messages) => messages,
            Err(err) => {
                fatal = Some(err);
                break;
            }
        };
        if message_type == command::ADD_PLUGIN && add_succeeded(&replies) {
            if !bridge.loaded {
                bridge.loaded = true;
                bridge.add = body.clone();
                bridge.follow.clear();
            } else {
                bridge.follow.push((message_type, body.clone()));
            }
        } else if message_type != command::DEL_PLUGIN && bridge.loaded && replayable(message_type) {
            bridge.follow.push((message_type, body.clone()));
        }
        let mut disconnected = false;
        for (reply_type, reply_body) in &replies {
            match write_until_disconnect(&mut command, *reply_type, reply_body) {
                Ok(true) => {}
                Ok(false) => {
                    disconnected = true;
                    break;
                }
                Err(err) => {
                    fatal = Some(err);
                    disconnected = true;
                    break;
                }
            }
        }
        if !disconnected {
            let mut inbound = client_in.lock().expect("command-in");
            for (push_type, push_body) in &pushes {
                match write_until_disconnect(&mut inbound, *push_type, push_body) {
                    Ok(true) => {}
                    Ok(false) => {
                        disconnected = true;
                        break;
                    }
                    Err(err) => {
                        fatal = Some(err);
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        drop(bridge);
        if disconnected {
            break;
        }
    }
    let _ = audio_shutdown.shutdown(Shutdown::Both);
    let _ = audio_thread.join();
    if let Some(err) = fatal {
        return Err(err);
    }
    if let Some(reason) = restart {
        return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, reason));
    }
    Ok(())
}

fn isolated_audio(mut client: TcpStream, bridge: Arc<Mutex<PluginBridge>>, client_in: Arc<Mutex<TcpStream>>) {
    loop {
        let frame = match read_audio_frame(&mut client) {
            Ok(frame) => frame,
            Err(err) if is_wait(err.kind()) => continue,
            Err(_) => break,
        };
        let response = {
            let mut bridge = bridge.lock().expect("bridge");
            match forward_audio(&mut bridge, &frame, &client_in) {
                Ok(bytes) => bytes,
                Err(_) => break,
            }
        };
        if client.write_all(&response).is_err() {
            break;
        }
    }
}

fn forward_audio(bridge: &mut PluginBridge, frame: &[u8], client_in: &Mutex<TcpStream>) -> std::io::Result<Vec<u8>> {
    let generation = bridge.generation;
    if child_is_dead(&mut bridge.child) {
        reload_plugin_child(bridge, generation, client_in)?;
    }
    match send_audio(bridge, frame) {
        Ok(bytes) => Ok(bytes),
        Err(err) if child_lost(&err) || child_is_dead(&mut bridge.child) => {
            let generation = bridge.generation;
            reload_plugin_child(bridge, generation, client_in)?;
            send_audio(bridge, frame)
        }
        Err(err) => Err(err),
    }
}

fn send_audio(bridge: &mut PluginBridge, frame: &[u8]) -> std::io::Result<Vec<u8>> {
    bridge.audio.write_all(frame)?;
    let is_double = frame.get(20).copied().unwrap_or(0) != 0;
    read_audio_response_frame(&mut bridge.audio, is_double)
}

fn reload_plugin_child(bridge: &mut PluginBridge, generation: u64, client_in: &Mutex<TcpStream>) -> std::io::Result<()> {
    if bridge.generation != generation {
        return Ok(());
    }
    let loaded = bridge.loaded;
    let add = bridge.add.clone();
    let follow = bridge.follow.clone();
    let request = bridge.request.clone();
    if loaded {
        push_status(client_in, false, "sandbox died")?;
    }
    let opened = open_plugin_child(&request)?;
    bridge.child = opened.child;
    bridge.command = opened.command;
    bridge.inbound = opened.inbound;
    bridge.audio = opened.audio;
    bridge.generation += 1;
    if !loaded {
        return Ok(());
    }
    let (replies, _) = exchange(bridge, command::ADD_PLUGIN, &add)?;
    if !add_succeeded(&replies) {
        bridge.loaded = false;
        push_status(client_in, false, "reload failed")?;
        return Err(invalid("sandbox reload"));
    }
    for (message_type, body) in &follow {
        let _ = exchange(bridge, *message_type, body)?;
    }
    push_status(client_in, true, "")?;
    Ok(())
}

fn push_status(client_in: &Mutex<TcpStream>, ok: bool, err: &str) -> std::io::Result<()> {
    let body = command::json_payload(&serde_json::json!({"idx": 0, "ok": ok, "err": err}));
    let mut socket = client_in.lock().expect("command-in");
    write_cmd(&mut socket, command::PLUGIN_STATUS, &body)
}

fn exchange(bridge: &mut PluginBridge, message_type: i32, body: &[u8]) -> std::io::Result<(Vec<(i32, Vec<u8>)>, Vec<(i32, Vec<u8>)>)> {
    write_cmd(&mut bridge.command, message_type, body)?;
    let replies = read_burst(&mut bridge.command)?;
    let pushes = read_burst(&mut bridge.inbound)?;
    Ok((replies, pushes))
}

fn open_plugin_child(request: &HandshakeRequest) -> std::io::Result<PluginLink> {
    let mut child = spawn_named_child("--sandbox-plugin-child", request)?;
    let port = read_child_port(&mut child)?;
    let command = connect_child(port as u16)?;
    let inbound = connect_child(port as u16)?;
    let audio = connect_child(port as u16)?;
    for sock in [&command, &inbound, &audio] {
        sock.set_read_timeout(Some(Duration::from_secs(20)))?;
        sock.set_write_timeout(Some(Duration::from_secs(20)))?;
    }
    Ok(PluginLink { child, command, inbound, audio })
}

fn spawn_named_child(arg: &str, request: &HandshakeRequest) -> std::io::Result<SandboxChild> {
    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe).arg(arg).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn()?;
    let stdin = child.stdin.as_mut().ok_or_else(|| invalid("sandbox stdin"))?;
    stdin.write_all(&request.to_bytes())?;
    stdin.flush()?;
    Ok(SandboxChild(Some(child)))
}

fn connect_child(port: u16) -> std::io::Result<TcpStream> {
    let start = std::time::Instant::now();
    let mut last = invalid("sandbox connect");
    while start.elapsed() < Duration::from_secs(3) {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(sock) => return Ok(sock),
            Err(err) => last = err,
        }
        thread::sleep(Duration::from_millis(5));
    }
    Err(last)
}

fn child_is_dead(child: &mut SandboxChild) -> bool {
    match child.0.as_mut() {
        Some(proc) => matches!(proc.try_wait(), Ok(Some(_)) | Err(_)),
        None => true,
    }
}

fn child_lost(err: &std::io::Error) -> bool {
    client_disconnected(err)
}

fn add_succeeded(replies: &[(i32, Vec<u8>)]) -> bool {
    replies.iter().any(|(message_type, body)| {
        *message_type == command::ADD_PLUGIN_RESULT
            && command::read_json_payload(body).and_then(|json| json.get("success").and_then(|value| value.as_bool())).unwrap_or(false)
    })
}

fn replayable(message_type: i32) -> bool {
    matches!(
        message_type,
        command::ADD_PLUGIN
            | command::SET_MONO_CHANNELS
            | command::SET_PLUGIN_SETTINGS
            | command::PLUGIN_SETTINGS
            | command::BYPASS_PLUGIN
            | command::UNBYPASS_PLUGIN
            | command::SET_PLUGIN_OVERSAMPLING
            | command::PARAMETER_VALUE
            | command::PRESET
            | command::EXCHANGE_PLUGINS
    )
}

fn write_i32(socket: &mut TcpStream, value: i32) -> std::io::Result<()> {
    socket.write_all(&value.to_le_bytes())
}

fn read_i32(socket: &mut TcpStream) -> std::io::Result<i32> {
    let mut raw = [0u8; 4];
    socket.read_exact(&mut raw)?;
    let value = i32::from_le_bytes(raw);
    if !(0..=10_000).contains(&value) {
        return Err(invalid("sandbox burst"));
    }
    Ok(value)
}

fn write_burst(socket: &mut TcpStream, messages: &[(i32, Vec<u8>)]) -> std::io::Result<()> {
    write_i32(socket, messages.len() as i32)?;
    for (message_type, body) in messages {
        write_cmd(socket, *message_type, body)?;
    }
    Ok(())
}

fn read_burst(socket: &mut TcpStream) -> std::io::Result<Vec<(i32, Vec<u8>)>> {
    let count = read_i32(socket)?;
    let mut messages = Vec::with_capacity(count as usize);
    for _ in 0..count {
        messages.push(read_cmd(socket)?);
    }
    Ok(messages)
}

fn read_audio_frame(socket: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut header_raw = [0u8; AudioRequestHeader::WIRE_SIZE];
    socket.read_exact(&mut header_raw)?;
    let header = AudioRequestHeader::from_bytes(&header_raw).ok_or_else(|| invalid("audio header"))?;
    if !audio_shape_ok(header.channels, header.samples, header.midi_events, header.is_double) {
        return Err(invalid("audio shape"));
    }
    let width = if header.is_double { 8 } else { 4 };
    let mut rest = vec![0u8; header.channels as usize * header.samples as usize * width];
    socket.read_exact(&mut rest)?;
    let midi = read_midi_events(socket, header.midi_events)?;
    let mut position = [0u8; PositionInfo::WIRE_SIZE];
    socket.read_exact(&mut position)?;
    let mut frame = Vec::with_capacity(header_raw.len() + rest.len() + midi.len() + position.len());
    frame.extend_from_slice(&header_raw);
    frame.extend_from_slice(&rest);
    frame.extend_from_slice(&midi);
    frame.extend_from_slice(&position);
    Ok(frame)
}

fn read_audio_response_frame(socket: &mut TcpStream, is_double: bool) -> std::io::Result<Vec<u8>> {
    let mut header_raw = [0u8; AudioResponseHeader::WIRE_SIZE];
    socket.read_exact(&mut header_raw)?;
    let header = AudioResponseHeader::from_bytes(&header_raw).ok_or_else(|| invalid("audio response"))?;
    if !audio_shape_ok(header.channels, header.samples, header.midi_events, is_double) {
        return Err(invalid("audio response shape"));
    }
    let width = if is_double { 8 } else { 4 };
    let mut rest = vec![0u8; header.channels as usize * header.samples as usize * width];
    socket.read_exact(&mut rest)?;
    let midi = read_midi_events(socket, header.midi_events)?;
    let mut frame = Vec::with_capacity(header_raw.len() + rest.len() + midi.len());
    frame.extend_from_slice(&header_raw);
    frame.extend_from_slice(&rest);
    frame.extend_from_slice(&midi);
    Ok(frame)
}

fn read_midi_events(socket: &mut TcpStream, count: i32) -> std::io::Result<Vec<u8>> {
    let mut midi = Vec::new();
    for _ in 0..count {
        let mut midi_header = [0u8; 8];
        socket.read_exact(&mut midi_header)?;
        let size = i32::from_le_bytes(midi_header[4..8].try_into().unwrap());
        if !(0..=65_536).contains(&size) {
            return Err(invalid("midi size"));
        }
        let mut data = vec![0u8; size as usize];
        if size > 0 {
            socket.read_exact(&mut data)?;
        }
        midi.extend_from_slice(&midi_header);
        midi.extend_from_slice(&data);
    }
    Ok(midi)
}

/// One blocking client. Tests and the one-socket helpers use this. The
/// accept loop in `serve_clients` is what `main` runs.
pub fn serve_worker(listener: TcpListener) -> std::io::Result<()> {
    crate::cpu::start();
    listener.set_nonblocking(false)?;
    let Some(accepted) = accept_client(&listener)? else {
        return Err(invalid("handshake rejected"));
    };
    run_session(accepted)
}

/// AudioGridder's master accept loop: each handshake starts a worker thread,
/// then the loop accepts the next client without waiting for that worker.
pub fn serve_clients(listener: TcpListener) -> std::io::Result<()> {
    crate::cpu::start();
    listener.set_nonblocking(false)?;
    loop {
        match accept_client(&listener) {
            Ok(Some(accepted)) => {
                thread::spawn(move || {
                    if let Err(err) = run_session(accepted) {
                        if restart_requested(&err) {
                            eprintln!("ag-server: {err}");
                            std::process::exit(1);
                        }
                        eprintln!("ag-server: {err}");
                    }
                });
            }
            Ok(None) => continue,
            Err(err) => return Err(err),
        }
    }
}

fn run_session(accepted: AcceptedClient) -> std::io::Result<()> {
    let AcceptedClient { worker, request: req } = accepted;
    worker.set_nonblocking(false)?;
    let mut sockets = Vec::with_capacity(4);
    for _ in 0..4 {
        let (sock, _) = accept_timeout(&worker)?;
        sock.set_read_timeout(Some(IO_TIMEOUT))?;
        sock.set_write_timeout(Some(IO_TIMEOUT))?;
        sockets.push(sock);
    }
    let mut command = sockets.remove(0);
    let command_in = Arc::new(Mutex::new(sockets.remove(0)));
    let audio = sockets.remove(0);
    let screen = Arc::new(Mutex::new(sockets.remove(0)));
    let audio_shutdown = audio.try_clone()?;

    let chain = Arc::new(Mutex::new(PluginChain::new(req.channels_out)));
    let screen_stop = Arc::new(AtomicBool::new(false));
    let screen_thread = {
        let live = chain.lock().expect("chain").live_editor();
        let stop = Arc::clone(&screen_stop);
        let socket = Arc::clone(&screen);
        let inbound = Arc::clone(&command_in);
        thread::spawn(move || stream_editor(stop, live, socket, inbound))
    };
    let _screen = StopClip { stop: screen_stop, thread: Some(screen_thread) };
    let clip_stop = Arc::new(AtomicBool::new(false));
    let clip_thread = {
        let stop = Arc::clone(&clip_stop);
        let inbound = Arc::clone(&command_in);
        let watched = Arc::clone(&chain);
        thread::spawn(move || watch_clipboard(stop, inbound, watched))
    };
    let _clip = StopClip { stop: clip_stop, thread: Some(clip_thread) };
    {
        let mut chain = chain.lock().expect("chain");
        chain.set_route(req.channels_in, req.channels_sc, req.active_channels);
        chain.set_plugin_list_filter(req.flags);
        chain.prepare(req.sample_rate as f32, req.samples_per_block as usize, req.channels_in as usize);
    }
    let audio_chain = Arc::clone(&chain);
    let audio_thread = thread::spawn(move || audio_loop(audio, audio_chain));

    let client_host = command.peer_addr().map(|addr| addr.ip().to_string()).unwrap_or_default();
    let replies = {
        let mut chain = chain.lock().expect("chain");
        chain.set_client_host(client_host);
        chain.handle(command::PLUGIN_LIST, &[])
    };
    for (reply_type, reply_body) in replies {
        if !write_until_disconnect(&mut command, reply_type, &reply_body)? {
            break;
        }
    }

    let mut quit = false;
    let mut restart: Option<&'static str> = None;
    while !quit && restart.is_none() {
        let (message_type, body) = match read_cmd(&mut command) {
            Ok(message) => message,
            Err(err) if is_wait(err.kind()) => continue,
            Err(err) if client_disconnected(&err) => break,
            Err(err) => return Err(err),
        };
        if message_type == RESCAN {
            let wipe = command::read_number(&body) == Some(1);
            restart = Some(crate::host::rescan(wipe));
            break;
        }
        if message_type == RESTART {
            restart = Some("restart");
            break;
        }
        quit = message_type == QUIT;
        let replies = handle_unlocked(&chain, message_type, &body);
        let mut disconnected = false;
        for (reply_type, reply_body) in replies {
            if !write_until_disconnect(&mut command, reply_type, &reply_body)? {
                disconnected = true;
                break;
            }
        }
        if disconnected {
            break;
        }
        // Encode before the command-in pushes. The client reads those pushes
        // and may close the screen socket as soon as they arrive.
        let screen_frame = chain.lock().expect("chain").take_screen();
        let pushes = chain.lock().expect("chain").take_outbound();
        if let Some((width, height, png)) = screen_frame {
            if !write_screen_until_disconnect(&mut screen.lock().expect("screen"), width, height, &png)? {
                break;
            }
        }
        for (push_type, push_body) in pushes {
            if !write_until_disconnect(&mut command_in.lock().expect("command-in"), push_type, &push_body)? {
                disconnected = true;
                break;
            }
        }
        if disconnected {
            break;
        }
    }
    let _ = command.shutdown(Shutdown::Both);
    let _ = audio_shutdown.shutdown(Shutdown::Both);
    let _ = audio_thread.join();
    if let Some(reason) = restart {
        return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, reason));
    }
    Ok(())
}

enum Samples {
    F32(Vec<Vec<f32>>),
    F64(Vec<Vec<f64>>),
}

fn audio_loop(mut socket: TcpStream, chain: Arc<Mutex<PluginChain>>) {
    loop {
        let (mut samples, midi, position) = match read_audio(&mut socket) {
            Ok(message) => message,
            Err(err) if is_wait(err.kind()) => continue,
            Err(_) => break,
        };
        let step = {
            let mut chain = chain.lock().expect("chain");
            let buffer_channels = match &samples {
                Samples::F32(audio) => audio.len(),
                Samples::F64(audio) => audio.len(),
            };
            if !chain.buffer_covers_active_inputs(buffer_channels) {
                None
            } else {
                let triples = midi_triples(&midi);
                match &mut samples {
                    Samples::F32(audio) => {
                        let frames = audio.first().map(|c| c.len()).unwrap_or(0);
                        chain.process_at(audio, &triples, frames, position);
                    }
                    Samples::F64(audio) => {
                        let frames = audio.first().map(|c| c.len()).unwrap_or(0);
                        chain.process_f64_at(audio, &triples, frames, position);
                    }
                }
                let returned = if chain.plugin_replaced_midi() { Vec::new() } else { midi };
                Some((chain.latency(), returned))
            }
        };
        let Some((latency, midi)) = step else {
            let _ = socket.shutdown(Shutdown::Both);
            break;
        };
        let bytes = match &samples {
            Samples::F32(audio) => match write_audio_response(audio, latency) {
                Some(mut bytes) => {
                    append_response_midi(&mut bytes, &midi);
                    bytes
                }
                None => break,
            },
            Samples::F64(audio) => {
                let mut bytes = write_f64_response(audio, latency);
                append_response_midi(&mut bytes, &midi);
                bytes
            }
        };
        if socket.write_all(&bytes).is_err() {
            break;
        }
    }
}

fn write_f64_response(channels: &[Vec<f64>], latency_samples: i32) -> Vec<u8> {
    let count = channels.len() as i32;
    let samples = channels.first().map(|channel| channel.len()).unwrap_or(0) as i32;
    let header = AudioResponseHeader { channels: count, samples, midi_events: 0, latency_samples };
    let mut out = Vec::with_capacity(AudioResponseHeader::WIRE_SIZE + channels.len() * samples as usize * 8);
    out.extend_from_slice(&header.to_bytes());
    for channel in channels {
        for sample in channel {
            out.extend_from_slice(&sample.to_le_bytes());
        }
    }
    out
}

fn write_screen(socket: &mut TcpStream, width: i32, height: i32, pixels: &[u8]) -> std::io::Result<()> {
    let mut body = Vec::with_capacity(32 + pixels.len());
    body.extend_from_slice(&width.to_le_bytes());
    body.extend_from_slice(&height.to_le_bytes());
    body.extend_from_slice(&width.to_le_bytes());
    body.extend_from_slice(&height.to_le_bytes());
    body.extend_from_slice(&1.0f64.to_le_bytes());
    body.extend_from_slice(&(pixels.len() as u64).to_le_bytes());
    body.extend_from_slice(pixels);
    write_cmd(socket, SCREEN_CAPTURE, &body)
}

/// Commands whose slow part runs on the main thread (creating a plugin, opening its
/// editor) do that part without the chain lock, so the audio thread keeps answering.
/// The client drops the session after 500 ms without audio. Only this thread changes
/// the slots, so the slot looked up before stays valid.
fn handle_unlocked(chain: &Arc<Mutex<PluginChain>>, message_type: i32, body: &[u8]) -> Vec<(i32, Vec<u8>)> {
    match message_type {
        command::EDIT_PLUGIN => {
            let pending = chain.lock().expect("chain").begin_edit(body);
            if let Some((index, target, title, x, y)) = pending {
                let window = crate::ui::open_target(target, &title, x, y);
                chain.lock().expect("chain").finish_edit(index, window);
            }
            Vec::new()
        }
        command::ADD_PLUGIN => {
            let request = chain.lock().expect("chain").load_request(body);
            let instances = request.as_ref().map(PluginChain::instantiate_for);
            let mut chain = chain.lock().expect("chain");
            if let Some(instances) = instances {
                chain.preload(instances);
            }
            chain.handle(message_type, body)
        }
        _ => chain.lock().expect("chain").handle(message_type, body),
    }
}

/// Editor stream, the old OpenGridder server's way (og-server `ScreenWorker`): capture the
/// open editor window up to 60 times a second and send only the rectangle that changed
/// (`RectEncoder`). When the window is closed on the server's screen, push HidePlugin like
/// `Worker::sendHideEditor`.
fn stream_editor(stop: Arc<AtomicBool>, live: Arc<AtomicU64>, socket: Arc<Mutex<TcpStream>>, inbound: Arc<Mutex<TcpStream>>) {
    const FRAME: Duration = Duration::from_micros(16_667);
    let mut encoder = crate::screen::RectEncoder::new();
    let mut current = 0u64;
    let mut pixels = Vec::new();
    // AG_STREAM_STATS=1 logs capture and encode times every 120 loops.
    let stats = std::env::var_os("AG_STREAM_STATS").is_some();
    let (mut loops, mut sent, mut capture_total, mut encode_total) = (0u32, 0u32, Duration::ZERO, Duration::ZERO);
    while !stop.load(Ordering::Relaxed) {
        let started = std::time::Instant::now();
        let id = live.load(Ordering::Acquire);
        if id != current {
            encoder.reset();
            current = id;
        }
        if id == 0 {
            thread::sleep(Duration::from_millis(30));
            continue;
        }
        if !crate::ui::is_open(id) {
            // Closed on the server: the chain cleans up on its next command; tell the client now.
            if live.compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                if let Ok(mut socket) = inbound.lock() {
                    let _ = write_cmd(&mut socket, command::HIDE_PLUGIN, &command::number(0));
                }
            }
            continue;
        }
        let captured = crate::ui::capture(id, &mut pixels);
        let capture_time = started.elapsed();
        if let Some((width, height)) = captured {
            let update = encoder.encode(width, height, &pixels);
            if stats {
                loops += 1;
                capture_total += capture_time;
                encode_total += started.elapsed() - capture_time;
                sent += update.is_some() as u32;
                if loops == 120 {
                    eprintln!(
                        "editor stream: {}x{} capture {:.1} ms, diff+encode {:.1} ms, {} of 120 loops sent",
                        width, height, capture_total.as_secs_f64() * 1000.0 / 120.0, encode_total.as_secs_f64() * 1000.0 / 120.0, sent
                    );
                    (loops, sent, capture_total, encode_total) = (0, 0, Duration::ZERO, Duration::ZERO);
                }
            }
            if let Some(update) = update {
                let Ok(mut socket) = socket.lock() else { break };
                match write_screen_rect(&mut socket, width, height, &update) {
                    Ok(()) => {}
                    Err(err) if client_disconnected(&err) => break,
                    Err(_) => encoder.reset(), // slow reader: send the whole picture next
                }
            }
        }
        if let Some(rest) = FRAME.checked_sub(started.elapsed()) {
            thread::sleep(rest);
        }
    }
}

/// `ScreenCapture` for part of the picture: the header carries the full editor size, the
/// image is the changed rectangle, and an "OGRC" trailer (x, y, w, h as little-endian u32)
/// says where it goes. A client that ignores the trailer reads a normal picture message.
fn write_screen_rect(socket: &mut TcpStream, width: i32, height: i32, update: &crate::screen::RectUpdate) -> std::io::Result<()> {
    let mut body = Vec::with_capacity(52 + update.image.len());
    body.extend_from_slice(&width.to_le_bytes());
    body.extend_from_slice(&height.to_le_bytes());
    body.extend_from_slice(&width.to_le_bytes());
    body.extend_from_slice(&height.to_le_bytes());
    body.extend_from_slice(&1.0f64.to_le_bytes());
    body.extend_from_slice(&(update.image.len() as u64).to_le_bytes());
    body.extend_from_slice(&update.image);
    body.extend_from_slice(b"OGRC");
    for value in [update.x, update.y, update.width, update.height] {
        body.extend_from_slice(&(value as u32).to_le_bytes());
    }
    write_cmd(socket, SCREEN_CAPTURE, &body)
}

struct StopClip {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for StopClip {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// `ClipboardTracker` polls the pasteboard while an editor is open and pushes
/// `Clipboard` on command-in when the text changes. The value already there
/// when the editor opens is the baseline, so opening an editor does not echo it.
fn watch_clipboard(stop: Arc<AtomicBool>, inbound: Arc<Mutex<TcpStream>>, chain: Arc<Mutex<PluginChain>>) {
    let mut last = String::new();
    let mut armed = false;
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(200));
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let open = chain.lock().expect("chain").editor_is_open();
        if !open {
            armed = false;
            continue;
        }
        let Ok(text) = crate::clipboard::get_text() else { continue };
        if !armed {
            last = text;
            armed = true;
            continue;
        }
        if text == last {
            continue;
        }
        last = text.clone();
        let body = command::length_prefixed(text.as_bytes());
        if let Ok(mut socket) = inbound.lock() {
            let _ = write_cmd(&mut socket, CLIPBOARD, &body);
        }
    }
}

fn is_wait(kind: std::io::ErrorKind) -> bool {
    kind == std::io::ErrorKind::TimedOut || kind == std::io::ErrorKind::WouldBlock
}

/// The client closed a socket. That ends the session. It is not a worker failure.
fn client_disconnected(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
    )
}

fn write_until_disconnect(socket: &mut TcpStream, message_type: i32, body: &[u8]) -> std::io::Result<bool> {
    match write_cmd(socket, message_type, body) {
        Ok(()) => Ok(true),
        Err(err) if client_disconnected(&err) => Ok(false),
        Err(err) => Err(err),
    }
}

fn write_screen_until_disconnect(socket: &mut TcpStream, width: i32, height: i32, pixels: &[u8]) -> std::io::Result<bool> {
    match write_screen(socket, width, height, pixels) {
        Ok(()) => Ok(true),
        Err(err) if client_disconnected(&err) => Ok(false),
        Err(err) => Err(err),
    }
}

/// `AudioMessage::readFromClient` sizes the buffer to the greater of the sent
/// shape and the requested shape. The socket still carries only the sent
/// channels and samples, so the extra channel and samples stay 0.
fn grow_to_requested<T: Copy + Default>(audio: &mut Vec<Vec<T>>, header: &AudioRequestHeader) {
    let channels = header.channels.max(header.channels_requested).clamp(header.channels, 2) as usize;
    let samples = header.samples.max(header.samples_requested).clamp(header.samples, 8192) as usize;
    for channel in audio.iter_mut() {
        if channel.len() < samples {
            channel.resize(samples, T::default());
        }
    }
    while audio.len() < channels {
        audio.push(vec![T::default(); samples]);
    }
}

struct WireMidi {
    sample: i32,
    data: Vec<u8>,
}

/// Three-byte events are what `MusicDeviceMIDIEvent` accepts. Shorter events
/// stay on the response when the chain does not replace the buffer.
fn midi_triples(midi: &[WireMidi]) -> Vec<[u8; 3]> {
    midi.iter()
        .filter_map(|event| {
            if event.data.len() < 3 {
                return None;
            }
            let mut data = [0u8; 3];
            data.copy_from_slice(&event.data[..3]);
            Some(data)
        })
        .collect()
}

/// `AudioMessage::sendToClient` writes `numMidiEvents`, then each event as
/// sample position, size, and bytes, after the audio samples.
fn append_response_midi(bytes: &mut Vec<u8>, midi: &[WireMidi]) {
    let count = i32::try_from(midi.len()).unwrap_or(i32::MAX);
    bytes[8..12].copy_from_slice(&count.to_le_bytes());
    for event in midi {
        bytes.extend_from_slice(&event.sample.to_le_bytes());
        let size = i32::try_from(event.data.len()).unwrap_or(0);
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&event.data);
    }
}

fn read_audio(socket: &mut TcpStream) -> std::io::Result<(Samples, Vec<WireMidi>, PositionInfo)> {
    let mut header_raw = [0u8; AudioRequestHeader::WIRE_SIZE];
    socket.read_exact(&mut header_raw)?;
    let header = AudioRequestHeader::from_bytes(&header_raw).ok_or_else(|| invalid("audio header"))?;
    if !audio_shape_ok(header.channels, header.samples, header.midi_events, header.is_double) {
        return Err(invalid("audio shape"));
    }
    let channels = header.channels as usize;
    let samples = header.samples as usize;
    let width = if header.is_double { 8 } else { 4 };
    let mut samples_block = if header.is_double {
        let mut audio = vec![vec![0.0f64; samples]; channels];
        for channel in &mut audio {
            let mut bytes = vec![0u8; samples * width];
            socket.read_exact(&mut bytes)?;
            for (index, sample) in channel.iter_mut().enumerate() {
                let at = index * width;
                *sample = f64::from_le_bytes(bytes[at..at + width].try_into().unwrap());
            }
        }
        Samples::F64(audio)
    } else {
        let mut audio = vec![vec![0.0f32; samples]; channels];
        for channel in &mut audio {
            let mut bytes = vec![0u8; samples * width];
            socket.read_exact(&mut bytes)?;
            for (index, sample) in channel.iter_mut().enumerate() {
                let at = index * width;
                *sample = f32::from_le_bytes(bytes[at..at + width].try_into().unwrap());
            }
        }
        Samples::F32(audio)
    };
    match &mut samples_block {
        Samples::F32(audio) => grow_to_requested(audio, &header),
        Samples::F64(audio) => grow_to_requested(audio, &header),
    }
    let mut midi = Vec::new();
    for _ in 0..header.midi_events {
        let mut midi_header = [0u8; 8];
        socket.read_exact(&mut midi_header)?;
        let size = i32::from_le_bytes(midi_header[4..8].try_into().unwrap());
        if !(0..=65_536).contains(&size) {
            return Err(invalid("midi size"));
        }
        let mut data = vec![0u8; size as usize];
        if size > 0 {
            socket.read_exact(&mut data)?;
        }
        let sample = i32::from_le_bytes(midi_header[0..4].try_into().unwrap());
        midi.push(WireMidi { sample, data });
    }
    let mut position_raw = [0u8; PositionInfo::WIRE_SIZE];
    socket.read_exact(&mut position_raw)?;
    let position = PositionInfo::from_bytes(&position_raw).ok_or_else(|| invalid("position"))?;
    Ok((samples_block, midi, position))
}

fn read_cmd(socket: &mut TcpStream) -> std::io::Result<(i32, Vec<u8>)> {
    let mut header = [0u8; command::CMD_HEADER];
    socket.read_exact(&mut header)?;
    let message_type = i32::from_le_bytes(header[0..4].try_into().unwrap());
    let size = i32::from_le_bytes(header[4..8].try_into().unwrap());
    if size < 0 || size as usize > command::MAX_BODY {
        return Err(invalid("command size"));
    }
    let mut body = vec![0u8; size as usize];
    if size > 0 {
        socket.read_exact(&mut body)?;
    }
    Ok((message_type, body))
}

fn write_cmd(socket: &mut TcpStream, message_type: i32, body: &[u8]) -> std::io::Result<()> {
    socket.write_all(&command::encode(message_type, body))
}

fn accept_timeout(listener: &TcpListener) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    listener.set_nonblocking(true)?;
    let start = std::time::Instant::now();
    loop {
        match listener.accept() {
            Ok((sock, addr)) => {
                listener.set_nonblocking(false)?;
                // A nonblocking listener hands out a nonblocking socket. A read
                // then returns EAGAIN before the client has written, and the
                // command loop used to treat that as a disconnect.
                sock.set_nonblocking(false)?;
                return Ok((sock, addr));
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > IO_TIMEOUT {
                    return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "worker accept"));
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(err) => return Err(err),
        }
    }
}

fn invalid(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_protocol::command::{self, ADD_PLUGIN, ADD_PLUGIN_RESULT, DEL_PLUGIN, PARAMETER_VALUE, PARAMETERS, PLUGIN_LIST, PRESETS, QUIT, RESULT};
    use ag_protocol::{read_audio_response, write_audio_request, HandshakeRequest, HandshakeResponse, PositionInfo, PROTOCOL_VERSION};
    use crate::chain::BUILTIN_GAIN_ID;

    fn handshake(port: u16) -> (TcpStream, u16) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        stream.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
        let request = HandshakeRequest { samples_per_block: 32, ..HandshakeRequest::default() };
        stream.write_all(&request.to_bytes()).unwrap();
        let mut raw = [0u8; HandshakeResponse::WIRE_SIZE];
        stream.read_exact(&mut raw).unwrap();
        let answer = HandshakeResponse::from_bytes(&raw).unwrap();
        assert_eq!(answer.version, PROTOCOL_VERSION);
        assert_ne!(answer.port as u16, port);
        (stream, answer.port as u16)
    }

    struct OpenClient {
        port: u16,
        command: TcpStream,
        inbound: TcpStream,
        audio: TcpStream,
        screen: TcpStream,
    }

    fn open_client(server_port: u16) -> OpenClient {
        let (handshake_sock, worker_port) = handshake(server_port);
        drop(handshake_sock);
        let mut sockets = Vec::new();
        for _ in 0..4 {
            let sock = TcpStream::connect(("127.0.0.1", worker_port)).unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
            sock.set_write_timeout(Some(Duration::from_secs(30))).unwrap();
            sockets.push(sock);
        }
        let mut command = sockets.remove(0);
        let inbound = sockets.remove(0);
        let audio = sockets.remove(0);
        let screen = sockets.remove(0);
        let (ty, body) = read_cmd(&mut command).unwrap();
        assert_eq!(ty, PLUGIN_LIST, "worker did not push the plugin list on connect");
        let packed = command::read_length_prefixed(&body).unwrap();
        assert!(!packed.is_empty(), "connect-time plugin list was empty");
        OpenClient { port: worker_port, command, inbound, audio, screen }
    }

    fn add_gain(client: &mut OpenClient, settings: &str) {
        let add = command::json_payload(&serde_json::json!({"id": BUILTIN_GAIN_ID, "settings": settings}));
        write_cmd(&mut client.command, ADD_PLUGIN, &add).unwrap();
        let (ty, body) = read_cmd(&mut client.command).unwrap();
        assert_eq!(ty, ADD_PLUGIN_RESULT);
        let result = command::read_json_payload(&body).unwrap();
        assert_eq!(result["success"], true);
        let (ty, _) = read_cmd(&mut client.command).unwrap();
        assert_eq!(ty, PRESETS);
        let (ty, _) = read_cmd(&mut client.command).unwrap();
        assert_eq!(ty, PARAMETERS);
    }

    fn first_sample(client: &mut OpenClient) -> f32 {
        let block = write_audio_request(&[vec![1.0; 32], vec![1.0; 32]], &PositionInfo::default()).unwrap();
        client.audio.write_all(&block).unwrap();
        let mut response = vec![0u8; 16 + 2 * 32 * 4];
        client.audio.read_exact(&mut response).unwrap();
        let (_header, audio) = read_audio_response(&response).unwrap();
        audio[0][0]
    }

    #[test]
    fn accept_loop_hosts_a_second_client_while_the_first_worker_is_open() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || serve_clients(listener));

        let mut first = open_client(port);
        let mut second = open_client(port);
        assert_ne!(first.port, second.port, "each handshake needs its own worker port");
        eprintln!("concurrent workers {} {}", first.port, second.port);

        add_gain(&mut first, "-6");
        add_gain(&mut second, "6");
        let low = first_sample(&mut first);
        let high = first_sample(&mut second);
        eprintln!("first audio {low}");
        eprintln!("second audio {high}");
        assert!((low - 0.5011872).abs() < 1.0e-5, "first worker was not -6 dB, got {low}");
        assert!((high - 1.9952624).abs() < 1.0e-5, "second worker was not +6 dB, got {high}");

        write_cmd(&mut first.command, QUIT, &[]).unwrap();
        write_cmd(&mut second.command, QUIT, &[]).unwrap();
        for client in [&first, &second] {
            let _ = client.command.shutdown(Shutdown::Both);
            let _ = client.inbound.shutdown(Shutdown::Both);
            let _ = client.audio.shutdown(Shutdown::Both);
            let _ = client.screen.shutdown(Shutdown::Both);
        }
    }

    #[test]
    fn four_sockets_add_a_gain_and_process_audio() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_worker(listener));

        let (handshake_sock, worker_port) = handshake(port);
        drop(handshake_sock);
        let mut sockets = Vec::new();
        for _ in 0..4 {
            let sock = TcpStream::connect(("127.0.0.1", worker_port)).unwrap();
            sock.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
            sock.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
            sockets.push(sock);
        }
        let mut command = sockets.remove(0);
        command.set_read_timeout(Some(Duration::from_secs(30))).unwrap();

        let (ty, body) = read_cmd(&mut command).unwrap();
        assert_eq!(ty, PLUGIN_LIST, "worker did not push the plugin list on connect");
        let packed = command::read_length_prefixed(&body).unwrap();
        assert!(!packed.is_empty(), "connect-time plugin list was empty");

        write_cmd(&mut command, PLUGIN_LIST, &[]).unwrap();
        let (ty, body) = read_cmd(&mut command).unwrap();
        assert_eq!(ty, PLUGIN_LIST);
        let packed = command::read_length_prefixed(&body).unwrap();
        assert!(!packed.is_empty());

        let add = command::json_payload(&serde_json::json!({"id": BUILTIN_GAIN_ID, "settings": "-6"}));
        write_cmd(&mut command, ADD_PLUGIN, &add).unwrap();
        let (ty, body) = read_cmd(&mut command).unwrap();
        assert_eq!(ty, ADD_PLUGIN_RESULT);
        let result = command::read_json_payload(&body).unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["name"], "AudioGridder Host Gain");
        let (ty, _) = read_cmd(&mut command).unwrap();
        assert_eq!(ty, PRESETS);
        let (ty, _) = read_cmd(&mut command).unwrap();
        assert_eq!(ty, PARAMETERS);

        write_cmd(&mut command, PARAMETER_VALUE, &command::parameter_value(0, 0, 0.5, 0)).unwrap();

        let block = write_audio_request(&[vec![1.0; 32], vec![1.0; 32]], &PositionInfo::default()).unwrap();
        sockets[1].write_all(&block).unwrap();
        let mut response = vec![0u8; 16 + 2 * 32 * 4];
        sockets[1].read_exact(&mut response).unwrap();
        let (_header, audio) = read_audio_response(&response).unwrap();
        assert!((audio[0][0] - 1.0).abs() < 1.0e-5, "unity after normalized 0.5, got {}", audio[0][0]);

        write_cmd(&mut command, DEL_PLUGIN, &command::number(0)).unwrap();
        let (ty, body) = read_cmd(&mut command).unwrap();
        assert_eq!(ty, RESULT);
        assert_eq!(command::read_result(&body).unwrap().0, 0);

        drop(command);
        drop(sockets);
        server.join().unwrap().unwrap();
    }
}
