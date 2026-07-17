//! Long-lived process registry and resumable output streaming.

use std::{
    collections::VecDeque,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use exec_core::{isolate_process_group, terminate_process_tree, Executor, LocalExecutor};
use exec_protocol::{
    b64_decode, b64_encode, error_code, method, Notification, ProcessExitParams, ProcessIdParams,
    ProcessInputParams, ProcessOutputParams, ProcessResumeParams, ProcessStartParams,
    ProcessStatusParams, ProcessStatusResult, Response, Stream,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::{checked_path, parse, text_msg, to_value, Outbound, Shared};

const MAX_PROCESSES: usize = 64;
const MAX_PROCESS_OUTPUT_BYTES: usize = 1_048_576;

pub(super) struct ProcShared {
    process_id: String,
    state: Mutex<ProcState>,
    tick: broadcast::Sender<()>,
    cancel: CancellationToken,
    input: mpsc::UnboundedSender<ProcessInput>,
}

#[derive(Default)]
struct ProcState {
    output: VecDeque<ProcessOutputParams>,
    output_bytes: usize,
    next_seq: u64,
    dropped_through_seq: Option<u64>,
    exit: Option<ProcessExitParams>,
}

struct ProcessInput {
    data: Vec<u8>,
    close: bool,
}

pub(super) fn handle_start(
    id: u64,
    params: serde_json::Value,
    shared: &Arc<Shared>,
    tx: &Outbound,
    conn_token: &CancellationToken,
) {
    let p: ProcessStartParams = match parse(params) {
        Ok(p) => p,
        Err((code, msg)) => {
            let _ = tx.send(text_msg(&Response::err(id, code, msg)));
            return;
        }
    };
    let cwd = match checked_path(&p.cwd, &shared.config.root) {
        Ok(c) => c,
        Err((code, msg)) => {
            let _ = tx.send(text_msg(&Response::err(id, code, msg)));
            return;
        }
    };

    let (tick, _) = broadcast::channel(16);
    let (input, input_rx) = mpsc::unbounded_channel();
    let proc = Arc::new(ProcShared {
        process_id: p.process_id.clone(),
        state: Mutex::new(ProcState::default()),
        tick,
        cancel: CancellationToken::new(),
        input,
    });

    {
        let mut procs = shared.procs.lock().unwrap();
        if procs.contains_key(&p.process_id) {
            let _ = tx.send(text_msg(&Response::err(
                id,
                error_code::EXEC_FAILED,
                "process_id already in use",
            )));
            return;
        }
        if procs.len() >= MAX_PROCESSES {
            procs.retain(|_, process| process.state.lock().unwrap().exit.is_none());
        }
        if procs.len() >= MAX_PROCESSES {
            let _ = tx.send(text_msg(&Response::err(
                id,
                error_code::EXEC_FAILED,
                "too many retained processes",
            )));
            return;
        }
        procs.insert(p.process_id.clone(), proc.clone());
    }

    let _ = tx.send(text_msg(&Response::ok(id, serde_json::json!({}))));

    tokio::spawn(run_process(
        proc.clone(),
        shared.clone(),
        p.command,
        cwd,
        Duration::from_millis(p.timeout_ms),
        p.pty,
        input_rx,
    ));
    spawn_streamer(proc, tx.clone(), 0, conn_token.clone());
}

pub(super) fn handle_status(
    id: u64,
    params: serde_json::Value,
    shared: &Arc<Shared>,
    tx: &Outbound,
) {
    let p: ProcessStatusParams = match parse(params) {
        Ok(p) => p,
        Err((code, msg)) => {
            let _ = tx.send(text_msg(&Response::err(id, code, msg)));
            return;
        }
    };
    let proc = shared.procs.lock().unwrap().get(&p.process_id).cloned();
    let Some(proc) = proc else {
        let _ = tx.send(text_msg(&Response::err(
            id,
            error_code::UNKNOWN_PROCESS,
            "unknown or expired process",
        )));
        return;
    };
    let result = {
        let state = proc.state.lock().unwrap();
        let output = state
            .output
            .iter()
            .filter(|chunk| chunk.seq > p.after_seq)
            .cloned()
            .collect();
        let truncated_before_seq = state.dropped_through_seq.and_then(|dropped| {
            (p.after_seq <= dropped).then(|| {
                state
                    .output
                    .front()
                    .map(|chunk| chunk.seq)
                    .unwrap_or(dropped.saturating_add(1))
            })
        });
        ProcessStatusResult {
            output,
            exit: state.exit.clone(),
            truncated_before_seq,
        }
    };
    let _ = tx.send(text_msg(&Response::ok(id, to_value(&result))));
}

pub(super) fn handle_input(
    id: u64,
    params: serde_json::Value,
    shared: &Arc<Shared>,
    tx: &Outbound,
) {
    let p: ProcessInputParams = match parse(params) {
        Ok(p) => p,
        Err((code, msg)) => {
            let _ = tx.send(text_msg(&Response::err(id, code, msg)));
            return;
        }
    };
    let proc = shared.procs.lock().unwrap().get(&p.process_id).cloned();
    let Some(proc) = proc else {
        let _ = tx.send(text_msg(&Response::err(
            id,
            error_code::UNKNOWN_PROCESS,
            "unknown or expired process",
        )));
        return;
    };
    let data = match b64_decode(&p.data) {
        Ok(data) => data,
        Err(error) => {
            let _ = tx.send(text_msg(&Response::err(
                id,
                error_code::INVALID_PARAMS,
                error,
            )));
            return;
        }
    };
    let response = match proc.input.send(ProcessInput {
        data,
        close: p.close,
    }) {
        Ok(()) => Response::ok(id, serde_json::json!({})),
        Err(_) => Response::err(id, error_code::EXEC_FAILED, "process stdin is closed"),
    };
    let _ = tx.send(text_msg(&response));
}

pub(super) fn handle_resume(
    id: u64,
    params: serde_json::Value,
    shared: &Arc<Shared>,
    tx: &Outbound,
    conn_token: &CancellationToken,
) {
    let p: ProcessResumeParams = match parse(params) {
        Ok(p) => p,
        Err((code, msg)) => {
            let _ = tx.send(text_msg(&Response::err(id, code, msg)));
            return;
        }
    };
    let proc = shared.procs.lock().unwrap().get(&p.process_id).cloned();
    match proc {
        Some(proc) => {
            let _ = tx.send(text_msg(&Response::ok(id, serde_json::json!({}))));
            spawn_streamer(proc, tx.clone(), p.after_seq, conn_token.clone());
        }
        None => {
            let _ = tx.send(text_msg(&Response::err(
                id,
                error_code::UNKNOWN_PROCESS,
                "unknown or expired process",
            )));
        }
    }
}

pub(super) fn handle_cancel(
    id: u64,
    params: serde_json::Value,
    shared: &Arc<Shared>,
    tx: &Outbound,
) {
    let p: ProcessIdParams = match parse(params) {
        Ok(p) => p,
        Err((code, msg)) => {
            let _ = tx.send(text_msg(&Response::err(id, code, msg)));
            return;
        }
    };
    if let Some(proc) = shared.procs.lock().unwrap().get(&p.process_id).cloned() {
        proc.cancel.cancel();
    }
    let _ = tx.send(text_msg(&Response::ok(id, serde_json::json!({}))));
}

fn spawn_streamer(
    proc: Arc<ProcShared>,
    tx: Outbound,
    after_seq: u64,
    conn_token: CancellationToken,
) {
    tokio::spawn(async move {
        let mut rx = proc.tick.subscribe();
        let mut cursor = after_seq;
        loop {
            let (chunks, exit) = {
                let st = proc.state.lock().unwrap();
                let chunks: Vec<ProcessOutputParams> = st
                    .output
                    .iter()
                    .filter(|c| c.seq > cursor)
                    .cloned()
                    .collect();
                (chunks, st.exit.clone())
            };
            for c in chunks {
                cursor = c.seq;
                if tx
                    .send(text_msg(&Notification::new(
                        method::PROCESS_OUTPUT,
                        to_value(&c),
                    )))
                    .is_err()
                {
                    return;
                }
            }
            if let Some(ex) = exit {
                if ex.seq > cursor {
                    let _ = tx.send(text_msg(&Notification::new(
                        method::PROCESS_EXIT,
                        to_value(&ex),
                    )));
                }
                return;
            }
            tokio::select! {
                _ = conn_token.cancelled() => return,
                _ = rx.recv() => {} // tick or lag: re-drain from cursor either way
            }
        }
    });
}

async fn run_process(
    proc: Arc<ProcShared>,
    shared: Arc<Shared>,
    command: String,
    cwd: PathBuf,
    timeout: Duration,
    pty: bool,
    input_rx: mpsc::UnboundedReceiver<ProcessInput>,
) {
    if pty {
        drop(input_rx);
        let fs = LocalExecutor;
        let result = fs
            .exec_streaming_pty(&command, &cwd, timeout, &proc.cancel, &|_, data| {
                append_output(&proc, Stream::Stdout, data.to_vec());
            })
            .await;
        match result {
            Ok(output) => append_exit(&proc, output.code, None),
            Err(error) => append_exit(&proc, None, Some(error)),
        }
        schedule_gc(shared, proc.process_id.clone());
        return;
    }

    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(&command)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    exec_core::configure_noninteractive(&mut cmd);
    isolate_process_group(&mut cmd);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            append_exit(&proc, None, Some(format!("failed to spawn shell: {e}")));
            schedule_gc(shared, proc.process_id.clone());
            return;
        }
    };

    let (otx, mut orx) = mpsc::channel::<(Stream, Vec<u8>)>(64);
    let outcome = tokio::spawn(pump(child, otx, proc.cancel.clone(), timeout, input_rx));

    while let Some((stream, data)) = orx.recv().await {
        append_output(&proc, stream, data);
    }
    match outcome.await {
        Ok(Outcome::Exited(code)) => append_exit(&proc, code, None),
        Ok(Outcome::Error(msg)) => append_exit(&proc, None, Some(msg)),
        Err(_) => append_exit(&proc, None, Some("process task panicked".into())),
    }
    schedule_gc(shared, proc.process_id.clone());
}

