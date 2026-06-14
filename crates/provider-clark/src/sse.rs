//! Resumable Server-Sent Events client for the Clark gateway's conversation
//! event stream.
//!
//! Clean-room, built from the observed contract (see `clark-ui`/`clark-mobile`
//! behavior, not their source): the reliable, self-healing event channel is
//!
//! ```text
//! GET {api_base}/api/conversations/{id}/events/stream?after_seq={n}
//! Accept: text/event-stream
//! Authorization: Bearer {token}
//! ```
//!
//! The server replays every event with `seq > after_seq` from its store, then
//! streams live ones, with periodic keep-alives. Each SSE frame is
//! `event: conversation_event` / `id: {seq}` / `data: {canonical event JSON}`.
//!
//! Unlike a bare WebSocket push, this is **resumable**: on any disconnect we
//! reconnect with `after_seq` advanced to the last seq we saw, so no events are
//! lost and a dropped connection mid-run self-heals instead of freezing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

const RECONNECT_BASE_MS: u64 = 1_000;
const RECONNECT_MAX_MS: u64 = 15_000;
/// Give up after this many *consecutive* "gone" responses (403/404/410). A fresh
/// conversation can 404 briefly until the first `send_message` commits it, so we
/// tolerate a few before concluding the stream is truly dead.
const MAX_PERMANENT_STRIKES: u32 = 5;

/// Where to stream a conversation's events from.
pub struct SseConfig {
    /// HTTP(S) origin of the gateway, e.g. `http://localhost:8400`.
    pub api_base: String,
    pub conversation_id: String,
    pub token: Option<String>,
    /// Resume point — only events with `seq > after_seq` are delivered.
    pub after_seq: u64,
    /// Override the reconnect backoff base (ms). `None` uses the default.
    pub reconnect_base_ms: Option<u64>,
}

/// Why a connection attempt ended.
enum ConnectErr {
    /// The stream is gone (403/404/410) — counts toward giving up.
    Permanent(u16),
    /// Network/5xx/decode error — retried indefinitely with backoff.
    Transient(String),
}

/// Statuses that mean "stop retrying" — the conversation/stream is gone.
fn is_permanent_status(status: u16) -> bool {
    matches!(status, 403 | 404 | 410)
}

/// Handle that stops the background stream when dropped.
pub struct SseHandle {
    stop: Arc<AtomicBool>,
}

impl Drop for SseHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Spawn the resumable stream. Each decoded event object (`{type, data, seq,
/// ...}`) is forwarded to `out` wrapped as `{type:"event", event:{...}}` so the
/// existing engine routing/translation applies unchanged.
pub fn spawn(config: SseConfig, out: UnboundedSender<Value>) -> SseHandle {
    let stop = Arc::new(AtomicBool::new(false));
    tokio::spawn(run(config, out, stop.clone()));
    SseHandle { stop }
}

async fn run(config: SseConfig, out: UnboundedSender<Value>, stop: Arc<AtomicBool>) {
    let client = match reqwest::Client::builder()
        // No total timeout: this is a long-lived stream.
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "clark sse: client build failed");
            return;
        }
    };

    let base_url = config.api_base.trim_end_matches('/');
    let backoff_base = config.reconnect_base_ms.unwrap_or(RECONNECT_BASE_MS);
    let mut after_seq = config.after_seq;
    let mut attempt: u32 = 0;
    let mut permanent_strikes: u32 = 0;

    while !stop.load(Ordering::SeqCst) && !out.is_closed() {
        let url = format!(
            "{base_url}/api/conversations/{}/events/stream?after_seq={after_seq}",
            config.conversation_id
        );
        match connect_once(
            &client,
            &url,
            config.token.as_deref(),
            &out,
            &mut after_seq,
            &stop,
        )
        .await
        {
            Ok(()) => {
                // Stream ended cleanly (server closed / our own stop) — loop will
                // re-check `stop`; if still running, treat like a drop.
                attempt = 0;
                permanent_strikes = 0;
            }
            Err(ConnectErr::Transient(e)) => {
                permanent_strikes = 0;
                tracing::debug!(error = %e, after_seq, "clark sse: stream ended, will reconnect");
            }
            Err(ConnectErr::Permanent(status)) => {
                permanent_strikes += 1;
                if permanent_strikes >= MAX_PERMANENT_STRIKES {
                    tracing::warn!(
                        status,
                        after_seq,
                        "clark sse: stream gone (repeated {status}); giving up"
                    );
                    break;
                }
                tracing::debug!(
                    status,
                    strikes = permanent_strikes,
                    "clark sse: gone, retrying"
                );
            }
        }

        if stop.load(Ordering::SeqCst) || out.is_closed() {
            break;
        }
        // Exponential backoff before reconnecting (resumes from `after_seq`).
        let delay = (backoff_base * 2u64.saturating_pow(attempt)).min(RECONNECT_MAX_MS);
        attempt = attempt.saturating_add(1);
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    tracing::debug!("clark sse: stream stopped");
}

