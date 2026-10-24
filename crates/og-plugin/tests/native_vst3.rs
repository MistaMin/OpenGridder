//! Opt-in integration with a real installed effect, never an emulated processor.
#![cfg(target_os = "macos")]
use og_common::message::PositionInfo;
use og_hosting::vst3;
use og_plugin::{PluginMode, PluginProcessor};
use og_server::{Server, ServerConfig};
#[test]
#[ignore = "requires OPENGRIDDER_TEST_VST3 pointing to the installed Water Channel Reference VST3"]
fn installed_vst3_network_audio_parameters_and_state() {
    let path = std::env::var_os("OPENGRIDDER_TEST_VST3")
        .expect("Set OPENGRIDDER_TEST_VST3 to the Water Channel Reference bundle");
    assert!(vst3::hosting_available(), "VST3 SDK bridge must be enabled");
    let desc = vst3::inspect_vst3_bundle(std::path::Path::new(&path))
        .expect("real VST3 factory descriptor");
    assert_eq!(
        desc.name, "Water Channel Reference",
        "this qualification targets the installed Water Channel Reference"
    );
    let server = Server::new(ServerConfig::default());
    server.add_known_plugin(desc.clone());
    let port = server.start("127.0.0.1", 0).unwrap();
    let mut plugin = PluginProcessor::new(PluginMode::Fx);
    plugin.prepare_to_play(48000.0, 1024);
    plugin.connect_to_server("127.0.0.1", port).unwrap();
    plugin.load_plugin(&desc, "Stereo").unwrap();
    assert_eq!(plugin.client.get_all_parameter_values(0).unwrap().len(), 44);
    plugin.client.set_parameter_value(0, 28, 0.0).unwrap(); // Disable compression.
    plugin.client.set_parameter_value(0, 29, 0.0).unwrap(); // Disable EQ.
    plugin.client.set_parameter_value(0, 40, 0.25).unwrap(); // Global output -12 dB.
    let mut midi = Vec::new();
    let mut measured = Vec::new();
    for frames in [64, 127, 256, 512, 1024] {
        let mut sum = 0.;
        for _ in 0..50 {
            let mut audio = vec![vec![0.125; frames], vec![-0.125; frames]];
            plugin.process_block_f32(&mut audio, &mut midi, &PositionInfo::default());
            assert!(audio.iter().flatten().all(|v| v.is_finite()));
            sum = audio[0].iter().map(|v| v.abs() as f64).sum::<f64>() / frames as f64;
            assert!(audio[0]
                .iter()
                .zip(&audio[1])
                .all(|(l, r)| (l + r).abs() < 1e-5));
        }
        assert!(
            sum > 0.01 && sum < 0.05,
            "-12 dB output must actually reach processor: frames={frames}, amplitude={sum}"
        );
        measured.push(sum);
    }
    plugin.client.set_parameter_value(0, 40, 0.3).unwrap();
    let state = plugin.get_state_information();
    let mut restored = PluginProcessor::new(PluginMode::Fx);
    restored.prepare_to_play(48000., 1024);
    restored.connect_to_server("127.0.0.1", port).unwrap();
    restored.set_state_information(&state).unwrap();
    assert!((restored.client.get_parameter_value(0, 40).unwrap() - 0.3).abs() < 1e-6);
    let mut audio = vec![vec![0.125; 1024], vec![-0.125; 1024]];
    for _ in 0..50 {
        audio[0].fill(0.125);
        audio[1].fill(-0.125);
        restored.process_block_f32(&mut audio, &mut midi, &PositionInfo::default());
    }
    let amplitude = audio[0].iter().map(|v| v.abs() as f64).sum::<f64>() / 1024.;
    assert!(amplitude > measured[4] * 1.25 && amplitude < measured[4] * 2.0);
    plugin.client.disconnect();
    restored.client.disconnect();
    server.stop();
}

#[test]
#[ignore = "requires OPENGRIDDER_TEST_COMPAT_VST3 pointing to an installed OpenPreamp VST3"]
fn installed_vst3_compatibility_state_stream_and_float64_transport() {
    let path =
        std::env::var_os("OPENGRIDDER_TEST_COMPAT_VST3").expect("Set OPENGRIDDER_TEST_COMPAT_VST3");
    let desc = vst3::inspect_vst3_bundle(std::path::Path::new(&path))
        .expect("real VST3 factory descriptor");
    let server = Server::new(ServerConfig::default());
    server.add_known_plugin(desc.clone());
    let port = server.start("127.0.0.1", 0).unwrap();
    let mut first = PluginProcessor::new(PluginMode::Fx);
    first.prepare_to_play(48000., 512);
    first.connect_to_server("127.0.0.1", port).unwrap();
    first.load_plugin(&desc, "Stereo").unwrap();
    let state = first.get_state_information();
    let mut second = PluginProcessor::new(PluginMode::Fx);
    second.prepare_to_play(48000., 512);
    second.connect_to_server("127.0.0.1", port).unwrap();
    second.set_state_information(&state).unwrap();
    let original = first.client.get_all_parameter_values(0).unwrap();
    let recalled = second.client.get_all_parameter_values(0).unwrap();
    assert!(!original.is_empty());
    assert_eq!(original.len(), recalled.len());
    for (a, b) in original.iter().zip(&recalled) {
        assert_eq!(a.0, b.0);
        assert!((a.1 - b.1).abs() < 1e-5);
    }
    let mut midi = Vec::new();
    for frames in [1, 64, 127, 512] {
        let mut audio = vec![vec![0.125f64; frames], vec![-0.125; frames]];
        second
            .client
            .process_audio_f64(&mut audio, &mut midi, &PositionInfo::default())
            .unwrap();
        assert!(audio.iter().flatten().all(|v| v.is_finite()));
    }
    // Replacing the state on a connected client must replace the remote chain.
    second.set_state_information(&state).unwrap();
    let serialized: serde_json::Value =
        serde_json::from_slice(&second.get_state_information()).unwrap();
    assert_eq!(serialized["loadedPlugins"].as_array().unwrap().len(), 1);
    first.client.disconnect();
    second.client.disconnect();
    server.stop();
}