enum Outcome {
    Exited(Option<i32>),
    Error(String),
}

async fn pump(
    mut child: tokio::process::Child,
    otx: mpsc::Sender<(Stream, Vec<u8>)>,
    cancel: CancellationToken,
    timeout: Duration,
    mut input_rx: mpsc::UnboundedReceiver<ProcessInput>,
) -> Outcome {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let r1 = tokio::spawn(read_stream(stdout, Stream::Stdout, otx.clone()));
    let r2 = tokio::spawn(read_stream(stderr, Stream::Stderr, otx));
    let mut stdin = child.stdin.take();
    let input = tokio::spawn(async move {
        while let Some(message) = input_rx.recv().await {
            if !message.data.is_empty() {
                let Some(writer) = stdin.as_mut() else { return };
                if writer.write_all(&message.data).await.is_err() {
                    return;
                }
                let _ = writer.flush().await;
            }
            if message.close {
                return;
            }
        }
    });
    let root_pid = child.id();
    let mut r1 = r1;
    let mut r2 = r2;

    enum WaitResult {
        Exited(std::io::Result<std::process::ExitStatus>),
        Interrupted(String),
    }

    let result = {
        let wait_fut = std::pin::pin!(child.wait());
        tokio::select! {
            status = wait_fut => WaitResult::Exited(status),
            _ = cancel.cancelled() => WaitResult::Interrupted("command cancelled".to_string()),
            _ = tokio::time::sleep(timeout) =>
                WaitResult::Interrupted(format!("command timed out after {} ms", timeout.as_millis())),
        }
    };

    let outcome = match result {
        WaitResult::Exited(Ok(status)) => Outcome::Exited(status.code()),
        WaitResult::Exited(Err(e)) => Outcome::Error(format!("command failed: {e}")),
        WaitResult::Interrupted(message) => {
            terminate_process_tree(&mut child, root_pid).await;
            Outcome::Error(message)
        }
    };
    if !drain_readers(&mut r1, &mut r2).await {
        terminate_process_tree(&mut child, root_pid).await;
        r1.abort();
        r2.abort();
    }
    input.abort();
    outcome
}

