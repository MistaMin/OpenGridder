//! `og-server`: Complete Rust rewrite of `AudioGridderServer` (v1.2.1-marcosfork).

pub mod audio_worker;
#[cfg(unix)]
pub mod isolation;
pub mod processor;
pub mod processor_chain;
pub mod sandbox;
pub mod screen_worker;
pub mod server;
pub mod worker;

pub use processor::Processor;
pub use processor_chain::ProcessorChain;
pub use sandbox::SandboxMode;
pub use server::{Server, ServerConfig};
pub use worker::Worker;

#[cfg(test)]
mod tests {
    use super::*;
    use og_common::server_plugin::ServerPlugin;
    use og_hosting::PluginFormatManager;

    #[test]
    fn test_server_config_save_persistence_and_processor_oversampling() {
        let tmp_cfg = std::env::temp_dir().join("opengridder_test_server.cfg");
        let _ = std::fs::remove_file(&tmp_cfg);

        let mut srv = Server::new(ServerConfig::default());
        srv.set_config_path(tmp_cfg.clone());

        let mut custom = srv.config();
        custom.name = "Marcos Studio Server".to_string();
        custom.id = 3;
        custom.sandbox_mode = SandboxMode::Chain as i32;
        srv.apply_and_save_config(custom)
            .expect("save_config should succeed");

        let mut srv2 = Server::new(ServerConfig::default());
        srv2.set_config_path(tmp_cfg.clone());
        let loaded = srv2.load_config().expect("load_config should succeed");
        assert_eq!(loaded.name, "Marcos Studio Server");
        assert_eq!(loaded.id, 3);
        assert_eq!(loaded.sandbox_mode, SandboxMode::Chain as i32);
        let _ = std::fs::remove_file(&tmp_cfg);

        // Verify Processor oversampling factor clamp and latency
        let fmt = PluginFormatManager::new_all_enabled();
        let desc = ServerPlugin::new(
            "TestEQ",
            "OpenGridder",
            "OpenGridder:Gain",
            "OpenGridder:Gain",
            "OpenGridder",
            "EQ",
            false,
            vec!["Stereo".to_string()],
        );
        let mut proc = Processor::new(&fmt, &desc, "Stereo", 0, 48000.0, 256).unwrap();
        assert_eq!(proc.oversampling_factor(), 1);
        assert_eq!(proc.latency_samples(), 0);

        proc.set_oversampling_factor(2);
        assert_eq!(proc.oversampling_factor(), 2);
        assert_eq!(proc.latency_samples(), 54);

        proc.set_oversampling_factor(4);
        assert_eq!(proc.oversampling_factor(), 4);
        assert_eq!(proc.latency_samples(), 59);
        proc.set_plugin_latency_samples(19);
        assert_eq!(
            proc.latency_samples(),
            64,
            "round plugin delay up from the 4x processing rate"
        );
        proc.set_plugin_latency_samples(0);

        // Invalid factor clamps back to 1x
        proc.set_oversampling_factor(99);
        assert_eq!(proc.oversampling_factor(), 1);
        assert_eq!(proc.latency_samples(), 0);
    }
}
