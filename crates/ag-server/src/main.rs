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
    if args.iter().any(|arg| arg == "--menu" || arg == "-m" || arg == "menu" || arg == "--status") {
        let id = args.windows(2).find(|pair| pair[0] == "--id").and_then(|pair| pair[1].parse::<u16>().ok()).unwrap_or(0);
        let menu = ag_server::menu::ServerMenu::new("AudioGridder Server", id, "0.1.0-alpha");
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
        eprintln!("AudioGridder server 0.1.0-alpha chain sandbox on port {port}.");
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
        eprintln!("AudioGridder server 0.1.0-alpha plugin sandbox on port {port}.");
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
        "AudioGridder server 0.1.0-alpha on port {port}. Hosts builtin gain plus scanned AudioUnit, LV2, and VST3 plug-ins."
    );
    if let Err(err) = serve_clients(listener) {
        eprintln!("ag-server: {err}");
        std::process::exit(1);
    }
}
