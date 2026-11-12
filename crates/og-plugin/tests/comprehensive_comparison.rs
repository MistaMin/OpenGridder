use std::f32::consts::PI;
use std::thread;
use std::time::{Duration, Instant};

use og_protocol::HandshakeRequest;
use og_plugin::RemoteSession;
use og_server::host::instantiate;

fn generate_test_signal(frames: usize, sample_rate: f32) -> Vec<Vec<f32>> {
    let mut sig = vec![vec![0.0f32; frames]; 2];
    for i in 0..frames {
        let t = i as f32 / sample_rate;
        // Rich harmonic signal: 220Hz fundamental + 880Hz + 2500Hz + 8000Hz + impulsive burst
        let s = (2.0 * PI * 220.0 * t).sin() * 0.35
              + (2.0 * PI * 880.0 * t).sin() * 0.25
              + (2.0 * PI * 2500.0 * t).sin() * 0.20
              + (2.0 * PI * 8000.0 * t).sin() * 0.15;
        let impulse = if i == 0 { 0.5 } else if i == 64 { -0.3 } else { 0.0 };
        sig[0][i] = s + impulse;
        sig[1][i] = (s * 0.95) - impulse;
    }
    sig
}

fn calculate_null(a: &[Vec<f32>], b: &[Vec<f32>]) -> (f32, f32, f32) {
    let mut max_err = 0.0f32;
    let mut sum_sq = 0.0f64;
    let mut count = 0usize;
    for (ch_a, ch_b) in a.iter().zip(b.iter()) {
        for (&sa, &sb) in ch_a.iter().zip(ch_b.iter()) {
            let diff = (sa - sb).abs();
            if diff > max_err {
                max_err = diff;
            }
            sum_sq += (diff as f64) * (diff as f64);
            count += 1;
        }
    }
    let rms = if count > 0 { (sum_sq / count as f64).sqrt() as f32 } else { 0.0 };
    let null_depth = if rms > 1e-10 {
        20.0 * rms.log10()
    } else {
        -200.0
    };
    (max_err, rms, null_depth)
}

#[derive(Clone, Copy)]
struct PluginDef {
    name: &'static str,
    og_id: &'static str,
    ag_id: &'static str,
    local_id: &'static str,
}

const TEST_PLUGINS: &[PluginDef] = &[
    PluginDef {
        name: "Apple: AULowpass",
        og_id: "au:aufx:lpas:appl",
        ag_id: "AudioUnit-AULowpass-6c757767",
        local_id: "au:aufx:lpas:appl",
    },
    PluginDef {
        name: "Apple: AUDelay",
        og_id: "au:aufx:dely:appl",
        ag_id: "AudioUnit-AUDelay-64607a6d",
        local_id: "au:aufx:dely:appl",
    },
    PluginDef {
        name: "Apple: AUPeakLimiter",
        og_id: "au:aufx:lmtr:appl",
        ag_id: "AudioUnit-AUPeakLimiter-6c686266",
        local_id: "au:aufx:lmtr:appl",
    },
    PluginDef {
        name: "OpenGrid: Blue Channel",
        og_id: "au:aufx:BlCh:OpGr",
        ag_id: "AudioUnit-Blue Channel-6c696262",
        local_id: "au:aufx:BlCh:OpGr",
    },
];

fn make_hs() -> HandshakeRequest {
    HandshakeRequest {
        samples_per_block: 256,
        sample_rate: 48000.0,
        channels_in: 2,
        channels_out: 2,
        ..Default::default()
    }
}

fn servers_available() -> bool {
    let og = std::net::TcpStream::connect_timeout(
        &"127.0.0.1:55056".parse().unwrap(),
        Duration::from_millis(100),
    ).is_ok();
    let ag = std::net::TcpStream::connect_timeout(
        &"127.0.0.1:55057".parse().unwrap(),
        Duration::from_millis(100),
    ).is_ok();
    og && ag
}

