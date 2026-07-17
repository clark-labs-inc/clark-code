//! Session-owned long-lived shell processes.
//!
//! Local tasks own an isolated process group on this machine. Remote tasks are
//! owned by the exec server and polled through the same executor abstraction.
//! Both paths share bounded head/tail output, stdin, kill, and registry limits.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, Command};

use crate::exec::Executor;

const MAX_TASKS: usize = 32;
const MAX_OUTPUT_BYTES: usize = 512_000;
const HEAD_BYTES: usize = 64_000;
const TAIL_BYTES: usize = MAX_OUTPUT_BYTES - HEAD_BYTES;

#[derive(Default)]
struct OutputBuffer {
    head: Vec<u8>,
    tail: Vec<u8>,
    total_bytes: usize,
    upstream_truncated: bool,
}

impl OutputBuffer {
    fn append(&mut self, bytes: &[u8]) {
        self.total_bytes = self.total_bytes.saturating_add(bytes.len());
        let head_room = HEAD_BYTES.saturating_sub(self.head.len());
        let head_take = head_room.min(bytes.len());
        self.head.extend_from_slice(&bytes[..head_take]);
        self.tail.extend_from_slice(&bytes[head_take..]);
        if self.tail.len() > TAIL_BYTES {
            self.tail.drain(..self.tail.len() - TAIL_BYTES);
        }
    }

