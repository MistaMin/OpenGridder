use std::thread;
use std::time::Duration;
use ag_plugin::RemoteSession;
use ag_protocol::HandshakeRequest;
use ag_server::host::instantiate;

fn make_hs() -> HandshakeRequest {
    HandshakeRequest {
        samples_per_block: 256,
        sample_rate: 48000.0,
        channels_in: 2,
        channels_out: 2,
        ..Default::default()
    }
}

fn acustica_available() -> bool {
    let server_ok = std::net::TcpStream::connect_timeout(
        &"127.0.0.1:55056".parse().unwrap(),
        Duration::from_millis(100),
    ).is_ok();
    let plugins_ok = std::path::Path::new("/Library/Audio/Plug-Ins/VST3/WHITE3ZL.vst3").exists();
    server_ok && plugins_ok
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

fn make_rich_signal(frames: usize) -> Vec<Vec<f32>> {
    let mut sig = vec![vec![0.0f32; frames]; 2];
    for i in 0..frames {
        let t = i as f32 / 48000.0;
        let s = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.4
              + (2.0 * std::f32::consts::PI * 1200.0 * t).sin() * 0.2;
        sig[0][i] = s;
        sig[1][i] = s * 0.9;
    }
    sig
}

#[test]
fn test_acustica_null_parity() {
    if !acustica_available() {
        eprintln!("Skipping test: requires ag-server on 55056 and Acustica Audio VST3 plugins");
        return;
    }
    let plugins = [
        ("Acustica Audio: WHITE3ZL", "vst3:/Library/Audio/Plug-Ins/VST3/WHITE3ZL.vst3"),
        ("Acustica Audio: AMBER4ZL", "vst3:/Library/Audio/Plug-Ins/VST3/AMBER4ZL.vst3"),
        ("Acustica Audio: WHITE3", "vst3:/Library/Audio/Plug-Ins/VST3/WHITE3.vst3"),
    ];

    let frames = 256;
    let sig = make_rich_signal(frames);

    for (name, plug_id) in plugins {
        println!("\n=======================================================");
        println!("EVALUATING NULL PARITY: {name}");
        println!("=======================================================");

        // 1. Local direct
        let mut local = instantiate(plug_id, 48000.0, frames, 2).expect("Instantiate local");
        // 2. OpenGridder
        let mut og = RemoteSession::connect(55056, make_hs()).expect("Connect OG");
        og.add_plugin(plug_id, "").expect("Add to OG");
        thread::sleep(Duration::from_millis(400));

        // Warm up blocks to clear convolution PDC latency
        for _ in 0..12 {
            let mut l_buf = sig.clone();
            local.process_f32(&mut l_buf, &[], frames);
            let _ = og.exchange(&sig).unwrap();
        }

        // Test 5 blocks for bit-exact null
        for b in 0..5 {
            let mut l_buf = sig.clone();
            local.process_f32(&mut l_buf, &[], frames);
            let og_out = og.exchange(&sig).unwrap();

            let (max_err, rms_err, null_depth) = calculate_null(&l_buf, &og_out);
            println!("  [Baseline] Block {b}: MaxErr = {:.2e}, RMS = {:.2e}, Null = {:.2} dBFS (Local peak: {:.4}, OG peak: {:.4})",
                max_err, rms_err, null_depth,
                l_buf[0].iter().map(|s| s.abs()).fold(0.0f32, f32::max),
                og_out[0].iter().map(|s| s.abs()).fold(0.0f32, f32::max)
            );
            assert!(null_depth < -140.0, "Failed null on block {b}: {null_depth:.2} dBFS");
        }

        // Parameter Modulation Test
        let params = og.slot_parameters(0).to_vec();
        println!("  Found {} parameters exposed in OpenGridder", params.len());
        if let Some(first_param) = params.first() {
            println!("  Modulating parameter '{}' (idx: {}) to 0.75", first_param.name, first_param.idx);
            og.set_parameter(0, first_param.idx, 0.75).unwrap();
            local.set_param(first_param.idx, 0.75);
            thread::sleep(Duration::from_millis(50));

            // Settle 5 blocks
            for _ in 0..5 {
                let mut l_buf = sig.clone();
                local.process_f32(&mut l_buf, &[], frames);
                let _ = og.exchange(&sig).unwrap();
            }

            // Verify null under modulation
            let mut l_buf = sig.clone();
            local.process_f32(&mut l_buf, &[], frames);
            let og_out = og.exchange(&sig).unwrap();
            let (max_err, rms_err, null_depth) = calculate_null(&l_buf, &og_out);
            println!("  [Modulated] Parameter '{}' = 0.75: MaxErr = {:.2e}, RMS = {:.2e}, Null = {:.2} dBFS",
                first_param.name, max_err, rms_err, null_depth
            );
            assert!(null_depth < -140.0, "Failed modulated null: {null_depth:.2} dBFS");
        }
        thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn test_acustica_serial_chain() {
    if !acustica_available() {
        eprintln!("Skipping test: requires ag-server on 55056 and Acustica Audio VST3 plugins");
        return;
    }
    println!("\n=======================================================");
    println!("SERIAL CHAIN: WHITE3ZL -> AMBER4ZL");
    println!("=======================================================");
    let frames = 256;
    let sig = make_rich_signal(frames);

    let mut local1 = instantiate("vst3:/Library/Audio/Plug-Ins/VST3/WHITE3ZL.vst3", 48000.0, frames, 2).unwrap();
    let mut local2 = instantiate("vst3:/Library/Audio/Plug-Ins/VST3/AMBER4ZL.vst3", 48000.0, frames, 2).unwrap();

    let mut og = RemoteSession::connect(55056, make_hs()).unwrap();
    og.add_plugin("vst3:/Library/Audio/Plug-Ins/VST3/WHITE3ZL.vst3", "").unwrap();
    og.add_plugin("vst3:/Library/Audio/Plug-Ins/VST3/AMBER4ZL.vst3", "").unwrap();
    thread::sleep(Duration::from_millis(400));

    // Warm up chain
    for _ in 0..15 {
        let mut l_buf = sig.clone();
        local1.process_f32(&mut l_buf, &[], frames);
        local2.process_f32(&mut l_buf, &[], frames);
        let _ = og.exchange(&sig).unwrap();
    }

    for b in 0..5 {
        let mut l_buf = sig.clone();
        local1.process_f32(&mut l_buf, &[], frames);
        local2.process_f32(&mut l_buf, &[], frames);
        let og_out = og.exchange(&sig).unwrap();

        let (max_err, rms_err, null_depth) = calculate_null(&l_buf, &og_out);
        println!("  [Chain] Block {b}: MaxErr = {:.2e}, RMS = {:.2e}, Null = {:.2} dBFS", max_err, rms_err, null_depth);
        assert!(null_depth < -140.0, "Chain null failed: {null_depth:.2} dBFS");
    }
}

#[test]
fn test_acustica_oversampling() {
    if !acustica_available() {
        eprintln!("Skipping test: requires ag-server on 55056 and Acustica Audio VST3 plugins");
        return;
    }
    println!("\n=======================================================");
    println!("OVERSAMPLING SWEEP: WHITE3ZL (1x, 2x, 4x)");
    println!("=======================================================");
    let frames = 256;
    let sig = make_rich_signal(frames);

    let mut os_session = RemoteSession::connect(55056, make_hs()).unwrap();
    os_session.add_plugin("vst3:/Library/Audio/Plug-Ins/VST3/WHITE3ZL.vst3", "").unwrap();
    thread::sleep(Duration::from_millis(400));

    // Warm up 1x
    for _ in 0..10 {
        let _ = os_session.exchange(&sig).unwrap();
    }
    let (base_out, base_lat) = os_session.exchange_measured(&sig).unwrap();
    println!("  1x Mode: Latency = {} samples | Peak = {:.4}", base_lat, base_out[0].iter().map(|s| s.abs()).fold(0.0f32, f32::max));

    for factor in [2, 4] {
        os_session.set_oversampling(0, factor).unwrap();
        thread::sleep(Duration::from_millis(50));
        for _ in 0..5 {
            let _ = os_session.exchange(&sig).unwrap();
        }
        let (os_out, os_lat) = os_session.exchange_measured(&sig).unwrap();
        let (max_diff, rms_diff, _) = calculate_null(&base_out, &os_out);
        println!("  {}x Mode: Latency = {} samples (+{} samples) | Max Diff = {:.4} | RMS Diff = {:.4}",
            factor, os_lat, os_lat - base_lat, max_diff, rms_diff
        );
        assert!(max_diff > 0.001, "Oversampling {}x did not affect audio response", factor);
        assert!(os_lat >= base_lat, "Oversampling {}x did not report increased latency", factor);
    }
}
