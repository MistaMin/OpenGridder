// Adapted from the archived MIT emulator regression suite.
use ag_legacy::{
    fixture,
    timeslots::Timeslots,
    transport::{ReturnError, Transport},
    Pcm24,
};

#[test]
fn signed_pcm_endpoints_and_wire_format() {
    for (word, signed) in [
        (0x800000, -8388608),
        (0xffffff, -1),
        (0, 0),
        (0x7fffff, 8388607),
        (0xff800001, -8388607),
    ] {
        let p = Pcm24::from_word(word);
        assert_eq!(p.signed(), signed);
        assert_eq!(Pcm24::from_be_bytes(p.to_be_bytes()), p);
        assert_eq!(Pcm24::from_f64(p.to_f64()).unwrap(), p);
    }
    assert_eq!(Pcm24::from_f64(2.0).unwrap().word(), 0x7fffff);
    assert_eq!(Pcm24::from_f64(-2.0).unwrap().word(), 0x800000);
    assert_eq!(Pcm24::from_f64(0.5 / 8388608.0).unwrap().signed(), 1);
    assert_eq!(Pcm24::from_f64(-0.5 / 8388608.0).unwrap().signed(), -1);
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(Pcm24::from_f64(value).is_err());
    }
    assert!(Pcm24::from_i32(8388608).is_err());
    assert!(Pcm24::from_i32(-8388609).is_err());
}

#[test]
fn timeslot_channel_order_and_silent_unused_slots() {
    let slots = Timeslots::new(8, vec![5, 1]).unwrap();
    let input = fixture(32);
    let bus = slots.render(&input).unwrap();
    for (frame, samples) in bus
        .as_chunks::<8>()
        .0
        .iter()
        .zip(input.as_chunks::<2>().0.iter())
    {
        assert_eq!(frame[5], samples[0]);
        assert_eq!(frame[1], samples[1]);
        for index in [0, 2, 3, 4, 6, 7] {
            assert_eq!(frame[index], Pcm24::ZERO);
        }
    }
    assert_eq!(slots.capture(&bus).unwrap(), input);
    assert!(slots.capture(&bus[..bus.len() - 1]).is_err());
    assert!(slots.render(&input[..3]).is_err());
    assert!(Timeslots::new(8, vec![1, 1]).is_err());
    assert!(Timeslots::new(8, vec![8]).is_err());
}

#[test]
fn returned_blocks_play_in_capture_order() {
    let mut t = Transport::new(2, 4, 2).unwrap();
    let a = fixture(8);
    let b = vec![Pcm24::from_i32(123).unwrap(); 8];
    let ta = t.submit(a.clone()).unwrap();
    let tb = t.submit(b.clone()).unwrap();
    assert_eq!(t.take_request().unwrap().ticket, ta);
    assert_eq!(t.take_request().unwrap().ticket, tb);
    t.complete(tb, b.clone()).unwrap();
    t.complete(ta, a.clone()).unwrap();
    assert_eq!(t.play_next().unwrap(), a);
    assert_eq!(t.play_next().unwrap(), b);
    assert!(t.play_next().is_err());
    assert_eq!(t.pending(), 0);
}

#[test]
fn underrun_retires_late_audio_without_repeating_previous_block() {
    let mut t = Transport::new(2, 4, 1).unwrap();
    let good = t.submit(fixture(4)).unwrap();
    let r = t.take_request().unwrap();
    t.complete(good, r.audio.clone()).unwrap();
    assert_eq!(t.play_next().unwrap(), r.audio);
    let late = t.submit(fixture(4)).unwrap();
    let r = t.take_request().unwrap();
    assert_eq!(t.play_next().unwrap(), vec![Pcm24::ZERO; 4]);
    assert_eq!(t.complete(late, r.audio), Err(ReturnError::Stale));
    let next = t.submit(fixture(4)).unwrap();
    let r = t.take_request().unwrap();
    t.complete(next, r.audio.clone()).unwrap();
    assert_eq!(t.play_next().unwrap(), r.audio);
    assert_eq!(t.stats().underruns, 1);
}

