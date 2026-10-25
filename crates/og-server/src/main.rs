//! OpenGridder standalone server. Native chain workers are isolated by default.
use og_common::defaults::DEFAULT_SERVER_PORT;
use og_common::version::AUDIOGRIDDER_VERSION;
use og_hosting::{au, vst3};
use og_server::{SandboxMode, Server, ServerConfig};
use std::path::PathBuf;

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--chain-worker") {
        #[cfg(unix)]
        {
            return og_server::isolation::run_worker();
        }
        #[cfg(not(unix))]
        {
            return Err("Chain workers are not implemented on this platform".into());
        }
    }
    if args.first().map(String::as_str) == Some("--probe-vst3") {
        let path = args.get(1).ok_or("Missing VST3 probe path")?;
        println!(
            "{}",
            serde_json::to_string(&vst3::inspect_vst3_classes(std::path::Path::new(path)))
                .map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!(
            "OpenGridder Server {} (Rust / no JUCE; AU={}, VST3={})",
            AUDIOGRIDDER_VERSION,
            cfg!(target_os = "macos"),
            vst3::hosting_available()
        );
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("OpenGridder Server\n\n--bind ADDRESS        Listen address (default 127.0.0.1)\n--port PORT           Listen port (default 55056 + configured ID; 0 picks a free port)\n--au IDENTIFIER       Serve one installed AU, e.g. AudioUnit:aufx:lpas:appl\n--vst3-bundle PATH    Serve effects from one VST3 bundle\n--list               Print the selected plugin catalog as JSON and exit\n--no-scan            Start with an empty catalog\n--no-isolation       Run chains in server threads (development only)\n--version            Print compiled hosting capabilities\n\nNative chains use child processes by default. VST3 scans have a 10-second timeout.\nRemote access: set --bind 0.0.0.0 on a trusted studio LAN (protocol has no authentication).");
        return Ok(());
    }
    // Load settings before constructing the format manager, so enabled formats/folders apply.
    let mut config = Server::default()
        .load_config()
        .unwrap_or_else(ServerConfig::default);
    config.sandbox_mode = SandboxMode::Chain as i32;
    config.lv2 = false; // No native LV2 host yet; never advertise a fake processor.
    let mut bind = "127.0.0.1".to_string();
    let default_port = DEFAULT_SERVER_PORT
        .checked_add(
            config
                .id
                .max(0)
                .try_into()
                .map_err(|_| "Invalid server ID")?,
        )
        .ok_or("Server port overflow")?;
    let mut port = default_port;
    let mut only_au = None;
    let mut only_vst3: Option<PathBuf> = None;
    let mut list = false;
    let mut rescan = false;
    let mut scan_progress = false;
    let mut index = 0;
    while index < args.len() {
        let option = &args[index];
        match option.as_str() {
            "--bind" | "--port" | "--au" | "--vst3-bundle" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("Missing value for {option}"))?;
                match option.as_str() {
                    "--bind" => bind = value.clone(),
                    "--port" => port = value.parse().map_err(|_| "Invalid port")?,
                    "--au" => only_au = Some(value.clone()),
                    _ => only_vst3 = Some(PathBuf::from(value)),
                }
            }
            "--list" => list = true,
            "--rescan" => rescan = true,
            "--scan-progress" => scan_progress = true,
            "--no-scan" => config.scan_for_plugins = false,
            "--no-isolation" => config.sandbox_mode = SandboxMode::None as i32,
            _ => return Err(format!("Unknown option: {option}")),
        }
        index += 1;
    }
    // Set before starting threads; every module is inspected in its own subprocess.
    std::env::set_var(
        "OPENGRIDDER_VST3_PROBE",
        std::env::current_exe().map_err(|e| e.to_string())?,
    );
    let mut server = Server::new(config.clone());
    server.set_worker_executable(std::env::current_exe().map_err(|e| e.to_string())?);
    if let Some(id) = only_au {
        let desc = au::scan_audiounits()
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("AU not installed: {id}"))?;
        if desc.is_instrument || !desc.id.starts_with("AudioUnit:aufx:") {
            return Err("Only audio effects are supported yet".into());
        }
        server.add_known_plugin(desc);
    } else if let Some(path) = only_vst3 {
        if !vst3::hosting_available() {
            return Err("Rebuild with OPENGRIDDER_VST3_SDK to enable VST3 hosting".into());
        }
        let plugins = vst3::inspect_vst3_classes(&path);
        if plugins.is_empty() {
            return Err(format!(
                "No loadable VST3 effects found in {}",
                path.display()
            ));
        }
        for desc in plugins.into_iter().filter(|p| !p.is_instrument) {
            server.add_known_plugin(desc);
        }
    } else if config.scan_for_plugins {
        if scan_progress {
            let progress_cb = |name: &str, cur: usize, total: usize, fmt: &str| {
                println!(
                    "SCAN_PROGRESS:{{\"current\":{cur},\"total\":{total},\"name\":{name:?},\"format\":{fmt:?}}}"
                );
            };
            server.scan_plugins_with_progress(rescan, Some(&progress_cb));
        } else {
            server.scan_plugins_with_progress(rescan, None);
        }
    }
    if list {
        println!(
            "{}",
            serde_json::to_string_pretty(&server.known_plugins()).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    let port = server.start(&bind, port)?;
    if scan_progress {
        let cat = serde_json::to_string(&server.known_plugins()).unwrap_or_default();
        println!("SCAN_CATALOG:{cat}");
        println!("SCAN_COMPLETE:{{\"total\":{},\"port\":{port}}}", server.known_plugins().len());
    }
    println!(
        "OpenGridder Server listening on {bind}:{port}; {} effects; chain isolation={}",
        server.known_plugins().len(),
        config.sandbox_mode == SandboxMode::Chain as i32
    );
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
fn main() {
    env_logger::init();
    if let Err(error) = run() {
        eprintln!("OpenGridder: {error}");
        std::process::exit(1);
    }
}
