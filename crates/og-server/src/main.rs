//! Listen on the AudioGridder server port. Each handshake starts a worker
//! thread, and the accept loop does not wait for that client to finish.
//! `--sandbox chain` starts one child process per handshake instead.
//! `--sandbox plugin` keeps the worker in this process and hosts each chain
//! in a child. The handshake flag stays clear in that mode.
//! `--id N` adds N to the listen port. The default id is 0, so the port stays 55056.

use std::io::Write;
use std::net::TcpListener;

use og_protocol::SERVER_PORT;
use og_server::{sandbox_chain_child, sandbox_plugin_child, serve_chain, serve_clients, serve_plugin};

/// Clients must send the server's token (see `og_protocol::token`). Without one the server would
/// accept any machine on the network, so a token that cannot be read or created is fatal.
fn require_token() {
    match og_protocol::token::load_or_create() {
        Ok(token) => og_server::require_token(token),
        Err(err) => {
            eprintln!("og-server: could not read or create the access token: {err}");
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--scan-worker") {
        og_server::bundle_scan::worker_main();
        return;
    }
    if args.iter().any(|arg| arg == "--scan-progress") {
        og_server::bundle_scan::PROGRESS.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-v") {
        println!("OpenGridder Server {}", og_server::build_label());
        return;
    }
    if args.iter().any(|arg| arg == "--menu" || arg == "-m" || arg == "menu" || arg == "--status") {
        let id = args.windows(2).find(|pair| pair[0] == "--id").and_then(|pair| pair[1].parse::<u16>().ok()).unwrap_or(0);
        let menu = og_server::menu::ServerMenu::new("AudioGridder Server", id, og_server::build_label());
        print!("{}", menu.render_cli());
        return;
    }
    if args.iter().any(|arg| arg == "--sandbox-chain-child") {
        if let Err(err) = sandbox_chain_child() {
            eprintln!("og-server: {err}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|arg| arg == "--sandbox-plugin-child") {
        if let Err(err) = sandbox_plugin_child() {
            eprintln!("og-server: {err}");
            std::process::exit(1);
        }
        return;
    }
    let sandbox = args.windows(2).find(|pair| pair[0] == "--sandbox").map(|pair| pair[1].as_str());
    if let Some(mode @ ("chain" | "plugin")) = sandbox {
        let requested = args.windows(2).find(|pair| pair[0] == "--port").and_then(|pair| pair[1].parse::<u16>().ok());
        let listener = if requested == Some(0) {
            TcpListener::bind(("127.0.0.1", 0)).unwrap_or_else(|err| {
                eprintln!("og-server: could not bind an ephemeral port: {err}");
                std::process::exit(1);
            })
        } else {
            require_token();
            let port = requested.unwrap_or(SERVER_PORT);
            TcpListener::bind(("0.0.0.0", port)).unwrap_or_else(|err| {
                eprintln!("og-server: could not bind port {port}: {err}");
                std::process::exit(1);
            })
        };
        let port = listener.local_addr().map(|addr| addr.port()).unwrap_or(0);
        if requested == Some(0) {
            println!("master {port}");
            let _ = std::io::stdout().flush();
        }
        eprintln!("OpenGridder Server {} {mode} sandbox on port {port}.", og_server::build_label());
        let served = if mode == "chain" { serve_chain(listener) } else { serve_plugin(listener) };
        if let Err(err) = served {
            eprintln!("og-server: {err}");
            std::process::exit(1);
        }
        return;
    }
    let id = match args.windows(2).find(|pair| pair[0] == "--id") {
        Some(pair) => pair[1].parse::<u16>().unwrap_or_else(|_| {
            eprintln!("og-server: --id needs a port offset");
            std::process::exit(1);
        }),
        None => 0,
    };
    let port = SERVER_PORT.saturating_add(id);
    require_token();
    let listener = TcpListener::bind(("0.0.0.0", port)).unwrap_or_else(|err| {
        eprintln!("og-server: could not bind port {port}: {err}");
        // Tell the server app, which otherwise keeps showing its scan screen.
        let message = if err.kind() == std::io::ErrorKind::AddrInUse {
            format!("Port {port} is already in use by another server (AudioGridder Server, or a second OpenGridder Server). Quit it, then start OpenGridder Server again.")
        } else {
            format!("Could not open port {port}: {err}")
        };
        println!("SERVER_ERROR:{}", serde_json::json!({"message": message, "port": port}));
        let _ = std::io::stdout().flush();
        std::process::exit(1);
    });
    eprintln!(
        "OpenGridder Server {} on port {port}: AudioUnit, VST3, CLAP and LV2 plug-ins.",
        og_server::build_label()
    );
    let rescan = args.iter().any(|arg| arg == "--rescan");
    if rescan {
        og_server::host::rescan(true);
    }
    let no_scan = args.iter().any(|arg| arg == "--no-scan");
    if no_scan {
        println!("SCAN_COMPLETE:{}", serde_json::json!({"total": 0, "port": port}));
        let _ = std::io::stdout().flush();
    } else {
        // The startup scan loads new or changed VST3/CLAP bundles in scan worker processes.
        og_server::bundle_scan::WORKERS.store(true, std::sync::atomic::Ordering::Relaxed);
        let plugins = og_server::host::scan();
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
    let mdns_info = og_server::mdns::ServiceInfo {
        id: id as i32,
        uuid: format!("{:08x}-0000-4000-8000-{:012x}", std::process::id(), port),
        local_mode: false,
        load: 0.0,
        version: og_server::VERSION.to_string(),
        port,
        hostname: host_name,
    };
    // Bonjour through the system's mDNSResponder (the hand-written responder broadcast
    // from a random port without answering queries, so macOS clients never saw it).
    if !og_server::ui::advertise(port, &mdns_info.txt_record()) {
        eprintln!("og-server: could not advertise the server with Bonjour");
    }
    // New or changed VST3/CLAP bundles are loaded in scan workers while the server already
    // serves (a fresh machine's first scan can take a long time: licence dialogs, Gatekeeper).
    if !no_scan {
        og_server::host::scan_bundles_in_background();
    }
    // Clients are served on a background thread; the main thread runs the AppKit loop
    // that shows and drives the plugins' editor windows.
    std::thread::spawn(move || {
        if let Err(err) = serve_clients(listener) {
            eprintln!("og-server: {err}");
            std::process::exit(1);
        }
        std::process::exit(0);
    });
    og_server::ui::run();
}
