//! ag-tray daemon entry point.
use ag_protocol::TRAY_PORT;
use ag_tray::PluginTrayServer;

fn main() {
    println!("AudioGridder PluginTray 0.1.0-alpha on 127.0.0.1:{}", TRAY_PORT);
    let tray = PluginTrayServer::new();
    if let Err(e) = tray.start("127.0.0.1", TRAY_PORT) {
        eprintln!("Failed to start ag-tray: {}", e);
        std::process::exit(1);
    }
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
