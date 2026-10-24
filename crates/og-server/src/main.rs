//! `AudioGridderServer` standalone executable entry point (`Server/Source/App.cpp`).

use og_common::defaults::DEFAULT_SERVER_PORT;
use og_common::version::AUDIOGRIDDER_VERSION;
use og_server::{Server, ServerConfig};

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("OpenGridder Server {} (Rust / Zero-JUCE / MIT)", AUDIOGRIDDER_VERSION);
        return;
    }

    let server = Server::new(ServerConfig::default());
    let _ = server.load_config();
    let cfg = server.config();
    let port = DEFAULT_SERVER_PORT + (cfg.id.max(0) as u16);

    println!(
        "Starting OpenGridder Server {} on 0.0.0.0:{} (AU={}, VST3={}, LV2={})",
        AUDIOGRIDDER_VERSION, port, cfg.au, cfg.vst3, cfg.lv2
    );
    let count = server.scan_plugins();
    println!("Discovered {} plugins", count);

    if let Err(e) = server.start("0.0.0.0", port) {
        eprintln!("Failed to start server: {}", e);
        std::process::exit(1);
    }

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
