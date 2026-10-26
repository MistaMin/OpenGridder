#![cfg(target_os = "macos")]
use og_client::{Client, ClientConfig};
use og_common::{message::PositionInfo, server_plugin::ServerPlugin};
use og_server::{SandboxMode, Server, ServerConfig};
use std::time::Instant;

#[test]
fn isolated_au_chain_crash_preserves_other_client_and_accepts_reconnect() {
    let mut cfg = ServerConfig::default();
    cfg.sandbox_mode = SandboxMode::Chain as i32;
    let mut server = Server::new(cfg);
    server.set_worker_executable(env!("CARGO_BIN_EXE_OpenGridderServer").into());
    let desc = ServerPlugin::new(
        "AULowpass",
        "Apple",
        "AudioUnit:aufx:lpas:appl",
        "AULowpass",
        "AudioUnit",
        "Filter",
        false,
        vec!["Stereo".into()],
    );
    server.add_known_plugin(desc.clone());
    let port = server.start("127.0.0.1", 0).unwrap();
    let connect = || {
        let client = Client::new(ClientConfig {
            port,
            samples_per_block: 1024,
            ..Default::default()
        });
        client.connect().unwrap();
        assert!(client.is_sandbox_enabled());
        client.add_plugin(&desc.id, "Stereo", 0).unwrap();
        client
    };
    let first = connect();
    let second = connect();
    first.set_parameter_value(0, 0, 0.95).unwrap();
    second.set_parameter_value(0, 0, 0.1).unwrap();
    let state = first.get_plugin_settings(0).unwrap();
    assert!(!state.is_empty());
    let pids = server.worker_process_ids();
    assert_eq!(pids.len(), 2);
    assert_ne!(pids[0], pids[1]);
    // Kill only this test's first child, simulating a native plugin crash.
    assert_eq!(unsafe { libc::kill(pids[0] as i32, libc::SIGKILL) }, 0);
    let mut audio = vec![vec![0.1; 64], vec![-0.1; 64]];
    let mut midi = Vec::new();
    assert!(first
        .process_audio_f32(&mut audio, &mut midi, &PositionInfo::default())
        .is_err());
    second
        .process_audio_f32(&mut audio, &mut midi, &PositionInfo::default())
        .unwrap();
    assert!((second.get_parameter_value(0, 0).unwrap() - 0.1).abs() < 1e-5);
    first.disconnect();
    first.connect().unwrap();
    first.add_plugin(&desc.id, "Stereo", 0).unwrap();
    first.set_plugin_settings(0, &state).unwrap();
    assert!((first.get_parameter_value(0, 0).unwrap() - 0.95).abs() < 1e-5);
    // Measure a real stereo AU processing round trip after warmup. No scheduling
    // threshold assertion: CI/desktop contention must not make this test flaky.
    let mut times = Vec::new();
    for index in 0..220 {
        audio[0].fill(0.1);
        audio[1].fill(-0.1);
        let start = Instant::now();
        first
            .process_audio_f32(&mut audio, &mut midi, &PositionInfo::default())
            .unwrap();
        if index >= 20 {
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
    }
    times.sort_by(f64::total_cmp);
    eprintln!("Real AU / isolated worker / stereo / 48 kHz / 64 samples: loopback RTT median={:.3} ms p95={:.3} ms p99={:.3} ms max={:.3} ms (200 measured blocks)", times[100],times[190],times[198],times[199]);
    first.disconnect();
    second.disconnect();
    server.stop();
    assert!(server.worker_process_ids().is_empty());
}
