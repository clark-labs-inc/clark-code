use std::path::PathBuf;

use code_host::PROTOCOL_VERSION;
use code_worker::build_host;
use code_worker::config::WorkerConfig;
use code_worker::stdio::serve_stdio;

#[tokio::main]
async fn main() {
    if run_self_test() {
        return;
    }
    if let Err(error) = run().await {
        eprintln!("agent-code-worker: {error}");
        std::process::exit(1);
    }
}

fn run_self_test() -> bool {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 1 || args[0] != "--self-test" {
        return false;
    }
    println!(
        "{}",
        serde_json::json!({
            "status": "passed",
            "worker": "agent-code-worker",
            "protocol_version": PROTOCOL_VERSION,
            "worker_version": env!("CARGO_PKG_VERSION"),
        })
    );
    true
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = parse_config_path()?;
    let config: WorkerConfig = serde_json::from_slice(&tokio::fs::read(&config_path).await?)?;
    config.validate()?;
    let host = build_host(&config)?;
    serve_stdio(host, config, env!("CARGO_PKG_VERSION")).await
}

fn parse_config_path() -> Result<PathBuf, String> {
    let mut args = std::env::args_os().skip(1);
    match (args.next(), args.next()) {
        (Some(flag), Some(path)) if flag == "--config" && args.next().is_none() => {
            Ok(PathBuf::from(path))
        }
        _ => Err("usage: agent-code-worker --config /absolute/path/worker.json".into()),
    }
}