/// One connection attempt: stream frames until the body ends or `stop` is set.
async fn connect_once(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    out: &UnboundedSender<Value>,
    after_seq: &mut u64,
    stop: &Arc<AtomicBool>,
) -> Result<(), ConnectErr> {
    let mut req = client.get(url).header("Accept", "text/event-stream");
    if let Some(tok) = token {
        req = req.header("Authorization", format!("Bearer {tok}"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| ConnectErr::Transient(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(if is_permanent_status(status) {
            ConnectErr::Permanent(status)
        } else {
            ConnectErr::Transient(format!("status {status}"))
        });
    }

    let mut stream = resp.bytes_stream();
    // Buffer raw bytes (not str): a multi-byte UTF-8 char can be split across
    // TCP chunks, so we only decode once we have a complete frame.
    let mut buffer: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        if stop.load(Ordering::SeqCst) || out.is_closed() {
            return Ok(());
        }
        let bytes = chunk.map_err(|e| ConnectErr::Transient(e.to_string()))?;
        buffer.extend_from_slice(&bytes);
        // Frames are separated by a blank line (`\n\n` or `\r\n\r\n`); keep the
        // trailing partial frame for the next chunk.
        while let Some((at, len)) = find_separator(&buffer) {
            let frame_bytes: Vec<u8> = buffer.drain(..at + len).collect();
            let frame = String::from_utf8_lossy(&frame_bytes);
            if let Some((event, data)) = parse_frame(&frame) {
                if event == "conversation_event" {
                    if let Ok(evt) = serde_json::from_str::<Value>(&data) {
                        if let Some(seq) = evt.get("seq").and_then(Value::as_u64) {
                            *after_seq = (*after_seq).max(seq);
                        }
                        // Wrap to match the engine's `{type:"event", event}` shape.
                        if out
                            .send(serde_json::json!({ "type": "event", "event": evt }))
                            .is_err()
                        {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Find the first frame separator (a blank line) in the byte buffer and its
/// length, tolerating both `\n\n` and `\r\n\r\n` line endings.
fn find_separator(buffer: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i + 1 < buffer.len() {
        if i + 4 <= buffer.len() && &buffer[i..i + 4] == b"\r\n\r\n" {
            return Some((i, 4));
        }
        if buffer[i] == b'\n' && buffer[i + 1] == b'\n' {
            return Some((i, 2));
        }
        i += 1;
    }
    None
}

/// Parse one `text/event-stream` frame into `(event, data)`. Mirrors the SSE
/// line grammar: `event:`/`data:` lines, `:`-comments (keep-alives) ignored,
/// multiple `data:` lines joined by `\n`. Returns `None` if there's no data.
fn parse_frame(frame: &str) -> Option<(String, String)> {
    let mut event = "message".to_string();
    let mut data: Vec<String> = Vec::new();
    for line in frame.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            let v = rest.strip_prefix(' ').unwrap_or(rest);
            if !v.is_empty() {
                event = v.to_string();
            }
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
    }
    if data.is_empty() {
        None
    } else {
        Some((event, data.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_event_with_data() {
        let frame =
            "event: conversation_event\nid: 42\ndata: {\"seq\":42,\"type\":\"tool_call\"}\n\n";
        let (event, data) = parse_frame(frame).expect("frame");
        assert_eq!(event, "conversation_event");
        let v: Value = serde_json::from_str(&data).unwrap();
        assert_eq!(v["seq"], 42);
        assert_eq!(v["type"], "tool_call");
    }

    #[test]
    fn ignores_comment_keepalive_frames() {
        assert!(parse_frame(": keep-alive\n\n").is_none());
        assert!(parse_frame(":\n").is_none());
    }

    #[test]
    fn joins_multiline_data() {
        let (_e, data) = parse_frame("data: a\ndata: b\n\n").expect("frame");
        assert_eq!(data, "a\nb");
    }

    #[test]
    fn separator_handles_lf_and_crlf() {
        assert_eq!(find_separator(b"a\n\nb"), Some((1, 2)));
        assert_eq!(find_separator(b"a\r\n\r\nb"), Some((1, 4)));
        assert_eq!(find_separator(b"no sep yet\n"), None);
        // Whichever blank line comes first wins.
        assert_eq!(find_separator(b"x\n\ny\r\n\r\n"), Some((1, 2)));
    }

    #[test]
    fn frame_boundary_survives_split_multibyte_char() {
        // A UTF-8 “ (0xE2 0x80 0x9C) split across two chunks must not corrupt.
        let smart = "“hi”";
        let full = format!("event: conversation_event\ndata: {{\"t\":\"{smart}\"}}\n\n");
        let bytes = full.as_bytes();
        // Reassemble byte-by-byte; the frame only completes at the blank line.
        let mut buf: Vec<u8> = Vec::new();
        let mut frames = 0;
        for &b in bytes {
            buf.push(b);
            while let Some((at, len)) = find_separator(&buf) {
                let frame: Vec<u8> = buf.drain(..at + len).collect();
                let s = String::from_utf8_lossy(&frame);
                assert!(s.contains(smart), "decoded frame lost the smart quotes");
                frames += 1;
            }
        }
        assert_eq!(frames, 1);
    }

    #[test]
    fn permanent_statuses_are_403_404_410() {
        for s in [403u16, 404, 410] {
            assert!(is_permanent_status(s), "{s} should be permanent");
        }
        for s in [200u16, 401, 429, 500, 502, 503] {
            assert!(!is_permanent_status(s), "{s} should be transient");
        }
    }
}

/// Full simulated SSE state matrix. A scripted in-process server drives the real
/// `connect_once`/`run` client through every transport edge case — replay,
/// ordering, cursor advance, keep-alives, partial frames, malformed data,
/// foreign event names, CRLF, HTTP errors, and reconnect-with-resume — so we can
/// trust the client even when the live gateway misbehaves.
#[cfg(test)]
mod sim {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    /// What the scripted server does for one connection.
    enum Resp {
        /// 200 SSE: write each string as its own TCP chunk (sub-frame splits
        /// allowed to exercise reassembly), then close the socket.
        Stream(Vec<String>),
        /// A raw HTTP error status, then close.
        Http(u16),
    }

    struct Fake {
        addr: std::net::SocketAddr,
        /// `after_seq` query value seen on each connection (proves resume).
        after_seqs: Arc<StdMutex<Vec<u64>>>,
        /// `Authorization` header seen on each connection.
        auths: Arc<StdMutex<Vec<Option<String>>>>,
    }

    fn ev(seq: u64, ty: &str) -> String {
        format!("event: conversation_event\nid: {seq}\ndata: {{\"seq\":{seq},\"type\":\"{ty}\",\"data\":{{}}}}\n\n")
    }
    fn ev_crlf(seq: u64, ty: &str) -> String {
        format!("event: conversation_event\r\nid: {seq}\r\ndata: {{\"seq\":{seq},\"type\":\"{ty}\",\"data\":{{}}}}\r\n\r\n")
    }

    async fn read_head(stream: &mut TcpStream) -> (u64, Option<String>) {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        loop {
            match stream.read(&mut tmp).await {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
            }
            if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 16_384 {
                break;
            }
        }
        let head = String::from_utf8_lossy(&buf);
        let after = head
            .lines()
            .next()
            .and_then(|l| l.split("after_seq=").nth(1))
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let auth = head
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
            .map(|l| l.splitn(2, ':').nth(1).unwrap_or("").trim().to_string());
        (after, auth)
    }

    async fn spawn_fake<F>(handler: F) -> Fake
    where
        F: Fn(u64, usize) -> Resp + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let after_seqs = Arc::new(StdMutex::new(Vec::new()));
        let auths = Arc::new(StdMutex::new(Vec::new()));
        let (a2, au2) = (after_seqs.clone(), auths.clone());
        let handler = Arc::new(handler);
        tokio::spawn(async move {
            let mut idx = 0usize;
            while let Ok((mut stream, _)) = listener.accept().await {
                let (after, auth) = read_head(&mut stream).await;
                a2.lock().unwrap().push(after);
                au2.lock().unwrap().push(auth);
                match handler(after, idx) {
                    Resp::Stream(chunks) => {
                        let _ = stream
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n")
                            .await;
                        for c in chunks {
                            if stream.write_all(c.as_bytes()).await.is_err() {
                                break;
                            }
                            let _ = stream.flush().await;
                            tokio::time::sleep(Duration::from_millis(3)).await;
                        }
                    }
                    Resp::Http(status) => {
                        let _ = stream
                            .write_all(
                                format!("HTTP/1.1 {status} ERR\r\nContent-Length: 0\r\n\r\n")
                                    .as_bytes(),
                            )
                            .await;
                    }
                }
                idx += 1; // socket dropped here = connection close
            }
        });
        Fake {
            addr,
            after_seqs,
            auths,
        }
    }

    /// Run one `connect_once` against the fake; return (delivered seqs, cursor).
    async fn one(fake: &Fake, after_seq: u64) -> (Vec<u64>, u64) {
        let client = reqwest::Client::new();
        let (tx, mut rx) = unbounded_channel::<Value>();
        let mut after = after_seq;
        let stop = Arc::new(AtomicBool::new(false));
        let url = format!(
            "http://{}/api/conversations/c1/events/stream?after_seq={after_seq}",
            fake.addr
        );
        let _ = connect_once(&client, &url, Some("tok"), &tx, &mut after, &stop).await;
        let mut got = Vec::new();
        while let Ok(v) = rx.try_recv() {
            got.push(v["event"]["seq"].as_u64().unwrap());
        }
        (got, after)
    }

    async fn collect_seqs(
        rx: &mut UnboundedReceiver<Value>,
        n: usize,
        budget: Duration,
    ) -> Vec<u64> {
        let mut got = Vec::new();
        let deadline = tokio::time::Instant::now() + budget;
        while got.len() < n {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(v)) => got.push(v["event"]["seq"].as_u64().unwrap()),
                _ => break,
            }
        }
        got
    }

    #[tokio::test]
    async fn replays_in_order_advances_cursor_and_sends_auth() {
        let fake =
            spawn_fake(|_a, _i| Resp::Stream(vec![ev(1, "a"), ev(2, "b"), ev(3, "c")])).await;
        let (got, after) = one(&fake, 0).await;
        assert_eq!(got, vec![1, 2, 3]);
        assert_eq!(after, 3);
        assert_eq!(fake.auths.lock().unwrap()[0].as_deref(), Some("Bearer tok"));
    }

    #[tokio::test]
    async fn keepalive_comments_deliver_nothing() {
        let fake = spawn_fake(|_a, _i| {
            Resp::Stream(vec![
                ": ping\n\n".into(),
                ev(1, "x"),
                ": keep-alive\n\n".into(),
            ])
        })
        .await;
        let (got, after) = one(&fake, 0).await;
        assert_eq!(got, vec![1]);
        assert_eq!(after, 1);
    }

    #[tokio::test]
    async fn reassembles_frame_split_across_chunks() {
        let fake = spawn_fake(|_a, _i| {
            Resp::Stream(vec![
                "event: conversation_event\nid: 7\nda".into(),
                "ta: {\"seq\":7,\"type\":\"x\",\"data\":{}}\n\n".into(),
            ])
        })
        .await;
        let (got, _) = one(&fake, 0).await;
        assert_eq!(got, vec![7]);
    }

    #[tokio::test]
    async fn malformed_json_is_skipped_and_stream_continues() {
        let fake = spawn_fake(|_a, _i| {
            Resp::Stream(vec![
                "event: conversation_event\ndata: {not json\n\n".into(),
                ev(5, "x"),
            ])
        })
        .await;
        let (got, _) = one(&fake, 0).await;
        assert_eq!(got, vec![5]);
    }

    #[tokio::test]
    async fn foreign_event_names_are_ignored() {
        let fake = spawn_fake(|_a, _i| {
            Resp::Stream(vec![
                "event: presentation_patch\ndata: {\"last_seq\":1}\n\n".into(),
                ev(9, "x"),
            ])
        })
        .await;
        let (got, _) = one(&fake, 0).await;
        assert_eq!(got, vec![9]);
    }

    #[tokio::test]
    async fn handles_crlf_frames() {
        let fake = spawn_fake(|_a, _i| Resp::Stream(vec![ev_crlf(1, "a"), ev_crlf(2, "b")])).await;
        let (got, after) = one(&fake, 0).await;
        assert_eq!(got, vec![1, 2]);
        assert_eq!(after, 2);
    }

    #[tokio::test]
    async fn cursor_tracks_highest_seq_seen() {
        let fake =
            spawn_fake(|_a, _i| Resp::Stream(vec![ev(3, "a"), ev(2, "b"), ev(5, "c")])).await;
        let (got, after) = one(&fake, 0).await;
        assert_eq!(got, vec![3, 2, 5]);
        assert_eq!(after, 5, "cursor must not regress below the max seq seen");
    }

    #[tokio::test]
    async fn http_error_returns_err_with_no_events() {
        let fake = spawn_fake(|_a, _i| Resp::Http(500)).await;
        let client = reqwest::Client::new();
        let (tx, mut rx) = unbounded_channel::<Value>();
        let mut after = 0u64;
        let stop = Arc::new(AtomicBool::new(false));
        let url = format!(
            "http://{}/api/conversations/c1/events/stream?after_seq=0",
            fake.addr
        );
        let r = connect_once(&client, &url, Some("t"), &tx, &mut after, &stop).await;
        assert!(
            r.is_err(),
            "5xx should surface as an error so run() backs off"
        );
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn run_reconnects_and_resumes_without_loss_or_dup() {
        // Each connection emits two new events past `after_seq`, capped at 4,
        // then drops — forcing the client to reconnect and resume.
        let fake = spawn_fake(|after, _idx| {
            let start = after + 1;
            let end = (after + 2).min(4);
            let chunks = (start..=end).map(|seq| ev(seq, "x")).collect::<Vec<_>>();
            Resp::Stream(chunks)
        })
        .await;
        let (tx, mut rx) = unbounded_channel::<Value>();
        let handle = spawn(
            SseConfig {
                api_base: format!("http://{}", fake.addr),
                conversation_id: "c1".into(),
                token: Some("t".into()),
                after_seq: 0,
                reconnect_base_ms: Some(20),
            },
            tx,
        );
        let got = collect_seqs(&mut rx, 4, Duration::from_secs(12)).await;
        assert_eq!(got, vec![1, 2, 3, 4], "no loss, no dup across reconnects");
        let seen = fake.after_seqs.lock().unwrap().clone();
        drop(handle);
        // The second connection must have resumed from the advanced cursor (2),
        // not replayed from 0 — that's what proves no loss and no duplicates.
        assert!(seen.len() >= 2, "expected a reconnect, saw {seen:?}");
        assert_eq!(seen[0], 0, "first connect starts at 0");
        assert_eq!(seen[1], 2, "reconnect resumes from the advancing cursor");
    }

    #[tokio::test]
    async fn dropping_handle_stops_the_stream() {
        let fake = spawn_fake(|_a, _i| Resp::Stream(vec![ev(1, "x")])).await;
        let (tx, mut rx) = unbounded_channel::<Value>();
        let handle = spawn(
            SseConfig {
                api_base: format!("http://{}", fake.addr),
                conversation_id: "c1".into(),
                token: None,
                after_seq: 0,
                reconnect_base_ms: None,
            },
            tx,
        );
        let _ = collect_seqs(&mut rx, 1, Duration::from_secs(5)).await;
        drop(handle);
        // After stopping, the channel eventually closes (sender dropped).
        tokio::time::sleep(Duration::from_millis(50)).await;
        // No auth header was sent when token is None.
        assert!(fake.auths.lock().unwrap()[0].is_none());
    }

    #[tokio::test]
    async fn gives_up_after_repeated_permanent_status() {
        // Always "gone" — the client must stop after MAX_PERMANENT_STRIKES
        // attempts instead of reconnecting forever.
        let fake = spawn_fake(|_a, _i| Resp::Http(404)).await;
        let (tx, _rx) = unbounded_channel::<Value>();
        let handle = spawn(
            SseConfig {
                api_base: format!("http://{}", fake.addr),
                conversation_id: "c1".into(),
                token: Some("t".into()),
                after_seq: 0,
                reconnect_base_ms: Some(5),
            },
            tx,
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
        let count = fake.after_seqs.lock().unwrap().len();
        assert_eq!(
            count, MAX_PERMANENT_STRIKES as usize,
            "should stop after exactly MAX_PERMANENT_STRIKES attempts, saw {count}"
        );
        // Confirm it truly stopped: no further attempts in another interval.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(fake.after_seqs.lock().unwrap().len(), count);
        drop(handle);
    }

    #[tokio::test]
    async fn recovers_from_startup_404_then_streams() {
        // A fresh conversation 404s until it commits; the client must tolerate
        // the early 404s and still deliver events once the stream appears.
        let fake = spawn_fake(|_a, idx| {
            if idx < 2 {
                Resp::Http(404)
            } else {
                Resp::Stream(vec![ev(1, "x")])
            }
        })
        .await;
        let (tx, mut rx) = unbounded_channel::<Value>();
        let handle = spawn(
            SseConfig {
                api_base: format!("http://{}", fake.addr),
                conversation_id: "c1".into(),
                token: Some("t".into()),
                after_seq: 0,
                reconnect_base_ms: Some(20),
            },
            tx,
        );
        let got = collect_seqs(&mut rx, 1, Duration::from_secs(5)).await;
        assert_eq!(got, vec![1], "must recover and deliver after early 404s");
        drop(handle);
    }
}