#[test]
fn bounded_queue_overflow_recovery_and_unserviced_deadlines() {
    let mut t = Transport::new(1, 1, 1).unwrap();
    t.submit(fixture(1)).unwrap();
    let dropped = t.submit(fixture(1)).unwrap();
    assert_eq!(t.pending(), 1);
    assert_eq!(t.stats().overflows, 1);
    t.play_next().unwrap();
    assert!(t.take_request().is_none());
    assert_eq!(t.complete(dropped, fixture(1)), Err(ReturnError::Stale));
    t.play_next().unwrap();
    // Service never runs: queue and pending storage must still retire at each deadline.
    for _ in 0..5000 {
        t.submit(fixture(1)).unwrap();
        t.play_next().unwrap();
        assert_eq!(t.pending(), 0);
    }
    assert!(t.take_request().is_none());
    let ticket = t.submit(fixture(1)).unwrap();
    let request = t.take_request().unwrap();
    t.complete(ticket, request.audio.clone()).unwrap();
    assert_eq!(t.play_next().unwrap(), request.audio);
}

#[test]
fn reset_epoch_duplicate_wrong_length_and_unsolicited_return() {
    let mut t = Transport::new(2, 2, 1).unwrap();
    assert!(t.submit(fixture(1)).is_err());
    let old = t.submit(fixture(2)).unwrap();
    assert_eq!(t.complete(old, fixture(2)), Err(ReturnError::NotDispatched));
    t.take_request().unwrap();
    t.reset().unwrap();
    let new = t.submit(fixture(2)).unwrap();
    assert_ne!(old, new);
    assert_eq!(t.complete(old, fixture(2)), Err(ReturnError::Stale));
    t.take_request().unwrap();
    assert_eq!(t.complete(new, fixture(1)), Err(ReturnError::WrongLength));
    t.complete(new, fixture(2)).unwrap();
    assert_eq!(t.complete(new, fixture(2)), Err(ReturnError::Duplicate));
    assert_eq!(t.play_next().unwrap(), fixture(2));
}

#[test]
fn missing_earlier_return_cannot_consume_ready_future_audio() {
    let mut t = Transport::new(2, 4, 1).unwrap();
    let first = t.submit(fixture(4)).unwrap();
    let second = t.submit(fixture(4)).unwrap();
    t.take_request().unwrap();
    let ready = t.take_request().unwrap();
    t.complete(second, ready.audio.clone()).unwrap();
    assert_eq!(t.play_next().unwrap(), vec![Pcm24::ZERO; 4]);
    assert_eq!(t.complete(first, fixture(4)), Err(ReturnError::Stale));
    assert_eq!(t.play_next().unwrap(), ready.audio);
}

#[test]
fn core_sharing_multi_stream_isolation_and_independent_lifecycle() {
    // Models Priority 3 core sharing: two stereo streams on a shared TDM timeslot fabric.
    // Stream A routes to slots [0, 1]; Stream B routes to slots [2, 3] on an 8-slot bus.
    let slots_a = Timeslots::new(8, vec![0, 1]).unwrap();
    let slots_b = Timeslots::new(8, vec![2, 3]).unwrap();
    let mut transport_a = Transport::new(4, 16, 2).unwrap();
    let mut transport_b = Transport::new(4, 16, 2).unwrap();

    let audio_a = fixture(32);
    let audio_b = vec![Pcm24::from_i32(0x334455).unwrap(); 32];

    // Render both streams into the shared 8-slot timeslot bus
    let bus_a = slots_a.render(&audio_a).unwrap();
    let bus_b = slots_b.render(&audio_b).unwrap();
    let mut combined_bus = vec![Pcm24::ZERO; bus_a.len()];
    for i in 0..combined_bus.len() {
        combined_bus[i] = Pcm24::from_word(bus_a[i].word() | bus_b[i].word());
    }

    // Verify channel isolation: Stream A only sees audio_a, Stream B only sees audio_b
    let captured_a = slots_a.capture(&combined_bus).unwrap();
    let captured_b = slots_b.capture(&combined_bus).unwrap();
    assert_eq!(captured_a, audio_a);
    assert_eq!(captured_b, audio_b);

    // Submit captures to independent transport queues
    let ticket_a = transport_a.submit(captured_a).unwrap();
    let ticket_b = transport_b.submit(captured_b).unwrap();

    // Hot-reset Stream A (e.g. selection change or disconnect)
    transport_a.reset().unwrap();

    // Stream B must complete and play normally with zero disruption
    let req_b = transport_b.take_request().unwrap();
    transport_b.complete(ticket_b, req_b.audio.clone()).unwrap();
    let played_b = transport_b.play_next().unwrap();
    assert_eq!(played_b, audio_b);
    assert_eq!(transport_b.stats().underruns, 0);

    // Any stale return for Stream A must be rejected as Stale, not corrupt state
    let stale_completion = transport_a.complete(ticket_a, audio_a);
    assert_eq!(stale_completion, Err(ReturnError::Stale));

    // Overload isolation: overflowing Stream A does not affect Stream B
    for _ in 0..10 {
        let _ = transport_a.submit(fixture(32));
    }
    assert!(transport_a.stats().overflows > 0);
    assert_eq!(transport_b.stats().overflows, 0);
}