    fn render(&self) -> String {
        let omitted = self
            .total_bytes
            .saturating_sub(self.head.len().saturating_add(self.tail.len()));
        let mut bytes = self.head.clone();
        if omitted > 0 || self.upstream_truncated {
            let detail = if omitted > 0 {
                format!("\n… [{omitted} middle bytes truncated] …\n")
            } else {
                "\n… [earlier remote output truncated] …\n".to_string()
            };
            bytes.extend_from_slice(detail.as_bytes());
        }
        bytes.extend_from_slice(&self.tail);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

enum Backend {
    Local {
        pid: Option<u32>,
        stdin: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    },
    Remote {
        executor: Arc<dyn Executor>,
        process_id: String,
        cursor: Arc<AtomicU64>,
    },
}

struct Entry {
    ordinal: u64,
    command: String,
    backend: Backend,
    output: Arc<Mutex<OutputBuffer>>,
    /// `None` while running; `Some(exit_code)` after completion.
    exit_code: Arc<Mutex<Option<Option<i32>>>>,
    error: Arc<Mutex<Option<String>>>,
}

#[derive(Default)]
pub struct BackgroundTasks {
    next_id: AtomicU64,
    tasks: Mutex<HashMap<String, Entry>>,
}

pub struct TaskStatus {
    pub command: String,
    pub output: String,
    pub exit_code: Option<Option<i32>>,
    pub error: Option<String>,
}

impl BackgroundTasks {
    pub async fn spawn(
        &self,
        executor: Arc<dyn Executor>,
        command: String,
        cwd: &Path,
    ) -> Result<String, String> {
        self.make_room()?;
        let ordinal = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let id = format!("bg-{ordinal}");
        let output = Arc::new(Mutex::new(OutputBuffer::default()));
        let exit_code = Arc::new(Mutex::new(None));
        let error = Arc::new(Mutex::new(None));

        let backend = if executor.is_local() {
            let mut process = Command::new("sh");
            process
                .arg("-c")
                .arg(&command)
                .current_dir(cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            exec_core::configure_noninteractive(&mut process);
            exec_core::isolate_process_group(&mut process);
            let mut child = process
                .spawn()
                .map_err(|e| format!("failed to start background task: {e}"))?;
            let pid = child.id();
            let stdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take()));
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let reader_output = output.clone();
            let waiter_exit = exit_code.clone();
            let waiter_error = error.clone();
            tokio::spawn(async move {
                let mut readers = Vec::new();
                if let Some(stream) = stdout {
                    readers.push(tokio::spawn(read_into(stream, reader_output.clone())));
                }
                if let Some(stream) = stderr {
                    readers.push(tokio::spawn(read_into(stream, reader_output.clone())));
                }
                let outcome = child.wait().await;
                for mut reader in readers {
                    if tokio::time::timeout(Duration::from_millis(200), &mut reader)
                        .await
                        .is_err()
                    {
                        reader.abort();
                    }
                }
                match outcome {
                    Ok(status) => *waiter_exit.lock().unwrap() = Some(status.code()),
                    Err(failure) => {
                        *waiter_error.lock().unwrap() = Some(failure.to_string());
                        *waiter_exit.lock().unwrap() = Some(None);
                    }
                }
            });
            Backend::Local { pid, stdin }
        } else {
            let process_id = executor.background_start(&command, cwd).await?;
            Backend::Remote {
                executor,
                process_id,
                cursor: Arc::new(AtomicU64::new(0)),
            }
        };

        self.tasks.lock().unwrap().insert(
            id.clone(),
            Entry {
                ordinal,
                command,
                backend,
                output,
                exit_code,
                error,
            },
        );
        Ok(id)
    }

    fn make_room(&self) -> Result<(), String> {
        let mut tasks = self.tasks.lock().unwrap();
        if tasks.len() < MAX_TASKS {
            return Ok(());
        }
        let oldest_finished = tasks
            .iter()
            .filter(|(_, entry)| entry.exit_code.lock().unwrap().is_some())
            .min_by_key(|(_, entry)| entry.ordinal)
            .map(|(id, _)| id.clone());
        if let Some(id) = oldest_finished {
            tasks.remove(&id);
            Ok(())
        } else {
            Err(format!(
                "too many running background tasks (maximum {MAX_TASKS})"
            ))
        }
    }

    pub async fn status(&self, id: &str) -> Option<TaskStatus> {
        let remote = {
            let tasks = self.tasks.lock().unwrap();
            let entry = tasks.get(id)?;
            match &entry.backend {
                Backend::Remote {
                    executor,
                    process_id,
                    cursor,
                } if entry.exit_code.lock().unwrap().is_none() => Some((
                    executor.clone(),
                    process_id.clone(),
                    cursor.clone(),
                    entry.output.clone(),
                    entry.exit_code.clone(),
                    entry.error.clone(),
                )),
                _ => None,
            }
        };
        if let Some((executor, process_id, cursor, output, exit_code, error)) = remote {
            let after = cursor.load(Ordering::SeqCst);
            match executor.background_status(&process_id, after).await {
                Ok(status) => {
                    {
                        let mut buffer = output.lock().unwrap();
                        for chunk in status.output {
                            buffer.append(&chunk.data);
                        }
                        buffer.upstream_truncated |= status.truncated;
                    }
                    cursor.store(status.cursor, Ordering::SeqCst);
                    if let Some(code) = status.exit_code {
                        *exit_code.lock().unwrap() = Some(code);
                    }
                    if status.error.is_some() {
                        *error.lock().unwrap() = status.error;
                    }
                }
                Err(failure) => {
                    *error.lock().unwrap() = Some(failure);
                    *exit_code.lock().unwrap() = Some(None);
                }
            }
        }

        let tasks = self.tasks.lock().unwrap();
        let entry = tasks.get(id)?;
        let command = entry.command.clone();
        let output = entry.output.lock().unwrap().render();
        let exit_code = *entry.exit_code.lock().unwrap();
        let error = entry.error.lock().unwrap().clone();
        Some(TaskStatus {
            command,
            output,
            exit_code,
            error,
        })
    }

    pub async fn write(&self, id: &str, data: &[u8], close: bool) -> Result<(), String> {
        enum Target {
            Local(Arc<tokio::sync::Mutex<Option<ChildStdin>>>),
            Remote(Arc<dyn Executor>, String),
        }
        let target = {
            let tasks = self.tasks.lock().unwrap();
            let entry = tasks
                .get(id)
                .ok_or_else(|| format!("no background task `{id}`"))?;
            if entry.exit_code.lock().unwrap().is_some() {
                return Err(format!("background task `{id}` has already finished"));
            }
            match &entry.backend {
                Backend::Local { stdin, .. } => Target::Local(stdin.clone()),
                Backend::Remote {
                    executor,
                    process_id,
                    ..
                } => Target::Remote(executor.clone(), process_id.clone()),
            }
        };
        match target {
            Target::Local(stdin) => {
                let mut stdin = stdin.lock().await;
                if !data.is_empty() {
                    stdin
                        .as_mut()
                        .ok_or("background task stdin is closed")?
                        .write_all(data)
                        .await
                        .map_err(|error| error.to_string())?;
                }
                if close {
                    stdin.take();
                }
                Ok(())
            }
            Target::Remote(executor, process_id) => {
                executor.background_write(&process_id, data, close).await
            }
        }
    }

    pub async fn kill(&self, id: &str) -> Result<(), String> {
        enum Target {
            Local(Option<u32>),
            Remote(Arc<dyn Executor>, String),
            Finished,
        }
        let target = {
            let tasks = self.tasks.lock().unwrap();
            let entry = tasks
                .get(id)
                .ok_or_else(|| format!("no background task `{id}`"))?;
            if entry.exit_code.lock().unwrap().is_some() {
                Target::Finished
            } else {
                match &entry.backend {
                    Backend::Local { pid, .. } => Target::Local(*pid),
                    Backend::Remote {
                        executor,
                        process_id,
                        ..
                    } => Target::Remote(executor.clone(), process_id.clone()),
                }
            }
        };
        match target {
            Target::Local(pid) => exec_core::terminate_pid_tree(pid).await,
            Target::Remote(executor, process_id) => executor.background_kill(&process_id).await?,
            Target::Finished => {}
        }
        Ok(())
    }

    pub async fn clear_all(&self) {
        let ids = self
            .tasks
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in ids {
            let _ = self.kill(&id).await;
        }
        self.tasks.lock().unwrap().clear();
    }
}

async fn read_into<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    output: Arc<Mutex<OutputBuffer>>,
) {
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(count) => output.lock().unwrap().append(&chunk[..count]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;

    async fn wait_for_finish(tasks: &BackgroundTasks, id: &str) -> TaskStatus {
        for _ in 0..100 {
            let status = tasks.status(id).await.unwrap();
            if status.exit_code.is_some() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("task {id} never finished");
    }

    #[tokio::test]
    async fn spawns_accepts_input_and_captures_output() {
        let tasks = BackgroundTasks::default();
        let dir = tempfile::tempdir().unwrap();
        let id = tasks
            .spawn(
                Arc::new(LocalExecutor),
                "read value; echo got:$value".to_string(),
                dir.path(),
            )
            .await
            .unwrap();
        tasks.write(&id, b"hello\n", true).await.unwrap();
        let status = wait_for_finish(&tasks, &id).await;
        assert_eq!(status.exit_code, Some(Some(0)));
        assert!(status.output.contains("got:hello"), "{}", status.output);
    }

    #[tokio::test]
    async fn keeps_head_and_tail_when_output_is_large() {
        let mut output = OutputBuffer::default();
        output.append(b"HEAD");
        output.append(&vec![b'x'; MAX_OUTPUT_BYTES]);
        output.append(b"TAIL");
        let rendered = output.render();
        assert!(rendered.starts_with("HEAD"));
        assert!(rendered.ends_with("TAIL"));
        assert!(rendered.contains("middle bytes truncated"));
    }

    #[tokio::test]
    async fn finished_status_contains_the_drained_output_tail() {
        let tasks = BackgroundTasks::default();
        let dir = tempfile::tempdir().unwrap();
        let id = tasks
            .spawn(
                Arc::new(LocalExecutor),
                "i=0; while [ $i -lt 2000 ]; do echo line-$i; i=$((i+1)); done; echo FINAL"
                    .to_string(),
                dir.path(),
            )
            .await
            .unwrap();
        let status = wait_for_finish(&tasks, &id).await;
        assert!(status.output.contains("FINAL"), "{}", status.output);
    }

    #[tokio::test]
    async fn kill_stops_a_process_group() {
        let tasks = BackgroundTasks::default();
        let dir = tempfile::tempdir().unwrap();
        let id = tasks
            .spawn(Arc::new(LocalExecutor), "sleep 30".to_string(), dir.path())
            .await
            .unwrap();
        tasks.kill(&id).await.unwrap();
        let status = wait_for_finish(&tasks, &id).await;
        assert!(status.exit_code.is_some());
    }

    #[tokio::test]
    async fn clear_all_kills_and_forgets_tasks() {
        let tasks = BackgroundTasks::default();
        let dir = tempfile::tempdir().unwrap();
        let id = tasks
            .spawn(Arc::new(LocalExecutor), "sleep 30".to_string(), dir.path())
            .await
            .unwrap();
        tasks.clear_all().await;
        assert!(tasks.status(&id).await.is_none());
    }
}