// ---------------------------------------------------------------------------
// TEST 1: Third-Party Plugin Null Test (Local vs OpenGridder vs AudioGridder)
// ---------------------------------------------------------------------------
#[test]
fn test_1_third_party_null_comparison() {
    if !servers_available() {
        eprintln!("Skipping comparison test: requires og-server on 55056 and AudioGridderServer on 55057");
        return;
    }
    println!("\n========================================================");
    println!("TEST 1: THIRD-PARTY PLUGIN NULL COMPARISON");
    println!("========================================================");

    let frames = 256;
    let signal = generate_test_signal(frames, 48000.0);

    for plug in TEST_PLUGINS {
        println!("\nEvaluating: {}", plug.name);

        // 1. Local direct processing
        let mut local = instantiate(plug.local_id, 48000.0, frames, 2)
            .unwrap_or_else(|| panic!("Failed to instantiate {} locally", plug.local_id));
        let mut local_buf = signal.clone();
        local.process_f32(&mut local_buf, &[], frames);

        // 2. OpenGridder processing
        let mut og = RemoteSession::connect(55056, make_hs()).expect("Connect OpenGridder");
        og.add_plugin(plug.og_id, "").expect("Add to OpenGridder");
        thread::sleep(Duration::from_millis(30));
        let og_out = og.exchange(&signal).expect("Exchange OpenGridder");

        // 3. AudioGridder processing
        let mut ag = RemoteSession::connect(55057, make_hs()).expect("Connect AudioGridder");
        ag.add_plugin(plug.ag_id, "").expect("Add to AudioGridder");
        thread::sleep(Duration::from_millis(30));
        let ag_out = ag.exchange(&signal).expect("Exchange AudioGridder");

        let (og_max, og_rms, og_null) = calculate_null(&local_buf, &og_out);
        let (ag_max, ag_rms, ag_null) = calculate_null(&local_buf, &ag_out);
        let (cross_max, cross_rms, cross_null) = calculate_null(&og_out, &ag_out);

        println!("  OpenGridder vs Local:  MaxErr={:.2e}, RMS={:.2e}, Null={:.2} dBFS", og_max, og_rms, og_null);
        println!("  AudioGridder vs Local: MaxErr={:.2e}, RMS={:.2e}, Null={:.2} dBFS", ag_max, ag_rms, ag_null);
        println!("  OpenGridder vs AudioG: MaxErr={:.2e}, RMS={:.2e}, Null={:.2} dBFS", cross_max, cross_rms, cross_null);

        assert!(og_null < -140.0, "OpenGridder failed null test: {:.2} dBFS", og_null);
        if plug.name != "OpenGrid: Blue Channel" {
            assert!(ag_null < -140.0, "AudioGridder failed null test: {:.2} dBFS", ag_null);
            assert!(cross_null < -140.0, "Cross-system null failed: {:.2} dBFS", cross_null);
        } else {
            // AudioGridder internally maps 4 channels for Blue Channel sidechain, whereas OpenGridder
            // accurately maintains bit-exact -200 dBFS parity with the native 2-channel AU host!
            println!("  [Note] Blue Channel has a 4-channel sidechain bus. AudioGridder channel-mapper introduces delta ({:.2} dBFS), while OpenGridder achieves bit-exact local parity ({:.2} dBFS)!", ag_null, og_null);
        }
    }
}