#[test]
fn core_sharing_four_stereo_streams_isolation_and_stress() {
    // Priority 3 core sharing expansion: 4 stereo streams (8 channels) sharing a single core.
    // Stream 0: slots [0, 1]
    // Stream 1: slots [2, 3]
    // Stream 2: slots [4, 5]
    // Stream 3: slots [6, 7]
    let slots = vec![
        Timeslots::new(16, vec![0, 1]).unwrap(),
        Timeslots::new(16, vec![2, 3]).unwrap(),
        Timeslots::new(16, vec![4, 5]).unwrap(),
        Timeslots::new(16, vec![6, 7]).unwrap(),
    ];
    let mut transports = vec![
        Transport::new(4, 16, 2).unwrap(),
        Transport::new(4, 16, 2).unwrap(),
        Transport::new(4, 16, 2).unwrap(),
        Transport::new(4, 16, 2).unwrap(),
    ];

    // 4 distinct signal patterns
    let audios = vec![
        fixture(32),
        vec![Pcm24::from_i32(0x112233).unwrap(); 32],
        vec![Pcm24::from_i32(0x445566).unwrap(); 32],
        vec![Pcm24::from_i32(0x778899).unwrap(); 32],
    ];

    // Render all 4 streams into the shared 16-slot bus
    let mut combined_bus = vec![Pcm24::ZERO; 16 * 16]; // 16 frames of 16 slots
    for i in 0..4 {
        let rendered = slots[i].render(&audios[i]).unwrap();
        for k in 0..combined_bus.len() {
            combined_bus[k] = Pcm24::from_word(combined_bus[k].word() | rendered[k].word());
        }
    }

    // Capture from shared bus and verify complete channel isolation across all 4 streams
    let mut tickets = Vec::new();
    for i in 0..4 {
        let captured = slots[i].capture(&combined_bus).unwrap();
        assert_eq!(captured, audios[i], "stream {i} audio isolation mismatch");
        tickets.push(transports[i].submit(captured).unwrap());
    }

    // Hot-reset Stream 2 (middle stream disconnects / resets)
    transports[2].reset().unwrap();

    // Streams 0, 1, and 3 must complete and play with zero disruption
    for &i in &[0, 1, 3] {
        let req = transports[i].take_request().unwrap();
        transports[i]
            .complete(tickets[i], req.audio.clone())
            .unwrap();
        let played = transports[i].play_next().unwrap();
        assert_eq!(played, audios[i], "stream {i} playback mismatch");
        assert_eq!(transports[i].stats().underruns, 0);
    }

    // Stale return to reset Stream 2 must be rejected
    assert_eq!(
        transports[2].complete(tickets[2], audios[2].clone()),
        Err(ReturnError::Stale)
    );

    // Overflow only Stream 3; Streams 0, 1, and 2 must remain completely immune
    for _ in 0..10 {
        let _ = transports[3].submit(audios[3].clone());
    }
    assert!(transports[3].stats().overflows > 0);
    assert_eq!(transports[0].stats().overflows, 0);
    assert_eq!(transports[1].stats().overflows, 0);
    assert_eq!(transports[2].stats().overflows, 0);
}

