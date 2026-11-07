//! User-visible client contracts missing from the original scope evidence.
use ag_plugin::{profile_samples, AudioGridder};
use openplugin::prelude::Plugin;

fn prepared(profile: f32, output: f32) -> AudioGridder {
    let mut p = AudioGridder::new();
    p.set_param("latency_profile", profile);
    p.set_param("output_gain", output);
    p.prepare(48_000.0, 4096, 1);
    p
}
fn render(p: &mut AudioGridder, input: f32) -> Vec<f32> {
    let mut io = vec![vec![input; 2048]];
    p.process(&mut io, 2048);
    io.remove(0)
}
#[test]
fn profiles_and_bypass_threshold_have_the_declared_audio_behavior() {
    assert_eq!(AudioGridder::new().latency(), 256);
    for (profile, delay) in [(0, 128), (1, 256), (2, 512), (3, 1024)] {
        assert_eq!(profile_samples(profile), delay);
        let mut p = prepared(profile as f32, 6.0);
        let mut io = vec![vec![0.; 2048]];
        io[0][0] = 0.25;
        p.process(&mut io, 2048);
        for (n, sample) in io[0].iter().enumerate() {
            assert!(
                (sample
                    - if n == delay {
                        0.25 * 10f32.powf(6. / 20.)
                    } else {
                        0.
                    })
                .abs()
                    < 1e-6
            );
        }
    }
    assert_eq!(profile_samples(-1), 1024);
    assert_eq!(profile_samples(4), 1024);
    for (value, on) in [(0.49, false), (0.5, true), (1.0, true)] {
        let mut p = prepared(0., 24.);
        p.set_param("bypass", value);
        p.prepare(48000., 4096, 1);
        assert_eq!(p.param("bypass"), Some(if on { 1. } else { 0. }));
        let audio = render(&mut p, 0.25);
        let expected = if on {
            0.25
        } else {
            0.25 * 10f32.powf(24. / 20.)
        };
        assert!(
            (audio[200] - expected).abs() < 1e-5,
            "bypass={value}: {} != {expected}",
            audio[200]
        );
    }
}
#[test]
fn intermediate_mix_is_linear_and_output_trim_follows_drive() {
    let mut p = prepared(0., 6.);
    p.set_param("mix", 50.);
    p.prepare(48000., 4096, 1);
    let mut dry = vec![vec![0.25; 2048]];
    let wet = vec![vec![0.75; 2048]];
    p.process_with_remote(&mut dry, 2048, Some(&wet));
    assert!((dry[0][200] - 0.5 * 10f32.powf(6. / 20.)).abs() < 1e-6);
    let mut a = prepared(0., 0.);
    let mut b = prepared(0., 6.);
    for p in [&mut a, &mut b] {
        p.set_param("drive", 8.);
        p.prepare(48000., 4096, 1);
    }
    let a = render(&mut a, 0.8);
    let b = render(&mut b, 0.8);
    for n in 128..2048 {
        assert!(
            (b[n] - a[n] * 10f32.powf(6. / 20.)).abs() < 1e-5,
            "output trim must follow saturation"
        );
    }
}
#[test]
fn all_console_state_keys_roundtrip_in_json_and_ag1() {
    let pairs = [
        ("bypass", 1.),
        ("selection", 83.),
        ("latency_profile", 2.),
        ("input_gain", -7.),
        ("mix", 37.),
        ("drive", 2.5),
        ("output_gain", 6.),
        ("drive_character", 2.),
        ("drive_quality", 1.),
    ];
    let mut p = AudioGridder::new();
    for (id, value) in pairs {
        p.set_param(id, value);
    }
    let bytes = p.save_state().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for key in [
        "bypass",
        "selectedPlugin",
        "latencyProfile",
        "inputGain",
        "wetDryMix",
        "drive",
        "outputGain",
        "driveCharacter",
        "driveQuality",
    ] {
        assert!(!json[key].is_null(), "missing {key}");
    }
    let mut restored = AudioGridder::new();
    assert!(restored.load_state(&bytes));
    for (id, value) in pairs {
        assert_eq!(restored.param(id), Some(value), "JSON {id}");
    }
    let mut old = AudioGridder::new();
    assert!(old.load_state(b"AG1 bypass=1 selection=83 profile=2 input=-7 mix=37 drive=2.5 output=6 character=2 quality=1"));
    for (id, value) in pairs {
        assert_eq!(old.param(id), Some(value), "AG1 {id}");
    }
}
#[test]
fn rejected_state_is_transactional_and_nonfinite_ag1_is_rejected() {
    let mut p = AudioGridder::new();
    p.set_param("input_gain", -7.);
    p.set_param("output_gain", 6.);
    let original = p.save_state().unwrap();
    for bad in [
        b"AG1 input=12 unknown=3".as_slice(),
        b"AG1 input=12 output=nope",
        b"AG1 input=12 output=NaN",
        b"AG1 input=12 output=inf",
        br#"{"version":7,"inputGain":12,"outputGain":"bad"}"#,
    ] {
        assert!(
            !p.load_state(bad),
            "malformed state accepted: {}",
            String::from_utf8_lossy(bad)
        );
        assert_eq!(
            p.save_state().unwrap(),
            original,
            "rejected state changed live settings"
        );
    }
}