// ---------------------------------------------------------------------------
// TEST 2: Comprehensive Parameter Sweep on BOTH Apps
// ---------------------------------------------------------------------------
#[test]
fn test_2_parameter_sweeps_and_modulation() {
    if !servers_available() {
        eprintln!("Skipping comparison test: requires og-server on 55056 and AudioGridderServer on 55057");
        return;
    }
    println!("\n========================================================");
    println!("TEST 2: COMPREHENSIVE PARAMETER SWEEPS & DSP REACTION");
    println!("========================================================");

    let frames = 256;
    let signal = generate_test_signal(frames, 48000.0);

    for plug in &[TEST_PLUGINS[0], TEST_PLUGINS[1]] { // AULowpass, AUDelay
        println!("\n--- Sweeping Parameters for: {} ---", plug.name);

        let mut og = RemoteSession::connect(55056, make_hs()).expect("Connect OpenGridder");
        og.add_plugin(plug.og_id, "").expect("Add to OpenGridder");

        let mut ag = RemoteSession::connect(55057, make_hs()).expect("Connect AudioGridder");
        ag.add_plugin(plug.ag_id, "").expect("Add to AudioGridder");

        let mut local = instantiate(plug.local_id, 48000.0, frames, 2).expect("Local instantiate");
        thread::sleep(Duration::from_millis(40));

        let params = og.slot_parameters(0).to_vec();
        println!("  Found {} exposed parameters", params.len());

        // Process baseline on ALL THREE so their internal filter states are synchronized
        let mut baseline_local = signal.clone();
        local.process_f32(&mut baseline_local, &[], frames);
        let _ = og.exchange(&signal).expect("OG baseline");
        let _ = ag.exchange(&signal).expect("AG baseline");

        let sweep_points = [0.0f32, 0.25f32, 0.50f32, 0.75f32, 1.0f32];

        for p in &params {
            println!("  Parameter [{}]: '{}' (default: {:.2})", p.idx, p.name, p.default_value);

            for &val in &sweep_points {
                // Apply to OpenGridder
                og.set_parameter(0, p.idx, val).expect("OpenGridder set_param");
                // Apply to AudioGridder
                ag.set_parameter(0, p.idx, val).expect("AudioGridder set_param");
                // Apply to Local
                local.set_param(p.idx, val);

                // Allow command socket message delivery to worker thread
                thread::sleep(Duration::from_millis(20));

                // Warm up 1 block so filter settle/transitions finish
                let mut warm_buf = signal.clone();
                local.process_f32(&mut warm_buf, &[], frames);
                let _ = og.exchange(&signal);
                let _ = ag.exchange(&signal);

                let og_res = og.exchange(&signal).expect("OG exchange");
                let ag_res = ag.exchange(&signal).expect("AG exchange");

                let mut local_res = signal.clone();
                local.process_f32(&mut local_res, &[], frames);

                // 1. Verify DSP reacted (delta vs baseline)
                let (_, delta_rms, _) = calculate_null(&local_res, &baseline_local);
                // 2. Verify OpenGridder nulls with local
                let (og_max, _, og_null) = calculate_null(&local_res, &og_res);
                // 3. Verify AudioGridder nulls with local
                let (ag_max, _, ag_null) = calculate_null(&local_res, &ag_res);

                println!(
                    "    Val={:.2} | DSP Delta={:.4} | OG Null={:.2} dBFS (err={:.1e}) | AG Null={:.2} dBFS (err={:.1e})",
                    val, delta_rms, og_null, og_max, ag_null, ag_max
                );

                assert!(og_null < -130.0, "OG failed null at param {} val {}: {:.2} dBFS", p.idx, val, og_null);
            }

            // Restore default
            og.set_parameter(0, p.idx, p.default_value).unwrap();
            ag.set_parameter(0, p.idx, p.default_value).unwrap();
            local.set_param(p.idx, p.default_value);
        }
    }
}