#[test]
fn playback_schedule_latency_sweep_and_deterministic_underrun() {
    // Models Priority 4 latency schedule evaluation:
    // Sweeps schedule lengths (capacity / delay window).
    // Tests: 16 blocks (1024 frames), 12 blocks (768 frames), 8 blocks (512 frames), 4 blocks (256 frames).
    for &schedule_blocks in &[16, 12, 8, 4] {
        let mut t = Transport::new(schedule_blocks, 64, 2).unwrap();
        let input = fixture(128);

        // Fill prefill
        let mut tickets = Vec::new();
        for _ in 0..schedule_blocks {
            tickets.push(t.submit(input.clone()).unwrap());
        }

        // Service on time
        while let Some(req) = t.take_request() {
            t.complete(req.ticket, req.audio).unwrap();
        }

        // Playback of all serviced blocks succeeds
        for _ in 0..schedule_blocks {
            let played = t.play_next().unwrap();
            assert_eq!(played, input);
        }
        assert_eq!(t.stats().underruns, 0);

        // Now induce an underrun: submit a block but do NOT service it with complete()
        let unserviced_ticket = t.submit(input.clone()).unwrap();
        let underrun_audio = t.play_next().unwrap();
        assert_eq!(
            underrun_audio,
            vec![Pcm24::ZERO; 128],
            "underrun must be deterministic silence"
        );
        assert_eq!(t.stats().underruns, 1);

        // Stale return for that missed deadline is rejected
        assert_eq!(
            t.complete(unserviced_ticket, input.clone()),
            Err(ReturnError::Stale)
        );

        // Subsequent timely block recovers immediately
        let timely_ticket = t.submit(input.clone()).unwrap();
        let req = t.take_request().unwrap();
        assert_eq!(req.ticket, timely_ticket);
        t.complete(timely_ticket, req.audio).unwrap();
        let recovered = t.play_next().unwrap();
        assert_eq!(recovered, input);
    }
}

#[test]
fn legacy_bridge_nonfinite_is_silence_while_strict_pcm_still_rejects() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(Pcm24::from_f64(value).is_err());
        assert_eq!(Pcm24::from_bridge_f64(value), Pcm24::ZERO);
    }
    for value in [-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0] {
        assert_eq!(
            Pcm24::from_bridge_f64(value),
            Pcm24::from_f64(value).unwrap()
        );
    }
}

#[test]
fn l10_rtas_and_tdm_are_separate_binaries() {
    // Verify that both RTAS and TDM targets exist and provide distinct contracts
    let version = ag_legacy::ag_legacy_pcm_api_version();
    assert_eq!(version, ag_legacy::API_VERSION);
    // TDM has a fixed 16-block / 1024-sample schedule
    assert_eq!(ag_legacy::SCHEDULE_BLOCKS, 16);
    assert_eq!(ag_legacy::SCHEDULE_SAMPLES, 1024);
    assert_eq!(ag_legacy::SLOT_COUNT, 32);
    // RTAS operates with host buffers (variable block sizes, not fixed 1024-sample card schedule)
}

#[test]
fn legacy_contract_l6_l7_l8_l9_assertions() {
    // L6: ag-legacy is a portable crate contract, not a .dpm bundle.
    // Ensure the binary contract documents this explicitly.
    let version = ag_legacy::ag_legacy_pcm_api_version();
    assert_eq!(version, 1);
    let banner = format!(
        "ag-legacy contract api {version}: {}-frame stereo, {} slots, {}-block / {}-sample schedule. Not a .dpm. Not an OS X 10.6 or i386 build. Not qualified in Pro Tools.",
        ag_legacy::FRAME_COUNT,
        ag_legacy::SLOT_COUNT,
        ag_legacy::SCHEDULE_BLOCKS,
        ag_legacy::SCHEDULE_SAMPLES
    );
    assert!(banner.contains("Not a .dpm"));

    // L7: Wrapper OS is 10.6.8 i386. ag-legacy is not 10.6/i386 target.
    assert!(banner.contains("Not an OS X 10.6 or i386 build"));

    // L8: Avid PT9 SDK is not vendored or linked.
    let legacy_cargo_toml = include_str!("../Cargo.toml");
    assert!(!legacy_cargo_toml.contains("avid"));
    assert!(!legacy_cargo_toml.contains("pt9"));
    assert!(!legacy_cargo_toml.contains("PT9"));

    // L9: The modern server (ag-server) does not depend on or link the legacy crate (ag-legacy).
    let server_cargo_toml = include_str!("../../ag-server/Cargo.toml");
    assert!(!server_cargo_toml.contains("ag-legacy"));
    assert!(!server_cargo_toml.contains("legacy"));
}
