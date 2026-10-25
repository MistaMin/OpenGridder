//! `og-plugin`: Complete Rust rewrite of the AudioGridder DAW Plugin (`Fx`, `Instrument`, `Midi`)
//! with `GoodLookinUI` hardware rendering and per-plugin `1x`/`2x`/`4x` oversampling (v1.2.1-marcosfork).

pub mod editor;
pub mod plugin_formats;
pub mod processor;
pub mod native;
pub mod drive;
mod spsc;

pub use editor::PluginEditor;
pub use processor::{
    AutomationSlot, LoadedPlugin, PluginMode, PluginProcessor, NUM_AUTOMATION_SLOTS, STATE_VERSION,
};

#[cfg(test)]
mod tests {
    use super::*;
    use og_common::message::PositionInfo;
    use og_common::server_plugin::{PluginLayout, ServerPlugin};
    use og_hosting::vst3::default_plugin_parameters;
    use og_server::{Server, ServerConfig};

    #[test]
    fn test_end_to_end_server_plugin_oversampling_and_state_v7() {
        // 1. Start an OpenGridder Server on an ephemeral port with a registered test plugin
        let mut srv_cfg = ServerConfig::default();
        srv_cfg.scan_for_plugins = false;
        let server = Server::new(srv_cfg);

        let test_plugin = ServerPlugin {
            module_path: String::new(),
            name: "Marcos Saturator".to_string(),
            company: "MistaMin DSP".to_string(),
            id: "OpenGridder:Gain".to_string(),
            id_deprecated: "Marcos Saturator".to_string(),
            plugin_type: "OpenGridder".to_string(),
            category: "Fx".to_string(),
            version: "1.2.1".to_string(),
            is_instrument: false,
            unique_id: 0x559863f,
            num_input_ch: 2,
            num_output_ch: 2,
            presets: vec!["Default".to_string(), "Warm".to_string()],
            parameters: default_plugin_parameters(),
            layouts: vec![PluginLayout {
                inputs: vec!["Stereo".to_string()],
                outputs: vec!["Stereo".to_string()],
            }],
            has_editor: true,
            scoped_id: "VST3:VST3-marcos-sat".to_string(),
        };
        server.add_known_plugin(test_plugin.clone());

        let port = server.start("127.0.0.1", 0).expect("server should start");

        // 2. Connect DAW PluginProcessor to Server
        let mut proc = PluginProcessor::new(PluginMode::Fx);
        proc.prepare_to_play(48000.0, 128);
        proc.connect_to_server("127.0.0.1", port)
            .expect("client should connect and complete handshake v13");

        let avail = proc.client.available_plugins();
        assert_eq!(avail.len(), 1);
        assert_eq!(avail[0].name, "Marcos Saturator");

        // 3. Load the plugin into the remote chain
        let idx = proc
            .load_plugin(&test_plugin, "Stereo")
            .expect("load_plugin should succeed");
        assert_eq!(idx, 0);
        assert_eq!(proc.get_plugin_oversampling(0), 1);
        assert_eq!(proc.latency_samples, 0);

        // 4. Stream f32 audio at 1x oversampling
        let mut audio = vec![vec![0.5f32; 128], vec![-0.25f32; 128]];
        let mut midi = Vec::new();
        let pos = PositionInfo::default();
        proc.process_block_f32(&mut audio, &mut midi, &pos);
        assert!((audio[0][64] - 0.5).abs() < 1e-5);
        assert!((audio[1][64] + 0.25).abs() < 1e-5);

        // 5. Render GoodLookinUI PluginEditor and click the Oversampling badge to cycle 1x -> 2x -> 4x
        let mut editor = PluginEditor::new(720, 420);
        editor.render(&proc);
        assert_eq!(editor.buttons.len(), 1);

        // Button bounds are Rect::new(28.0, 54.0, 190.0, 28.0):
        // Right = 218, h = 28:
        // Delete: 190..218, MoveDown: 162..190, MoveUp: 134..162, Bypass: 106..134, Oversampling: 78..106
        let clicked_2x = editor.handle_mouse_click(&mut proc, 90.0, 68.0);
        assert!(clicked_2x);
        assert_eq!(proc.get_plugin_oversampling(0), 2);
        assert_eq!(proc.latency_samples, 12); // 2x FIR latency = 12 samples

        let clicked_4x = editor.handle_mouse_click(&mut proc, 90.0, 68.0);
        assert!(clicked_4x);
        assert_eq!(proc.get_plugin_oversampling(0), 4);
        assert_eq!(proc.latency_samples, 18); // 4x cascaded FIR latency = 18 samples

        // 6. Stream audio at 4x oversampling and verify DC convergence and latency reporting
        let mut dc_audio = vec![vec![0.5f32; 128], vec![0.5f32; 128]];
        proc.process_block_f32(&mut dc_audio, &mut midi, &pos);
        assert!((dc_audio[0][100] - 0.5).abs() < 1e-4);
        assert_eq!(proc.latency_samples, 18);

        // 7. Serialize DAW state (STATE_VERSION = 7) and verify index 11 is oversampling_factor = 4
        let state_bytes = proc.get_state_information();
        let state_json: serde_json::Value =
            serde_json::from_slice(&state_bytes).expect("valid state JSON");
        assert_eq!(
            state_json.get("version").and_then(|v| v.as_i64()),
            Some(7)
        );
        let lp_arr = state_json["loadedPlugins"][0]
            .as_array()
            .expect("LoadedPlugin array");
        assert_eq!(lp_arr.len(), 12);
        assert_eq!(lp_arr[11].as_i64(), Some(4)); // DATA_IDX::OVERSAMPLING_FACTOR = 11

        // 8. Restore state v7 into a fresh PluginProcessor and verify 4x oversampling is restored & synced
        let mut proc2 = PluginProcessor::new(PluginMode::Fx);
        proc2.prepare_to_play(48000.0, 128);
        proc2
            .connect_to_server("127.0.0.1", port)
            .expect("connect proc2");
        proc2
            .set_state_information(&state_bytes)
            .expect("restore state v7");
        assert_eq!(proc2.get_plugin_oversampling(0), 4);

        server.stop();
    }
}