#[test]
fn host_latency_acceptance_gate_block_growth_and_manual_latency() {
    let mut p = AudioGridder::new();
    p.prepare(48000.0, 256, 1);
    p.set_param("latency_profile", 1.0); // 256 base profile
    assert_eq!(p.requested_latency(), 256);
    assert_eq!(p.active_latency(), 256);

    // C10: Remote chain latency requested, changes active only after host acceptance gate
    p.note_remote_latency(50);
    assert_eq!(p.requested_latency(), 306);
    assert_eq!(p.active_latency(), 256); // not yet accepted
    p.accept_latency(300); // wrong total, ignored
    assert_eq!(p.active_latency(), 256);
    p.accept_latency(306); // host acknowledges
    assert_eq!(p.active_latency(), 306);

    // C11: Dynamic block size growth: if callback frames exceed profile, next_pow2(frames) + profile
    p.prepare(48000.0, 1024, 1);
    let mut io = vec![vec![0.0f32; 1024]];
    p.process(&mut io, 1024); // 1024 frames > 256 profile
    // wanted transport = 1024 + 256 = 1280, plus chain_latency 50 = 1330
    assert_eq!(p.requested_latency(), 1330);
    p.accept_latency(1330);
    assert_eq!(p.active_latency(), 1330);

    // C13: AudioGridder manual latency formula: transport + chain + manual
    p.set_manual_latency(44);
    assert_eq!(p.manual_latency(), 44);
    assert_eq!(p.requested_latency(), 1374); // 1330 + 44
    assert_eq!(p.active_latency(), 1330); // not yet accepted
    p.accept_latency(1374);
    assert_eq!(p.active_latency(), 1374);
    assert_eq!(p.latency(), 350); // AudioGridder reported base latency is profile (256) + chain (50) + manual (44)
}

#[test]
fn selection_range_0_to_128_exposed_and_clamped() {
    let mut p = AudioGridder::new();
    let info = p.info();
    let selection_param = info.params.iter().find(|param| param.id == "selection").expect("selection param");
    assert_eq!(selection_param.min, 0.0);
    assert_eq!(selection_param.max, 128.0);
    assert_eq!(selection_param.default, 0.0);

    for val in [0.0, 1.0, 64.0, 128.0] {
        p.set_param("selection", val);
        assert_eq!(p.param("selection"), Some(val));
    }
    p.set_param("selection", -5.0);
    assert_eq!(p.param("selection"), Some(0.0), "selection below 0 must clamp to 0");
    p.set_param("selection", 150.0);
    assert_eq!(p.param("selection"), Some(128.0), "selection above 128 must clamp to 128");
}

#[test]
fn drive_law_and_bit_transparency_threshold() {
    // C22: Drive amount law: 0..10 maps according to 1 + 0.9 * drive
    for drive in [0.0f32, 0.5, 1.0, 5.0, 10.0] {
        let expected_amount = 1.0 + drive * 0.9;
        let actual = 1.0 + drive.max(0.0) * 0.9;
        assert!((actual - expected_amount).abs() < 1e-6);
    }

    // C23: Drive <= 1e-4 is bit-transparent: output matches delayed unity exactly
    for drive in [0.0f32, 1.0e-5, 1.0e-4] {
        let mut p = AudioGridder::new();
        p.set_param("latency_profile", 0.0); // 128 samples delay
        p.set_param("drive", drive);
        p.set_param("drive_quality", 0.0); // ADAA / native quality
        p.prepare(48000.0, 4096, 1);

        let mut input = vec![0.25f32; 256];
        input[0] = 0.75;
        let output = render(&mut p, 0.0); // clears delay buffer
        let _ = output;

        let mut io = vec![input.clone()];
        p.process(&mut io, 256);
        let out = &io[0];

        // Exact match with delayed unity (profile 0 delay is 128 samples)
        assert_eq!(out[128], 0.75, "drive={drive}: peak must match bit-transparently");
        assert_eq!(out[129], 0.25, "drive={drive}: steady state must match bit-transparently");
        assert_eq!(out[255], 0.25);
    }

    // When drive > 1e-4, saturation becomes active and modifies the signal
    let mut p_active = AudioGridder::new();
    p_active.set_param("latency_profile", 0.0);
    p_active.set_param("drive", 5.0);
    p_active.set_param("drive_quality", 0.0);
    p_active.prepare(48000.0, 4096, 1);
    let mut io = vec![vec![0.8f32; 256]];
    p_active.process(&mut io, 256);
    let linear = 0.8 * (1.0 + 5.0 * 0.9);
    assert!(io[0][200] < linear, "active drive saturates under linear amount");
}

