use std::path::PathBuf;

use code_host::{Request, RequestCommand, Response, PROTOCOL_VERSION};
use code_worker::build_host;
use code_worker::config::{ConfigError, WorkerConfig};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;

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
    let registered = host
        .catalog()
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(plugin) = config
        .enabled_plugins
        .iter()
        .find(|plugin| !registered.contains(*plugin))
    {
        return Err(ConfigError::UnknownPlugin((*plugin).clone()).into());
    }
    let (output_tx, mut output_rx) = mpsc::channel::<Response>(64);
    let max_response_bytes = config.max_response_bytes;
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(response) = output_rx.recv().await {
            let line = bounded_response_line(response, max_response_bytes);
            if stdout.write_all(&line).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    });

    let mut input = BufReader::new(tokio::io::stdin());
    let mut tasks = JoinSet::new();
    let permits = std::sync::Arc::new(Semaphore::new(config.max_concurrent_requests));
    let mut shutting_down = false;
    while !shutting_down {
        // JoinSet retains completed tasks until they are explicitly reaped.
        // Without this, a burst of short requests eventually looks like a
        // permanently saturated worker even though no invocation is active.
        while tasks.try_join_next().is_some() {}
        let line = read_bounded_line(&mut input, config.max_request_bytes).await?;
        let Some(line) = line else { break };
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let line = match std::str::from_utf8(&line) {
            Ok(line) => line,
            Err(error) => {
                let _ = output_tx
                    .send(Response::error(None, "invalid_request", error.to_string()))
                    .await;
                continue;
            }
        };
        let request = match Request::from_json_str(line) {
            Ok(request) => request,
            Err(error) => {
                let _ = output_tx
                    .send(Response::error(None, "invalid_request", error))
                    .await;
                continue;
            }
        };
        let request_id = request.request_id.clone();
        let shutdown = matches!(request.command, RequestCommand::Shutdown);
        let permit = if shutdown {
            None
        } else {
            match permits.clone().try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => {
                    output_tx
                        .send(Response::error(
                            Some(request_id),
                            "busy",
                            "worker request concurrency limit reached",
                        ))
                        .await
                        .map_err(|_| "worker output channel closed")?;
                    continue;
                }
            }
        };
        let ping = matches!(&request.command, RequestCommand::Ping);
        let worker_name = config.worker_name.clone();
        let model = config.provider.model.clone();
        let execution_residency = config.execution_residency;
        let host = host.clone();
        let output = output_tx.clone();
        tasks.spawn(async move {
            let _permit = permit;
            let mut response = host.handle_stream(request, output.clone()).await;
            if ping {
                if let Response::Result { data, .. } = &mut response {
                    data["worker"] = serde_json::Value::String(worker_name);
                    data["worker_version"] =
                        serde_json::Value::String(env!("CARGO_PKG_VERSION").into());
                    data["model"] = serde_json::Value::String(model);
                    data["execution_residency"] =
                        serde_json::to_value(execution_residency).expect("residency serializes");
                }
            }
            let _ = output.send(response.with_request_id(request_id)).await;
        });
        if shutdown {
            shutting_down = true;
        }
    }
    while tasks.join_next().await.is_some() {}
    drop(output_tx);
    let _ = writer.await;
    Ok(())
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

async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    maximum_bytes: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::with_capacity(8 * 1024);
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(line))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if line.len().saturating_add(consumed) > maximum_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("control request exceeds {maximum_bytes} bytes"),
            ));
        }
        line.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        if newline.is_some() {
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            return Ok(Some(line));
        }
    }
}

fn bounded_response_line(response: Response, maximum_bytes: usize) -> Vec<u8> {
    let request_id = response.request_id().map(ToOwned::to_owned);
    let mut line = serde_json::to_vec(&response).expect("response serializes");
    if line.len().saturating_add(1) > maximum_bytes {
        line = serde_json::to_vec(&Response::error(
            request_id,
            "response_too_large",
            format!("worker response exceeds {maximum_bytes} bytes"),
        ))
        .expect("bounded error serializes");
    }
    line.push(b'\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_response_has_one_line() {
        let line = bounded_response_line(Response::result(None, "ok", serde_json::json!({})), 1024);
        assert_eq!(line.last(), Some(&b'\n'));
        assert_eq!(
            line[..line.len() - 1]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count(),
            0
        );
    }
}
