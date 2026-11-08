//! Listen on the AudioGridder server port. Each handshake starts a worker
//! thread, and the accept loop does not wait for that client to finish.
//! `--sandbox chain` starts one child process per handshake instead.
//! `--sandbox plugin` keeps the worker in this process and hosts each chain
//! in a child. The handshake flag stays clear in that mode.
//! `--id N` adds N to the listen port. The default id is 0, so the port stays 55056.

use std::io::Write;
use std::net::TcpListener;

use ag_protocol::SERVER_PORT;
use ag_server::{sandbox_chain_child, sandbox_plugin_child, serve_chain, serve_clients, serve_plugin};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--scan-worker") {
        ag_server::bundle_scan::worker_main();
        return;
    }
    if args.iter().any(|arg| arg == "--scan-progress") {
        ag_server::bundle_scan::PROGRESS.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-v") {
        println!("OpenGridder Server {}", ag_server::build_label());
        return;
    }
    if args.iter().any(|arg| arg == "--menu" || arg == "-m" || arg == "menu" || arg == "--status") {
        let id = args.windows(2).find(|pair| pair[0] == "--id").and_then(|pair| pair[1].parse::<u16>().ok()).unwrap_or(0);
        let menu = ag_server::menu::ServerMenu::new("AudioGridder Server", id, ag_server::build_label());
        print!("{}", menu.render_cli());
        return;
    }
    if args.iter().any(|arg| arg == "--sandbox-chain-child") {
        if let Err(err) = sandbox_chain_child() {
            eprintln!("ag-server: {err}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|arg| arg == "--sandbox-plugin-child") {
        if let Err(err) = sandbox_plugin_child() {
            eprintln!("ag-server: {err}");
            std::process::exit(1);
        }
        return;
    }
    if args.windows(2).any(|pair| pair[0] == "--sandbox" && pair[1] == "chain") {
        let requested = args.windows(2).find(|pair| pair[0] == "--port").and_then(|pair| pair[1].parse::<u16>().ok());
        let listener = if requested == Some(0) {
            TcpListener::bind(("127.0.0.1", 0)).unwrap_or_else(|err| {
                eprintln!("ag-server: could not bind an ephemeral port: {err}");
                std::process::exit(1);
            })
        } else {
            let port = requested.unwrap_or(SERVER_PORT);
            TcpListener::bind(("0.0.0.0", port)).unwrap_or_else(|err| {
                eprintln!("ag-server: could not bind port {port}: {err}");
                std::process::exit(1);
            })
        };
        let port = listener.local_addr().map(|addr| addr.port()).unwrap_or(0);
        if requested == Some(0) {
            println!("master {port}");
            let _ = std::io::stdout().flush();
        }
        eprintln!("AudioGridder server {} chain sandbox on port {port}.", ag_server::build_label());
        if let Err(err) = serve_chain(listener) {
            eprintln!("ag-server: {err}");
            std::process::exit(1);
        }
        return;
    }
    if args.windows(2).any(|pair| pair[0] == "--sandbox" && pair[1] == "plugin") {
        let requested = args.windows(2).find(|pair| pair[0] == "--port").and_then(|pair| pair[1].parse::<u16>().ok());
        let listener = if requested == Some(0) {
            TcpListener::bind(("127.0.0.1", 0)).unwrap_or_else(|err| {
                eprintln!("ag-server: could not bind an ephemeral port: {err}");
                std::process::exit(1);
            })
        } else {
            let port = requested.unwrap_or(SERVER_PORT);
            TcpListener::bind(("0.0.0.0", port)).unwrap_or_else(|err| {
                eprintln!("ag-server: could not bind port {port}: {err}");
                std::process::exit(1);
            })
        };
        let port = listener.local_addr().map(|addr| addr.port()).unwrap_or(0);
        if requested == Some(0) {
            println!("master {port}");
            let _ = std::io::stdout().flush();
        }
        eprintln!("AudioGridder server {} plugin sandbox on port {port}.", ag_server::build_label());
        if let Err(err) = serve_plugin(listener) {
            eprintln!("ag-server: {err}");
            std::process::exit(1);
        }
        return;
    }
    let id = match args.windows(2).find(|pair| pair[0] == "--id") {
        Some(pair) => pair[1].parse::<u16>().unwrap_or_else(|_| {
            eprintln!("ag-server: --id needs a port offset");
            std::process::exit(1);
        }),
        None => 0,
    };
    let port = SERVER_PORT.saturating_add(id);
    let listener = TcpListener::bind(("0.0.0.0", port)).unwrap_or_else(|err| {
        eprintln!("ag-server: could not bind port {port}: {err}");
        std::process::exit(1);
    });
    eprintln!(
        "AudioGridder server {} on port {port}. Hosts builtin gain plus scanned AudioUnit, LV2, and VST3 plug-ins.",
        ag_server::build_label()
    );
    let rescan = args.iter().any(|arg| arg == "--rescan");
    if rescan {
        ag_server::host::rescan(true);
    }
    let no_scan = args.iter().any(|arg| arg == "--no-scan");
    if no_scan {
        println!("SCAN_COMPLETE:{}", serde_json::json!({"total": 0, "port": port}));
        let _ = std::io::stdout().flush();
    } else {
        // The startup scan loads new or changed VST3/CLAP bundles in scan worker processes.
        ag_server::bundle_scan::WORKERS.store(true, std::sync::atomic::Ordering::Relaxed);
        let plugins = ag_server::host::scan();
        for (i, p) in plugins.iter().enumerate() {
            println!(
                "SCAN_PROGRESS:{}",
                serde_json::json!({
                    "current": i + 1,
                    "total": plugins.len(),
                    "name": p.name,
                    "format": p.format
                })
            );
            let _ = std::io::stdout().flush();
        }
        let catalog_json = serde_json::to_string(
            &plugins
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "id": p.id,
                        "name": p.name,
                        "company": p.company,
                        "format": p.format,
                        "category": p.category,
                        "is_instrument": p.is_instrument
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());
        println!("SCAN_CATALOG:{catalog_json}");
        let _ = std::io::stdout().flush();
        println!("SCAN_COMPLETE:{}", serde_json::json!({"total": plugins.len(), "port": port}));
        let _ = std::io::stdout().flush();
    }
    let host_name = std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string());
    let mdns_info = ag_server::mdns::ServiceInfo {
        id: id as i32,
        uuid: format!("{:08x}-0000-4000-8000-{:012x}", std::process::id(), port),
        local_mode: false,
        load: 0.0,
        version: ag_server::VERSION.to_string(),
        port,
        hostname: host_name,
    };
    // Bonjour through the system's mDNSResponder (the hand-written responder broadcast
    // from a random port without answering queries, so macOS clients never saw it).
    if !ag_server::ui::advertise(port, &mdns_info.txt_record()) {
        eprintln!("ag-server: could not advertise the server with Bonjour");
    }
    // Clients are served on a background thread; the main thread runs the AppKit loop
    // that shows and drives the plugins' editor windows.
    std::thread::spawn(move || {
        if let Err(err) = serve_clients(listener) {
            eprintln!("ag-server: {err}");
            std::process::exit(1);
        }
        std::process::exit(0);
    });
    ag_server::ui::run();
}