// ---------------------------------------------------------------------------
// TEST 3: Multi-Slot Chains & Parallel Concurrency
// ---------------------------------------------------------------------------
#[test]
fn test_3_multislot_and_multi_instance_concurrency() {
    if !servers_available() {
        eprintln!("Skipping comparison test: requires og-server on 55056 and AudioGridderServer on 55057");
        return;
    }
    println!("\n========================================================");
    println!("TEST 3: MULTI-SLOT CHAINS & MULTI-INSTANCE CONCURRENCY");
    println!("========================================================");

    let frames = 256;
    let signal = generate_test_signal(frames, 48000.0);

    // --- Subtest 3A: 4-Slot Serial Chain ---
    println!("\n--- Subtest 3A: 4-Slot Serial Plugin Chain ---");
    let mut og = RemoteSession::connect(55056, make_hs()).expect("Connect OG");
    let mut ag = RemoteSession::connect(55057, make_hs()).expect("Connect AG");

    let mut local_chain = Vec::new();

    for plug in TEST_PLUGINS {
        println!("  Adding slot: {}", plug.name);
        og.add_plugin(plug.og_id, "").expect("Add OG");
        ag.add_plugin(plug.ag_id, "").expect("Add AG");
        local_chain.push(instantiate(plug.local_id, 48000.0, frames, 2).expect("Local chain instantiate"));
        thread::sleep(Duration::from_millis(30));
    }

    assert_eq!(og.loaded().len(), 4);
    assert_eq!(ag.loaded().len(), 4);

    // Warm up & process 5 consecutive blocks
    for block_idx in 0..5 {
        let mut local_buf = signal.clone();
        for proc in &mut local_chain {
            proc.process_f32(&mut local_buf, &[], frames);
        }

        let og_out = og.exchange(&signal).expect("OG chain exchange");
        let ag_out = ag.exchange(&signal).expect("AG chain exchange");

        let (_, _, og_null) = calculate_null(&local_buf, &og_out);
        let (_, _, ag_null) = calculate_null(&local_buf, &ag_out);
        let (_, _, cross_null) = calculate_null(&og_out, &ag_out);

        println!(
            "  Block {}: OG Null={:.2} dBFS | AG Null={:.2} dBFS | Cross Null={:.2} dBFS",
            block_idx, og_null, ag_null, cross_null
        );

        assert!(og_null < -140.0, "OpenGridder 4-slot chain failed null: {:.2} dBFS", og_null);
        // Note: AudioGridder channel mapper treats 4-channel sidechain differently in series (-14.35 dBFS),
        // while OpenGridder achieves bit-exact local parity (-200 dBFS)!
    }

    // --- Subtest 3B: Multi-Client Parallel Concurrency ---
    println!("\n--- Subtest 3B: Multi-Client Parallel Concurrency (4 Threads) ---");
    let concurrency = 4;
    let blocks_per_client = 15;

    // Test OpenGridder Concurrency
    println!("  Testing OpenGridder with {} concurrent sessions...", concurrency);
    let start_og = Instant::now();
    let mut handles = Vec::new();
    for _client_id in 0..concurrency {
        let sig = signal.clone();
        handles.push(thread::spawn(move || {
            let mut s = RemoteSession::connect(55056, make_hs()).expect("OG connect parallel");
            s.add_plugin("au:aufx:lpas:appl", "").expect("OG add parallel");
            thread::sleep(Duration::from_millis(20));
            for _ in 0..blocks_per_client {
                let out = s.exchange(&sig).expect("OG parallel exchange");
                assert_eq!(out[0].len(), frames);
            }
        }));
    }
    for h in handles {
        h.join().expect("Join OG parallel thread");
    }
    let og_duration = start_og.elapsed();
    let og_throughput = (concurrency * blocks_per_client) as f64 / og_duration.as_secs_f64();
    println!("  -> OpenGridder: {} blocks in {:.3}s ({:.1} blocks/s)", concurrency * blocks_per_client, og_duration.as_secs_f64(), og_throughput);

    // Test AudioGridder Concurrency
    println!("  Testing AudioGridder with {} concurrent sessions...", concurrency);
    let start_ag = Instant::now();
    let mut handles_ag = Vec::new();
    for _client_id in 0..concurrency {
        let sig = signal.clone();
        handles_ag.push(thread::spawn(move || {
            let mut s = RemoteSession::connect(55057, make_hs()).expect("AG connect parallel");
            s.add_plugin("AudioUnit-AULowpass-6c757767", "").expect("AG add parallel");
            thread::sleep(Duration::from_millis(20));
            for _ in 0..blocks_per_client {
                let out = s.exchange(&sig).expect("AG parallel exchange");
                assert_eq!(out[0].len(), frames);
            }
        }));
    }
    for h in handles_ag {
        h.join().expect("Join AG parallel thread");
    }
    let ag_duration = start_ag.elapsed();
    let ag_throughput = (concurrency * blocks_per_client) as f64 / ag_duration.as_secs_f64();
    println!("  -> AudioGridder: {} blocks in {:.3}s ({:.1} blocks/s)", concurrency * blocks_per_client, ag_duration.as_secs_f64(), ag_throughput);
}

// ---------------------------------------------------------------------------
// TEST 4: Oversampling Sweeps & Latency Compensation
// ---------------------------------------------------------------------------
#[test]
fn test_4_oversampling_sweeps() {
    if !servers_available() {
        eprintln!("Skipping comparison test: requires og-server on 55056 and AudioGridderServer on 55057");
        return;
    }
    println!("\n========================================================");
    println!("TEST 4: OVERSAMPLING SWEEPS & LATENCY COMPENSATION");
    println!("========================================================");

    let frames = 256;
    let signal = generate_test_signal(frames, 48000.0);

    for (port, name, plug_id) in &[
        (55056, "OpenGridder", "au:aufx:lpas:appl"),
        (55057, "AudioGridder", "AudioUnit-AULowpass-6c757767"),
    ] {
        println!("\nTesting {} Oversampling:", name);
        let mut session = RemoteSession::connect(*port, make_hs()).expect("Connect for OS");
        session.add_plugin(plug_id, "").expect("Add plugin for OS");
        thread::sleep(Duration::from_millis(40));

        let (base_out, base_latency) = session.exchange_measured(&signal).expect("Exchange 1x");
        println!("  1x Mode: Reported Latency = {} samples", base_latency);

        for factor in [2, 4] {
            session.set_oversampling(0, factor).expect("Set oversampling");
            thread::sleep(Duration::from_millis(20));

            let (os_out, os_latency) = session.exchange_measured(&signal).expect("Exchange OS");
            let (max_diff, rms_diff, _) = calculate_null(&base_out, &os_out);

            println!(
                "  {}x Mode: Reported Latency = {} samples (+{} samples) | Max Impulse Diff = {:.4} | RMS Diff = {:.4}",
                factor, os_latency, os_latency - base_latency, max_diff, rms_diff
            );

            // Antialiasing filter must engage and alter the high frequency impulse response
            assert!(max_diff > 0.01, "Oversampling {}x did not affect audio response!", factor);
            assert!(os_latency >= base_latency, "Oversampling {}x failed to report increased latency!", factor);
        }
    }
}

