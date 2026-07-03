//! Backgrounded shell tasks: `bash(run_in_background: true)` starts a process
//! and returns immediately; `bash_output`/`bash_kill` poll/stop it later.
//!
//! Spawned directly via `tokio::process::Command` (not through the `Executor`
//! trait, which only has a blocking `exec()` — see `Executor::is_local`, which
//! gates this feature to local sessions only). Each task's `Child` is owned by
//! its own reader/waiter task; killing goes by PID rather than needing a
//! shared `&mut Child`, since the owning task is otherwise unreachable.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Cap on a single task's buffered combined stdout+stderr.
const MAX_OUTPUT_BYTES: usize = 512_000;

struct Entry {
    command: String,
    pid: Option<u32>,
    output: Arc<Mutex<Vec<u8>>>,
    /// `None` while running; `Some(exit_code)` once finished (`exit_code` is
    /// `None` if the process was killed by a signal rather than exiting).
    exit_code: Arc<Mutex<Option<Option<i32>>>>,
}

#[derive(Default)]
pub struct BackgroundTasks {
    next_id: AtomicU64,
    tasks: Mutex<HashMap<String, Entry>>,
}

pub struct TaskStatus {
    pub command: String,
    pub output: String,
    /// `None` while running.
    pub exit_code: Option<Option<i32>>,
}

impl BackgroundTasks {
    /// Start `command` in `cwd`, returning its task id immediately.
    pub fn spawn(&self, command: String, cwd: &Path) -> Result<String, String> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("failed to start background task: {e}"))?;

        let id = format!("bg-{}", self.next_id.fetch_add(1, Ordering::SeqCst) + 1);
        let pid = child.id();
        let output = Arc::new(Mutex::new(Vec::new()));
        let exit_code = Arc::new(Mutex::new(None));

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let out_for_readers = output.clone();
        let exit_for_waiter = exit_code.clone();
        tokio::spawn(async move {
            let mut readers = Vec::new();
            if let Some(s) = stdout {
                readers.push(tokio::spawn(read_into(s, out_for_readers.clone())));
            }
            if let Some(s) = stderr {
                readers.push(tokio::spawn(read_into(s, out_for_readers.clone())));
            }
            // Wait for the process itself first. The readers drain stdout/stderr
            // in their own tasks, so we don't need them to finish before we can
            // observe the exit — and we must not: a killed shell can orphan a
            // grandchild that keeps the output pipe open, so a readers-first wait
            // would wedge here forever and the task would never register as
            // finished (it'd show "running" in the UI after a kill). Give the
            // readers a brief grace to flush buffered output on a normal exit,
            // then abort any still blocked on an orphan-held pipe.
            let status = child.wait().await.ok();
            for mut r in readers {
                if tokio::time::timeout(Duration::from_millis(200), &mut r)
                    .await
                    .is_err()
                {
                    r.abort();
                }
            }
            *exit_for_waiter.lock().unwrap() = Some(status.and_then(|s| s.code()));
        });

        self.tasks.lock().unwrap().insert(
            id.clone(),
            Entry {
                command,
                pid,
                output,
                exit_code,
            },
        );
        Ok(id)
    }

    pub fn status(&self, id: &str) -> Option<TaskStatus> {
        let tasks = self.tasks.lock().unwrap();
        let entry = tasks.get(id)?;
        let output = entry.output.lock().unwrap();
        let output = String::from_utf8_lossy(&output).to_string();
        let exit_code = *entry.exit_code.lock().unwrap();
        Some(TaskStatus {
            command: entry.command.clone(),
            output,
            exit_code,
        })
    }

    /// Kill a running task by sending it a termination signal. Idempotent —
    /// killing an already-finished or unknown task is not an error.
    pub async fn kill(&self, id: &str) -> Result<(), String> {
        let pid = {
            let tasks = self.tasks.lock().unwrap();
            match tasks.get(id) {
                Some(entry) => entry.pid,
                None => return Err(format!("no background task `{id}`")),
            }
        };
        if let Some(pid) = pid {
            kill_pid(pid).await;
        }
        Ok(())
    }

    /// Kill every still-tracked task and clear the registry — called when a
    /// session resets (`new_session`), so tasks don't leak across "new
    /// chat"/project switches within one running app.
    pub async fn clear_all(&self) {
        let pids: Vec<u32> = {
            let tasks = self.tasks.lock().unwrap();
            tasks.values().filter_map(|e| e.pid).collect()
        };
        for pid in pids {
            kill_pid(pid).await;
        }
        self.tasks.lock().unwrap().clear();
    }
}

async fn read_into<R: tokio::io::AsyncRead + Unpin>(mut reader: R, buf: Arc<Mutex<Vec<u8>>>) {
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut b = buf.lock().unwrap();
                if b.len() < MAX_OUTPUT_BYTES {
                    let room = MAX_OUTPUT_BYTES - b.len();
                    b.extend_from_slice(&chunk[..n.min(room)]);
                }
            }
        }
    }
}

#[cfg(target_family = "unix")]
async fn kill_pid(pid: u32) {
    let _ = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .output()
        .await;
}

#[cfg(target_family = "windows")]
async fn kill_pid(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output()
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn wait_for_finish(tasks: &BackgroundTasks, id: &str) -> TaskStatus {
        for _ in 0..100 {
            let status = tasks.status(id).unwrap();
            if status.exit_code.is_some() {
                return status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("task {id} never finished");
    }

    #[tokio::test]
    async fn spawns_and_captures_output() {
        let tasks = BackgroundTasks::default();
        let dir = tempfile::tempdir().unwrap();
        let id = tasks
            .spawn("echo hello-bg".to_string(), dir.path())
            .unwrap();
        let status = wait_for_finish(&tasks, &id).await;
        assert_eq!(status.exit_code, Some(Some(0)));
        assert!(status.output.contains("hello-bg"), "{}", status.output);
    }

    #[tokio::test]
    async fn poll_before_finish_reports_no_exit_code() {
        let tasks = BackgroundTasks::default();
        let dir = tempfile::tempdir().unwrap();
        let id = tasks.spawn("sleep 5".to_string(), dir.path()).unwrap();
        let status = tasks.status(&id).unwrap();
        assert_eq!(status.exit_code, None);
        tasks.kill(&id).await.unwrap();
    }

    #[tokio::test]
    async fn kill_stops_a_running_task() {
        let tasks = BackgroundTasks::default();
        let dir = tempfile::tempdir().unwrap();
        let id = tasks.spawn("sleep 30".to_string(), dir.path()).unwrap();
        tasks.kill(&id).await.unwrap();
        // Killed processes exit (by signal, so `exit_code` may be `None`), the
        // waiter task should still observe and record completion.
        let status = wait_for_finish(&tasks, &id).await;
        assert!(status.exit_code.is_some());
    }

    #[tokio::test]
    async fn unknown_task_id_returns_none() {
        let tasks = BackgroundTasks::default();
        assert!(tasks.status("bg-999").is_none());
    }

    #[tokio::test]
    async fn clear_all_kills_running_tasks_and_empties_registry() {
        let tasks = BackgroundTasks::default();
        let dir = tempfile::tempdir().unwrap();
        let id = tasks.spawn("sleep 30".to_string(), dir.path()).unwrap();
        tasks.clear_all().await;
        assert!(tasks.status(&id).is_none());
    }
}
