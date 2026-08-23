use code_host::{HeadlessHost, Request, RequestCommand, Response};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;

use crate::config::WorkerConfig;

/// Serve one fully composed worker host over the canonical bounded JSONL
/// stdin/stdout transport. Branded worker binaries reuse this transport while
/// registering their own compile-time session extensions.
pub async fn serve_stdio(
    host: HeadlessHost,
    config: WorkerConfig,
    worker_version: &'static str,
) -> Result<(), Box<dyn std::error::Error>> {
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
        return Err(crate::config::ConfigError::UnknownPlugin((*plugin).clone()).into());
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
        while tasks.try_join_next().is_some() {}
        let line = match read_bounded_line(&mut input, config.max_request_bytes).await? {
            BoundedLine::Line(line) => line,
            BoundedLine::Eof => break,
            BoundedLine::Oversized => {
                // One oversized request is that caller's error, not grounds to
                // exit the process — this worker serves every session on the
                // host, and `?`-ing out of the serve loop killed them all.
                // The id is unknowable without buffering the line being
                // refused, so the error is unaddressed; the desktop reader
                // skips frames without a request id.
                let _ = output_tx
                    .send(Response::error(
                        None,
                        "request_too_large",
                        format!("control request exceeds {} bytes", config.max_request_bytes),
                    ))
                    .await;
                continue;
            }
        };
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
        let ping = matches!(&request.command, RequestCommand::Ping);
        // Ping answers without a permit, like Shutdown. A saturated worker is
        // mid-work by definition, and health checks exist to be answered at
        // exactly that moment — refusing them with `busy` made the desktop
        // classify a fully loaded worker as dead, replace it, and cancel every
        // invocation it was busy with.
        let permit = if shutdown || ping {
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
        let worker_name = config.worker_name.clone();
        let worker_version = worker_version.to_string();
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
                    data["worker_version"] = serde_json::Value::String(worker_version);
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

enum BoundedLine {
    Line(Vec<u8>),
    /// The line exceeded the bound. Its bytes were drained up to and including
    /// the newline without being buffered, so the loop can keep serving.
    Oversized,
    Eof,
}

async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    maximum_bytes: usize,
) -> std::io::Result<BoundedLine> {
    let mut line = Vec::with_capacity(8 * 1024);
    let mut oversized = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(if oversized {
                BoundedLine::Oversized
            } else if line.is_empty() {
                BoundedLine::Eof
            } else {
                BoundedLine::Line(line)
            });
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if !oversized && line.len().saturating_add(consumed) > maximum_bytes {
            oversized = true;
            line = Vec::new();
        }
        if !oversized {
            line.extend_from_slice(&available[..consumed]);
        }
        reader.consume(consumed);
        if newline.is_some() {
            if oversized {
                return Ok(BoundedLine::Oversized);
            }
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            return Ok(BoundedLine::Line(line));
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

    #[tokio::test]
    async fn an_oversized_line_is_drained_and_the_reader_keeps_serving() {
        // One oversized request must cost only that request. Erroring out of
        // the read (and therefore the serve loop) exited the worker process,
        // killing every session on the host.
        let oversized = "x".repeat(64);
        let feed = format!("{oversized}\n{{\"ok\":1}}\n");
        let mut reader = tokio::io::BufReader::new(std::io::Cursor::new(feed.into_bytes()));

        assert!(matches!(
            read_bounded_line(&mut reader, 16).await.unwrap(),
            BoundedLine::Oversized
        ));
        // The bytes were consumed up to the newline: the next read yields the
        // following, well-sized line intact.
        match read_bounded_line(&mut reader, 16).await.unwrap() {
            BoundedLine::Line(line) => assert_eq!(line, b"{\"ok\":1}"),
            other => panic!("expected the next line, got {}", kind_of(&other)),
        }
        assert!(matches!(
            read_bounded_line(&mut reader, 16).await.unwrap(),
            BoundedLine::Eof
        ));
    }

    #[tokio::test]
    async fn an_oversized_final_line_without_newline_still_reports_oversized() {
        let feed = "y".repeat(64);
        let mut reader = tokio::io::BufReader::new(std::io::Cursor::new(feed.into_bytes()));
        assert!(matches!(
            read_bounded_line(&mut reader, 16).await.unwrap(),
            BoundedLine::Oversized
        ));
    }

    fn kind_of(line: &BoundedLine) -> &'static str {
        match line {
            BoundedLine::Line(_) => "Line",
            BoundedLine::Oversized => "Oversized",
            BoundedLine::Eof => "Eof",
        }
    }

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