async fn drain_readers(
    r1: &mut tokio::task::JoinHandle<()>,
    r2: &mut tokio::task::JoinHandle<()>,
) -> bool {
    tokio::time::timeout(Duration::from_millis(500), async {
        let _ = r1.await;
        let _ = r2.await;
    })
    .await
    .is_ok()
}

async fn read_stream<R>(reader: Option<R>, stream: Stream, otx: mpsc::Sender<(Stream, Vec<u8>)>)
where
    R: AsyncReadExt + Unpin,
{
    let Some(mut reader) = reader else { return };
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if otx.send((stream, buf[..n].to_vec())).await.is_err() {
                    return;
                }
            }
        }
    }
}

fn append_output(proc: &ProcShared, stream: Stream, data: Vec<u8>) {
    {
        let mut st = proc.state.lock().unwrap();
        let chunk_truncated = data.len() > MAX_PROCESS_OUTPUT_BYTES;
        let data = if chunk_truncated {
            data[data.len() - MAX_PROCESS_OUTPUT_BYTES..].to_vec()
        } else {
            data
        };
        st.next_seq = st.next_seq.saturating_add(1);
        let seq = st.next_seq;
        st.output_bytes = st.output_bytes.saturating_add(data.len());
        st.output.push_back(ProcessOutputParams {
            process_id: proc.process_id.clone(),
            seq,
            stream,
            data: b64_encode(&data),
        });
        if chunk_truncated {
            st.dropped_through_seq = Some(seq.saturating_sub(1));
        }
        while st.output_bytes > MAX_PROCESS_OUTPUT_BYTES {
            let Some(removed) = st.output.pop_front() else {
                break;
            };
            st.output_bytes = st.output_bytes.saturating_sub(
                b64_decode(&removed.data)
                    .map(|bytes| bytes.len())
                    .unwrap_or(0),
            );
            st.dropped_through_seq = Some(removed.seq);
        }
    }
    let _ = proc.tick.send(());
}

fn append_exit(proc: &ProcShared, code: Option<i32>, error: Option<String>) {
    {
        let mut st = proc.state.lock().unwrap();
        let seq = st.next_seq.saturating_add(1);
        st.exit = Some(ProcessExitParams {
            process_id: proc.process_id.clone(),
            seq,
            code,
            error,
        });
    }
    let _ = proc.tick.send(());
}

fn schedule_gc(shared: Arc<Shared>, process_id: String) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        shared.procs.lock().unwrap().remove(&process_id);
    });
}