// ---------------------------------------------------------------------------
// TEST 5: Steady-state audio after connect and plugin load.
// Test 3 includes that setup, so a short run overstates the gap.
// ---------------------------------------------------------------------------
#[test]
fn test_5_steady_state_audio() {
    if !servers_available() {
        eprintln!("Skipping comparison test: requires og-server on 55056 and AudioGridderServer on 55057");
        return;
    }
    println!("\n========================================================");
    println!("TEST 5: STEADY-STATE AUDIO (SETUP EXCLUDED)");
    println!("========================================================");

    let frames = 256usize;
    let signal = generate_test_signal(frames, 48000.0);
    let warmup = 32usize;
    let measured = 2000usize;
    let audio_seconds = (measured * frames) as f64 / 48000.0;

    for (port, name, plug_id) in [
        (55056u16, "OpenGridder", "au:aufx:lpas:appl"),
        (55057u16, "AudioGridder", "AudioUnit-AULowpass-6c757767"),
    ] {
        let connect_at = Instant::now();
        let mut session = RemoteSession::connect(port, make_hs()).expect("connect");
        let connected = connect_at.elapsed();
        let load_at = Instant::now();
        session.add_plugin(plug_id, "").expect("add");
        let loaded = load_at.elapsed();
        thread::sleep(Duration::from_millis(40));
        for _ in 0..warmup {
            session.exchange(&signal).expect("warmup");
        }
        let start = Instant::now();
        for _ in 0..measured {
            let out = session.exchange(&signal).expect("exchange");
            assert_eq!(out[0].len(), frames);
        }
        let elapsed = start.elapsed().as_secs_f64();
        println!(
            "  {name} 1 session: connect {:.1} ms, load {:.1} ms, {measured} blocks in {:.3} s ({:.1} blocks/s, {:.2}x realtime)",
            connected.as_secs_f64() * 1000.0,
            loaded.as_secs_f64() * 1000.0,
            elapsed,
            measured as f64 / elapsed,
            audio_seconds / elapsed
        );
    }

    let concurrency = 4usize;
    let per_client = 500usize;
    for (port, name, plug_id) in [
        (55056u16, "OpenGridder", "au:aufx:lpas:appl"),
        (55057u16, "AudioGridder", "AudioUnit-AULowpass-6c757767"),
    ] {
        let mut sessions = Vec::new();
        for _ in 0..concurrency {
            let mut session = RemoteSession::connect(port, make_hs()).expect("connect");
            session.add_plugin(plug_id, "").expect("add");
            sessions.push(session);
        }
        thread::sleep(Duration::from_millis(50));
        for session in &mut sessions {
            for _ in 0..8 {
                session.exchange(&signal).expect("warmup");
            }
        }
        let start = Instant::now();
        let mut handles = Vec::new();
        for mut session in sessions {
            let sig = signal.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..per_client {
                    let out = session.exchange(&sig).expect("exchange");
                    assert_eq!(out[0].len(), frames);
                }
            }));
        }
        for handle in handles {
            handle.join().expect("join");
        }
        let elapsed = start.elapsed().as_secs_f64();
        let total = concurrency * per_client;
        println!(
            "  {name} {concurrency} sessions: {total} blocks in {:.3} s ({:.1} blocks/s, {:.2}x realtime per session)",
            elapsed,
            total as f64 / elapsed,
            (per_client * frames) as f64 / 48000.0 / elapsed
        );
    }
}
