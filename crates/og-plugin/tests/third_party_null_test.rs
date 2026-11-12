use std::time::Duration;
use og_protocol::HandshakeRequest;
use og_plugin::RemoteSession;
use og_server::host::{instantiate, scan};

#[test]
fn third_party_plugin_remote_vs_local_null_test() {
    let catalog = scan();
    let candidates: Vec<_> = catalog.iter().filter(|p| {
        !p.is_instrument && (
            p.id.contains("lpas") ||
            p.id.contains("dely") ||
            p.id.contains("lmtr") ||
            p.id.contains("BlCh")
        )
    }).cloned().collect();

    assert!(!candidates.is_empty(), "No candidate plugins found");

    for target in candidates {
        println!("\nTesting plugin: {} ({})", target.name, target.id);

        let mut session = RemoteSession::connect(55056, HandshakeRequest {
            samples_per_block: 256,
            sample_rate: 48000.0,
            channels_in: 2,
            channels_out: 2,
            ..Default::default()
        }).expect("Failed to connect to OpenGridder Server on 127.0.0.1:55056");

        session.add_plugin(&target.id, "").expect("Failed to load plugin into remote server slot");
        std::thread::sleep(Duration::from_millis(40));

        let mut local_slot = match instantiate(&target.id, 48000.0, 256, 2) {
            Some(s) => s,
            None => {
                println!("Skipping {} (cannot instantiate locally)", target.id);
                continue;
            }
        };

        let frames = 256;
        let mut test_signal = vec![vec![0.0f32; frames]; 2];
        for i in 0..frames {
            let t = i as f32 / 48000.0;
            let s = (2.0 * std::f32::consts::PI * 220.0 * t).sin() * 0.4
                  + (2.0 * std::f32::consts::PI * 880.0 * t).sin() * 0.3
                  + (2.0 * std::f32::consts::PI * 2500.0 * t).sin() * 0.2;
            test_signal[0][i] = s;
            test_signal[1][i] = s * 0.9;
        }

        // Warm-up / priming cycle
        let mut warm_local = test_signal.clone();
        local_slot.process_f32(&mut warm_local, &[], frames);
        let _ = session.exchange(&test_signal).expect("Warmup remote exchange failed");

        let mut local_out = test_signal.clone();
        local_slot.process_f32(&mut local_out, &[], frames);

        let remote_out = session.exchange(&test_signal)
            .expect("Remote audio exchange failed");

        let mut max_err = 0.0f32;
        let mut sum_sq_err = 0.0f64;
        let mut total_samples = 0;

        for ch in 0..2 {
            for i in 0..frames {
                let loc = local_out[ch][i];
                let rem = remote_out[ch][i];
                let diff = (rem - loc).abs();
                if diff > max_err {
                    max_err = diff;
                }
                sum_sq_err += (diff as f64) * (diff as f64);
                total_samples += 1;
            }
        }

        let rms_err = (sum_sq_err / total_samples as f64).sqrt();
        let null_db = if rms_err > 0.0 { 20.0 * (rms_err as f32).log10() } else { -200.0 };

        println!("  -> Max absolute error: {:.6e}", max_err);
        println!("  -> RMS error:          {:.6e}", rms_err);
        println!("  -> Null depth:         {:.2} dBFS", null_db);

        assert!(
            null_db < -120.0 || max_err < 1.0e-5,
            "Null test failed for {}: max={max_err}, null={null_db} dBFS", target.name
        );
    }
    println!("\nALL THIRD-PARTY CANDIDATES NULLED PERFECTLY AGAINST LOCAL NORMAL SLOTS!");
}
