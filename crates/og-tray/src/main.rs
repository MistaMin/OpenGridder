//! `AudioGridderPluginTray` executable entry point (`PluginTray/Source/App.cpp`).

use og_common::defaults::DEFAULT_TRAY_PORT;
use og_common::version::AUDIOGRIDDER_VERSION;
use og_tray::PluginTrayServer;

fn main() {
    env_logger::init();
    println!(
        "Starting OpenGridder PluginTray {} on 127.0.0.1:{}",
        AUDIOGRIDDER_VERSION, DEFAULT_TRAY_PORT
    );
    let tray = PluginTrayServer::new();
    if let Err(e) = tray.start("127.0.0.1", DEFAULT_TRAY_PORT) {
        eprintln!("Failed to start PluginTray: {}", e);
        std::process::exit(1);
    }
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
