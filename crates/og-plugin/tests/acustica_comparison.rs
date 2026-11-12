use std::f32::consts::PI;
use std::thread;
use std::time::Duration;
use og_plugin::RemoteSession;
use og_protocol::HandshakeRequest;
use og_server::host::instantiate;

fn make_hs() -> HandshakeRequest {
    HandshakeRequest {
        samples_per_block: 256,
        sample_rate: 48000.0,
        channels_in: 2,
        channels_out: 2,
        active_channels: u64::MAX,
        ..Default::default()
    }
}

fn acustica_available() -> bool {
    let og_ok = std::net::TcpStream::connect_timeout(
        &"127.0.0.1:55056".parse().unwrap(),
        Duration::from_millis(100),
    ).is_ok();
    let plugins_ok = std::path::Path::new("/Library/Audio/Plug-Ins/VST3/WHITE3ZL.vst3").exists();
    og_ok && plugins_ok
}

fn audiogridder_available() -> bool {
    std::net::TcpStream::connect_timeout(
        &"127.0.0.1:55057".parse().unwrap(),
        Duration::from_millis(100),
    ).is_ok()
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
        let s = (2.0 * PI * 220.0 * t).sin() * 0.35
              + (2.0 * PI * 880.0 * t).sin() * 0.25
              + (2.0 * PI * 2500.0 * t).sin() * 0.20;
        sig[0][i] = s;
        sig[1][i] = s * 0.95;
    }
    sig
}

#[derive(Clone, Copy)]
struct AcusticaPluginTest {
    name: &'static str,
    og_id: &'static str,
    ag_id: &'static str,
    param_idx: i32,
    param_val: f32,
}

const ACUSTICA_TEST_PLUGINS: &[AcusticaPluginTest] = &[
    AcusticaPluginTest {
        name: "Acustica Audio: WHITE3ZL",
        og_id: "vst3:/Library/Audio/Plug-Ins/VST3/WHITE3ZL.vst3",
        ag_id: "VST3-WHITE3ZL-1da962e7",
        param_idx: 0, // Hi_Boost_L
        param_val: 0.75,
    },
    AcusticaPluginTest {
        name: "Acustica Audio: AMBER4ZL",
        og_id: "vst3:/Library/Audio/Plug-Ins/VST3/AMBER4ZL.vst3",
        ag_id: "VST3-AMBER4ZL-f945449f",
        param_idx: 0, // LF Freq
        param_val: 0.70,
    },
    AcusticaPluginTest {
        name: "Acustica Audio: WHITE3",
        og_id: "vst3:/Library/Audio/Plug-Ins/VST3/WHITE3.vst3",
        ag_id: "VST3-WHITE3-88c4b026",
        param_idx: 0, // Hi_Boost_L
        param_val: 0.80,
    },
];

