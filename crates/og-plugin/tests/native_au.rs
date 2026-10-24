//! Runs Apple's installed AULowpass through the actual server/client protocol.
#![cfg(target_os = "macos")]
use og_common::{message::PositionInfo, server_plugin::ServerPlugin};
use og_plugin::{PluginMode, PluginProcessor};
use og_server::{Server, ServerConfig};

fn lowpass() -> ServerPlugin {
    let mut desc = ServerPlugin::new(
        "AULowpass",
        "Apple",
        "AudioUnit:aufx:lpas:appl",
        "AULowpass",
        "AudioUnit",
        "Filter",
        false,
        vec!["Stereo".into()],
    );
    desc.num_input_ch = 2;
    desc.num_output_ch = 2;
    desc
}
fn energy(processor: &mut PluginProcessor, frames: usize) -> f64 {
    let mut energy = 0.0;
    let mut midi = Vec::new();
    let settle = 48000 / frames + 1;
    for block in 0..settle + 10 {
        let left: Vec<f32> = (0..frames)
            .map(|n| {
                (std::f64::consts::TAU * 1000.0 * (block * frames + n) as f64 / 48000.0).sin()
                    as f32
                    * 0.25
            })
            .collect();
        let mut audio = vec![left.clone(), left.iter().map(|v| -v).collect()];
        processor.process_block_f32(&mut audio, &mut midi, &PositionInfo::default());
        assert!(audio.iter().flatten().all(|v| v.is_finite()));
        for (l, r) in audio[0].iter().zip(&audio[1]) {
            assert!((l + r).abs() < 1e-5, "stereo channels cross-contaminated");
        }
        if block >= settle {
            energy += audio[0].iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        }
    }
    (energy / (10 * frames) as f64).sqrt()
}
#[test]
fn real_au_network_audio_parameters_and_state_recall() {
    let server = Server::new(ServerConfig::default());
    let desc = lowpass();
    server.add_known_plugin(desc.clone());
    let port = server.start("127.0.0.1", 0).unwrap();
    let mut plugin = PluginProcessor::new(PluginMode::Fx);
    plugin.prepare_to_play(48000.0, 1024);
    plugin.connect_to_server("127.0.0.1", port).unwrap();
    assert!(plugin
        .client
        .add_plugin("missing-plugin", "Stereo", 0)
        .is_err());
    plugin.load_plugin(&desc, "Stereo").unwrap();
    let parameters = plugin.client.get_all_parameter_values(0).unwrap();
    assert_eq!(
        parameters.len(),
        2,
        "expected real lowpass cutoff/resonance parameters"
    );
    plugin.client.set_parameter_value(0, 0, 0.95).unwrap();
    let bright = energy(&mut plugin, 256);
    assert!(bright > 0.1, "bright lowpass output: {bright}");
    let saved = plugin.get_state_information();
    plugin.client.set_parameter_value(0, 0, 0.001).unwrap();
    assert!((plugin.client.get_parameter_value(0, 0).unwrap() - 0.001).abs() < 1e-5);
    for frames in [64, 127, 512, 1024] {
        let dark = energy(&mut plugin, frames);
        assert!(
            dark < bright * 0.05,
            "real filter must attenuate 1 kHz; frames={frames}, RMS={dark}, bright={bright}"
        );
    }
    let mut restored = PluginProcessor::new(PluginMode::Fx);
    restored.prepare_to_play(48000.0, 1024);
    restored.connect_to_server("127.0.0.1", port).unwrap();
    restored.set_state_information(&saved).unwrap();
    assert!((restored.client.get_parameter_value(0, 0).unwrap() - 0.95).abs() < 1e-5);
    assert!((energy(&mut restored, 256) - bright).abs() < 0.002);
    assert!(restored
        .client
        .set_plugin_settings(0, b"invalid state")
        .is_err());
    // Invalid state must not replace the live plugin's valid state.
    assert!((restored.client.get_parameter_value(0, 0).unwrap() - 0.95).abs() < 1e-5);
    plugin.client.disconnect();
    restored.client.disconnect();
    server.stop();
}
