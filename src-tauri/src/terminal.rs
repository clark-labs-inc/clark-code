//! Fast in-app terminal backed by a real PTY (`portable-pty`).
//!
//! `terminal_open` spawns the user's login shell in a pseudo-terminal rooted at
//! the project folder. A dedicated reader thread streams output to the UI as
//! base64 chunks over the `terminal://data` event (base64 keeps UTF-8 and
//! escape sequences intact across read boundaries). `terminal_write`,
//! `terminal_resize`, and `terminal_close` drive the other direction. The web UI
//! renders it with xterm.js.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;

use base64::Engine as _;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

/// One live PTY: the master end (for resize), a writer (for keystrokes), and the
/// child shell (so we can kill it on close).
struct Session {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    _process_fence: exec_core::ProcessFence,
}

/// Managed map of open terminals, keyed by a UI-generated id.
#[derive(Default)]
pub struct Terminals(Mutex<HashMap<String, Session>>);

impl Terminals {
    pub(crate) fn shutdown_all(&self) -> usize {
        let sessions = std::mem::take(&mut *self.0.lock().unwrap());
        let count = sessions.len();
        for (_, mut session) in sessions {
            let _ = session.child.kill();
        }
        count
    }
}

#[derive(Clone, Serialize)]
struct TermData {
    id: String,
    /// base64 of the raw PTY bytes.
    chunk: String,
}

fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[tauri::command]
pub fn terminal_open(
    app: AppHandle,
    terminals: State<'_, Terminals>,
    id: String,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let pair = native_pty_system()
        .openpty(pty_size(cols, rows))
        .map_err(|e| e.to_string())?;

    let shell = exec_core::interactive_shell();
    let mut cmd = CommandBuilder::new(shell.program);
    cmd.args(shell.args);
    cmd.env("TERM", "xterm-256color");
    if let Some(dir) = cwd.filter(|d| !d.is_empty()) {
        cmd.cwd(dir);
    }

    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    let process_fence = exec_core::ProcessFence::attach(child.process_id());
    // Drop the slave so the PTY reports EOF once the shell exits.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    // Reader thread: stream output to the UI until the shell exits.
    let app = app.clone();
    let stream_id = id.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let chunk = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    let payload = TermData {
                        id: stream_id.clone(),
                        chunk,
                    };
                    if app.emit("terminal://data", payload).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = app.emit("terminal://exit", stream_id);
    });

    let mut map = terminals.0.lock().unwrap();
    // An id collision (double-open, or a reused id) would otherwise drop the old
    // Session silently — and portable_pty's Child isn't killed on drop — orphaning
    // the previous shell whose reader thread keeps emitting for this same id.
    if let Some(mut old) = map.remove(&id) {
        let _ = old.child.kill();
    }
    map.insert(
        id,
        Session {
            writer,
            master: pair.master,
            child,
            _process_fence: process_fence,
        },
    );
    Ok(())
}

#[tauri::command]
pub fn terminal_write(
    terminals: State<'_, Terminals>,
    id: String,
    data: String,
) -> Result<(), String> {
    let mut map = terminals.0.lock().unwrap();
    let session = map.get_mut(&id).ok_or("no such terminal")?;
    session
        .writer
        .write_all(data.as_bytes())
        .map_err(|e| e.to_string())?;
    session.writer.flush().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn terminal_resize(
    terminals: State<'_, Terminals>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let map = terminals.0.lock().unwrap();
    let session = map.get(&id).ok_or("no such terminal")?;
    session
        .master
        .resize(pty_size(cols, rows))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn terminal_close(terminals: State<'_, Terminals>, id: String) -> Result<(), String> {
    if let Some(mut session) = terminals.0.lock().unwrap().remove(&id) {
        let _ = session.child.kill();
    }
    Ok(())
}