#[test]
fn test_acustica_side_by_side_null_and_modulation() {
    if !acustica_available() {
        eprintln!("Skipping test: requires og-server on 55056 and Acustica Audio VST3 plugins");
        return;
    }

    let frames = 256;
    let sig = make_rich_signal(frames);
    let ag_online = audiogridder_available();

    for test in ACUSTICA_TEST_PLUGINS {
        println!("\n=======================================================");
        println!("EVALUATING SIDE-BY-SIDE: {}", test.name);
        println!("=======================================================");

        // 1. Local
        let mut local = instantiate(test.og_id, 48000.0, frames, 2).expect("Instantiate local");

        // 2. OpenGridder
        let mut og = RemoteSession::connect(55056, make_hs()).expect("Connect OG");
        og.add_plugin(test.og_id, "").expect("Add to OG");

        // 3. AudioGridder (if available)
        let mut ag = if ag_online {
            match RemoteSession::connect(55057, make_hs()) {
                Ok(mut s) => {
                    if s.add_plugin(test.ag_id, "").is_ok() {
                        Some(s)
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        } else {
            None
        };

        println!("Waiting 2 seconds for Acqua background workers to settle...");
        thread::sleep(Duration::from_secs(2));

        // Warm up blocks to clear convolution PDC latency
        for _ in 0..20 {
            let mut l_buf = sig.clone();
            local.process_f32(&mut l_buf, &[], frames);
            let _ = og.exchange(&sig).expect("OG exchange");
            if let Some(ref mut ag_sess) = ag {
                let _ = ag_sess.exchange(&sig).expect("AG exchange");
            }
        }

        // --- Baseline Null Verification (5 blocks) ---
        for b in 0..5 {
            let mut l_buf = sig.clone();
            local.process_f32(&mut l_buf, &[], frames);
            let og_out = og.exchange(&sig).expect("OG exchange");

            let (og_max, og_rms, og_null) = calculate_null(&l_buf, &og_out);
            print!("  [Baseline] Block {b} | OG vs Local: MaxErr={:.2e}, RMS={:.2e}, Null={:.2} dBFS", og_max, og_rms, og_null);

            if let Some(ref mut ag_sess) = ag {
                let ag_out = ag_sess.exchange(&sig).expect("AG exchange");
                let (ag_max, ag_rms, ag_null) = calculate_null(&l_buf, &ag_out);
                let (cross_max, _, cross_null) = calculate_null(&og_out, &ag_out);
                println!(" | AG vs Local: MaxErr={:.2e}, Null={:.2} dBFS | OG vs AG Null={:.2} dBFS", ag_max, ag_null, cross_null);
                assert!(ag_null < -140.0, "AG baseline null failed: {ag_null:.2} dBFS");
                assert!(cross_null < -140.0, "Cross OG vs AG null failed: {cross_null:.2} dBFS");
            } else {
                println!();
            }

            assert!(og_null < -140.0, "OG baseline null failed: {og_null:.2} dBFS");
        }

        // --- Parameter Modulation Test ---
        let og_params = og.slot_parameters(0).to_vec();
        println!("  Plugin exposes {} parameters in OpenGridder", og_params.len());
        println!("  Modulating parameter idx {} to {:.2}", test.param_idx, test.param_val);

        og.set_parameter(0, test.param_idx, test.param_val).expect("OG set_param");
        local.set_param(test.param_idx, test.param_val);
        if let Some(ref mut ag_sess) = ag {
            let _ = ag_sess.set_parameter(0, test.param_idx, test.param_val);
        }

        thread::sleep(Duration::from_millis(100));

        // Settle filter transitions
        for _ in 0..15 {
            let mut l_buf = sig.clone();
            local.process_f32(&mut l_buf, &[], frames);
            let _ = og.exchange(&sig).expect("OG exchange");
            if let Some(ref mut ag_sess) = ag {
                let _ = ag_sess.exchange(&sig).expect("AG exchange");
            }
        }

        // Verify null under modulation
        let mut l_buf = sig.clone();
        local.process_f32(&mut l_buf, &[], frames);
        let og_out = og.exchange(&sig).expect("OG exchange");
        let (og_max, _og_rms, og_null) = calculate_null(&l_buf, &og_out);

        print!("  [Modulated] Param {} = {:.2} | OG vs Local: MaxErr={:.2e}, Null={:.2} dBFS", test.param_idx, test.param_val, og_max, og_null);

        if let Some(ref mut ag_sess) = ag {
            let ag_out = ag_sess.exchange(&sig).expect("AG exchange");
            let (ag_max, _, ag_null) = calculate_null(&l_buf, &ag_out);
            println!(" | AG vs Local: MaxErr={:.2e}, Null={:.2} dBFS", ag_max, ag_null);
            assert!(ag_null < -140.0, "AG modulated null failed: {ag_null:.2} dBFS");
        } else {
            println!();
        }

        assert!(og_null < -140.0, "OG modulated null failed: {og_null:.2} dBFS");
        thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn test_acustica_serial_chain() {
    if !acustica_available() {
        eprintln!("Skipping test: requires og-server on 55056 and Acustica Audio VST3 plugins");
        return;
    }
    println!("\n=======================================================");
    println!("SERIAL CHAIN: WHITE3ZL -> AMBER4ZL");
    println!("=======================================================");
    let frames = 256;
    let sig = make_rich_signal(frames);
    let ag_online = audiogridder_available();

    let mut local1 = instantiate("vst3:/Library/Audio/Plug-Ins/VST3/WHITE3ZL.vst3", 48000.0, frames, 2).unwrap();
    let mut local2 = instantiate("vst3:/Library/Audio/Plug-Ins/VST3/AMBER4ZL.vst3", 48000.0, frames, 2).unwrap();

    let mut og = RemoteSession::connect(55056, make_hs()).unwrap();
    og.add_plugin("vst3:/Library/Audio/Plug-Ins/VST3/WHITE3ZL.vst3", "").unwrap();
    og.add_plugin("vst3:/Library/Audio/Plug-Ins/VST3/AMBER4ZL.vst3", "").unwrap();

    let mut ag = if ag_online {
        match RemoteSession::connect(55057, make_hs()) {
            Ok(mut s) => {
                let r1 = s.add_plugin("VST3-WHITE3ZL-1da962e7", "");
                let r2 = s.add_plugin("VST3-AMBER4ZL-f945449f", "");
                if r1.is_ok() && r2.is_ok() {
                    Some(s)
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    } else {
        None
    };

    println!("Waiting 2 seconds for chained Acqua impulse models to initialize...");
    thread::sleep(Duration::from_secs(2));

    // Warm up chain (combined latency = 66 + 348 = 414 samples)
    for _ in 0..25 {
        let mut l_buf = sig.clone();
        local1.process_f32(&mut l_buf, &[], frames);
        local2.process_f32(&mut l_buf, &[], frames);
        let _ = og.exchange(&sig).unwrap();
        if let Some(ref mut ag_sess) = ag {
            let _ = ag_sess.exchange(&sig).unwrap();
        }
    }

    for b in 0..5 {
        let mut l_buf = sig.clone();
        local1.process_f32(&mut l_buf, &[], frames);
        local2.process_f32(&mut l_buf, &[], frames);
        let og_out = og.exchange(&sig).unwrap();

        let (og_max, _og_rms, og_null) = calculate_null(&l_buf, &og_out);
        print!("  [Chain] Block {b} | OG vs Local Chain: MaxErr={:.2e}, Null={:.2} dBFS", og_max, og_null);

        if let Some(ref mut ag_sess) = ag {
            let ag_out = ag_sess.exchange(&sig).unwrap();
            let (ag_max, _, ag_null) = calculate_null(&l_buf, &ag_out);
            let (_, _, cross_null) = calculate_null(&og_out, &ag_out);
            println!(" | AG vs Local: MaxErr={:.2e}, Null={:.2} dBFS | OG vs AG: {:.2} dBFS", ag_max, ag_null, cross_null);
            assert!(ag_null < -140.0, "AG serial chain null failed: {ag_null:.2} dBFS");
            assert!(cross_null < -140.0, "OG vs AG serial chain null failed: {cross_null:.2} dBFS");
        } else {
            println!();
        }

        assert!(og_null < -140.0, "OG serial chain null failed: {og_null:.2} dBFS");
    }
}

#[test]
fn test_acustica_oversampling() {
    if !acustica_available() {
        eprintln!("Skipping test: requires og-server on 55056 and Acustica Audio VST3 plugins");
        return;
    }
    println!("\n=======================================================");
    println!("OVERSAMPLING SWEEP: WHITE3ZL (1x, 2x, 4x)");
    println!("=======================================================");
    let frames = 256;
    let sig = make_rich_signal(frames);

    let mut os_session = RemoteSession::connect(55056, make_hs()).unwrap();
    os_session.add_plugin("vst3:/Library/Audio/Plug-Ins/VST3/WHITE3ZL.vst3", "").unwrap();
    thread::sleep(Duration::from_secs(1));

    // Warm up 1x
    for _ in 0..15 {
        let _ = os_session.exchange(&sig).unwrap();
    }
    let (base_out, base_lat) = os_session.exchange_measured(&sig).unwrap();
    let base_peak = base_out[0].iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    println!("  1x Mode: Latency = {} samples | Peak = {:.4}", base_lat, base_peak);
    assert_eq!(base_lat, 66, "WHITE3ZL reports 66 sample PDC latency");

    for factor in [2, 4] {
        os_session.set_oversampling(0, factor).unwrap();
        thread::sleep(Duration::from_millis(100));
        for _ in 0..10 {
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