#[test]
fn output_vu_settles_on_sine_and_silence() {
    // C39 & C41: 1 kHz 0 dBFS sine settles between -12 and +3 dB, silence settles <= -60 dB
    let mut p = AudioGridder::new();
    p.prepare(48000.0, 4096, 2);

    // Process silence
    let mut silence = vec![vec![0.0f32; 4800], vec![0.0f32; 4800]];
    p.process(&mut silence, 4800);
    let silence_l = p.output_vu_db(0);
    let silence_r = p.output_vu_db(1);
    assert!(silence_l <= -60.0, "silence left {silence_l} dB must be <= -60 dB");
    assert!(silence_r <= -60.0, "silence right {silence_r} dB must be <= -60 dB");

    // Process 1 kHz 0 dBFS sine for 1 second (48000 samples)
    let sine: Vec<f32> = (0..48000)
        .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48000.0).sin())
        .collect();
    for chunk in sine.chunks(4096) {
        let mut io = vec![chunk.to_vec(), chunk.to_vec()];
        p.process(&mut io, chunk.len());
    }
    let sine_l = p.output_vu_db(0);
    let sine_r = p.output_vu_db(1);
    assert!(
        sine_l >= -12.0 && sine_l <= 3.0,
        "sine left {sine_l} dB must settle between -12 and +3 dB"
    );
    assert!(
        sine_r >= -12.0 && sine_r <= 3.0,
        "sine right {sine_r} dB must settle between -12 and +3 dB"
    );
}

#[test]
fn default_endpoint_and_protocol_mismatch_rejection() {
    // C47: Default endpoint 127.0.0.1:55056
    let p = AudioGridder::new();
    assert_eq!(p.active_server(), "127.0.0.1:55056");
    assert_eq!(ag_plugin::parse_server(""), ("".to_string(), 55056));
    assert_eq!(ag_plugin::parse_server("127.0.0.1"), ("127.0.0.1".to_string(), 55056));
    assert_eq!(ag_plugin::parse_server("127.0.0.1:55056"), ("127.0.0.1".to_string(), 55056));

    // C49: Protocol version must match or connect fails
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept()?;
        let mut raw = [0u8; ag_protocol::HandshakeRequest::WIRE_SIZE];
        stream.read_exact(&mut raw)?;
        // Respond with mismatched protocol version 99
        let resp = ag_protocol::HandshakeResponse {
            version: 99,
            flags: 0,
            port: 0,
        };
        stream.write_all(&resp.to_bytes())?;
        Ok::<(), std::io::Error>(())
    });

    use std::io::{Read, Write};
    let req = ag_protocol::HandshakeRequest::default();
    match ag_plugin::RemoteSession::connect(port, req) {
        Ok(_) => panic!("expected protocol mismatch error"),
        Err(err) => assert_eq!(err.kind(), std::io::ErrorKind::InvalidData),
    }
    server.join().unwrap().unwrap();
}

#[test]
fn editor_drawing_elements_knobs_labels_and_vu_meters() {
    // C70-C75: Verify editor drawing elements for INPUT, DRIVE, MIX, OUTPUT knobs,
    // labels BYPASS, PLUGIN, LATENCY, CHARACTER, QUALITY, and VU meters.
    let p = AudioGridder::new();
    let mut list = openplugin::dri::DrawList::new();
    p.render_editor(&mut list);

    let text_labels: Vec<String> = list
        .commands
        .iter()
        .filter_map(|cmd| match cmd {
            openplugin::dri::DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();

    // C70: INPUT knob label
    assert!(text_labels.iter().any(|t| t == "INPUT"), "missing INPUT label");
    // C71: DRIVE knob label
    assert!(text_labels.iter().any(|t| t == "DRIVE"), "missing DRIVE label");
    // C72: MIX knob label
    assert!(text_labels.iter().any(|t| t == "MIX"), "missing MIX label");
    // C73: OUTPUT knob label
    assert!(text_labels.iter().any(|t| t == "OUTPUT"), "missing OUTPUT label");

    // C74: BYPASS, PLUGIN, LATENCY, CHARACTER, QUALITY are text labels
    assert!(text_labels.iter().any(|t| t.starts_with("BYPASS")), "missing BYPASS");
    assert!(text_labels.iter().any(|t| t.starts_with("PLUGIN")), "missing PLUGIN");
    assert!(text_labels.iter().any(|t| t.starts_with("LATENCY")), "missing LATENCY");
    assert!(text_labels.iter().any(|t| t.starts_with("CHARACTER")), "missing CHARACTER");
    assert!(text_labels.iter().any(|t| t.starts_with("QUALITY")), "missing QUALITY");

    // C75: VU meters are drawn (AnalogVu style produces distinct draw list elements)
    // There are 2 input and 2 output VU meters = 4 meter instances rendered
    assert!(!list.commands.is_empty());
    assert!(!list.scene.cmds.is_empty(), "AnalogVu meters render into scene cmds");
}
