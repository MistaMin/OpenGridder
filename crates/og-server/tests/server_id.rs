//! Server ID offsets the listen port. ID 0 stays on the AudioGridder port.
//! This test drives the binary.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use og_protocol::{HandshakeRequest, HandshakeResponse, SERVER_PORT};

struct Kill(Option<Child>);

impl Drop for Kill {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn connect_for(port: u16) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)) {
            return stream;
        }
        if Instant::now() >= deadline {
            panic!("server id did not listen on {port}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn server_id_offsets_the_listen_port() {
    let bin = env!("CARGO_BIN_EXE_og-server");
    let id: u16 = 3;
    let port = SERVER_PORT + id;
    let already_bound = std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{SERVER_PORT}").parse().unwrap(),
        Duration::from_millis(50),
    ).is_ok();
    let child = Command::new(bin)
        .args(["--id", "3"])
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("og-server");
    let kill = Kill(Some(child));
    let mut stream = connect_for(port);
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.write_all(&HandshakeRequest::default().to_bytes()).unwrap();
    let mut answer = [0u8; HandshakeResponse::WIRE_SIZE];
    stream.read_exact(&mut answer).unwrap();
    let response = HandshakeResponse::from_bytes(&answer).unwrap();
    eprintln!("id {id} version {} flags {} port {}", response.version, response.flags, response.port);
    assert_eq!(response.version, 13);
    assert_eq!(response.flags, 0);
    assert_ne!(response.port, i32::from(port), "worker port collided with the master");
    if !already_bound {
        let default_port = std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{SERVER_PORT}").parse().unwrap(),
            Duration::from_millis(200),
        );
        assert!(default_port.is_err(), "id 3 also bound {SERVER_PORT}");
    }
    drop(stream);
    drop(kill);
}
