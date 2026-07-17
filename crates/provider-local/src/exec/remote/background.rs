//! Remote long-lived process operations.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use exec_core::{BackgroundOutput, BackgroundStatus, ExecResult};
use exec_protocol::{
    b64_decode, b64_encode, method, ProcessIdParams, ProcessInputParams, ProcessStartParams,
    ProcessStatusParams, ProcessStatusResult, Stream,
};

use super::{from_value, to_value, Conn, RemoteExecutor};

pub(super) async fn start(conn: &Arc<Conn>, command: &str, cwd: &Path) -> ExecResult<String> {
    let process_id = uuid::Uuid::new_v4().to_string();
    conn.call(
        method::PROCESS_START,
        to_value(&ProcessStartParams {
            process_id: process_id.clone(),
            command: command.to_string(),
            cwd: RemoteExecutor::path(cwd),
            timeout_ms: Duration::from_secs(24 * 60 * 60).as_millis() as u64,
            pty: false,
        }),
    )
    .await?;
    Ok(process_id)
}

pub(super) async fn status(
    conn: &Arc<Conn>,
    process_id: &str,
    after_seq: u64,
) -> ExecResult<BackgroundStatus> {
    let value = conn
        .call(
            method::PROCESS_STATUS,
            to_value(&ProcessStatusParams {
                process_id: process_id.to_string(),
                after_seq,
            }),
        )
        .await?;
    let result: ProcessStatusResult = from_value(value)?;
    let mut cursor = after_seq;
    let mut output = Vec::with_capacity(result.output.len());
    for chunk in result.output {
        cursor = cursor.max(chunk.seq);
        output.push(BackgroundOutput {
            seq: chunk.seq,
            is_stderr: chunk.stream == Stream::Stderr,
            data: b64_decode(&chunk.data)?,
        });
    }
    let (exit_code, error) = match result.exit {
        Some(exit) => {
            cursor = cursor.max(exit.seq);
            (Some(exit.code), exit.error)
        }
        None => (None, None),
    };
    Ok(BackgroundStatus {
        output,
        exit_code,
        error,
        cursor,
        truncated: result.truncated_before_seq.is_some(),
    })
}

pub(super) async fn write(
    conn: &Arc<Conn>,
    process_id: &str,
    data: &[u8],
    close: bool,
) -> ExecResult<()> {
    conn.call(
        method::PROCESS_INPUT,
        to_value(&ProcessInputParams {
            process_id: process_id.to_string(),
            data: b64_encode(data),
            close,
        }),
    )
    .await?;
    Ok(())
}

pub(super) async fn kill(conn: &Arc<Conn>, process_id: &str) -> ExecResult<()> {
    conn.call(
        method::PROCESS_CANCEL,
        to_value(&ProcessIdParams {
            process_id: process_id.to_string(),
        }),
    )
    .await?;
    Ok(())
}
