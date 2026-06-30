//! The `clark-exec-server` binary: parse the environment, bind a loopback port,
//! announce the bound URL on stdout, then serve until killed.
//!
//! Launched on the remote host by the desktop's SSH orchestrator, roughly:
//!
//! ```sh
//! CLARK_EXEC_TOKEN=<secret> clark-exec-server --root /home/me/project
//! ```
//!
//! The token arrives via the environment (never argv, so it can't leak through
//! `ps`). On startup it prints exactly one line:
//!
//! ```text
//! CLARK_EXEC_SERVER_URL=ws://127.0.0.1:<port>
//! ```
//!
//! which the orchestrator parses to learn the ephemeral port to `ssh -L`-forward.

use std::path::PathBuf;

use exec_server::{bind, Config};

#[tokio::main]
async fn main() {
    let token = match std::env::var("CLARK_EXEC_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            eprintln!("clark-exec-server: CLARK_EXEC_TOKEN must be set");
            std::process::exit(2);
        }
    };

    // `--root <path>` confines all file ops; falls back to $CLARK_EXEC_ROOT.
    let mut root: Option<PathBuf> = std::env::var_os("CLARK_EXEC_ROOT").map(PathBuf::from);
    // `--listen <addr>` overrides the default loopback ephemeral bind.
    let mut addr = "127.0.0.1:0".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = args.next().map(PathBuf::from),
            "--listen" => {
                if let Some(a) = args.next() {
                    addr = a;
                }
            }
            other => {
                eprintln!("clark-exec-server: unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    // Canonicalize the root so containment compares against a real absolute path.
    let root = root.map(|r| std::fs::canonicalize(&r).unwrap_or(r));

    let server = match bind(Config { token, root, addr }).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("clark-exec-server: bind failed: {e}");
            std::process::exit(1);
        }
    };

    match server.local_addr() {
        Ok(a) => println!("CLARK_EXEC_SERVER_URL=ws://{a}"),
        Err(e) => {
            eprintln!("clark-exec-server: local_addr failed: {e}");
            std::process::exit(1);
        }
    }
    // Flush the URL line immediately — the orchestrator blocks on reading it.
    use std::io::Write;
    let _ = std::io::stdout().flush();

    server.serve().await;
}
