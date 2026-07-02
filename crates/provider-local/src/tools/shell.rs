//! `bash` — run a shell command in the project root. This is the deliberate hole
//! in the sandbox (a command can reach anywhere), which is why it is `mutating`
//! and defaults to requiring user confirmation. Output is captured and bounded.

use std::time::Duration;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{arg_str, ToolCtx, ToolExecutor, ToolOutcome};

const MAX_OUTPUT_BYTES: usize = 100_000;
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;

pub struct Bash;

#[async_trait]
impl ToolExecutor for Bash {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "Run a shell command in the project root and return its stdout, stderr, and exit code. Use it for builds, tests, git, and other tooling. Avoid using it to search or read files: prefer grep over `grep`/`rg`, glob over `find`/`ls`, read_file over `cat`/`head`/`tail`, and edit_file/write_file over `sed`/`echo >`. The shell does not persist state (cwd, env) between calls; each command starts fresh in the project root."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The shell command to run."},
                "timeout_ms": {"type": "integer", "description": "Timeout in milliseconds (default 120000, max 600000). Ignored when run_in_background is true."},
                "run_in_background": {"type": "boolean", "description": "Start a long-lived command (e.g. a dev server) without blocking; returns a task id immediately. Poll it with bash_output and stop it with bash_kill. Local projects only."}
            },
            "required": ["command"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let command = match arg_str(&args, "command") {
            Ok(c) => c,
            Err(e) => return ToolOutcome::error(e),
        };

        if args
            .get("run_in_background")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            if !ctx.executor.is_local() {
                return ToolOutcome::error(
                    "run_in_background isn't supported for remote projects yet — run without it.",
                );
            }
            return match ctx.background.spawn(command, ctx.sandbox.root()) {
                Ok(id) => ToolOutcome::ok(format!(
                    "Started background task `{id}`. Poll its output with \
                    bash_output(task_id=\"{id}\"); stop it with bash_kill(task_id=\"{id}\")."
                )),
                Err(e) => ToolOutcome::error(e),
            };
        }

        let timeout_ms = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        // Runs on the local machine, or on the remote host for a remote project —
        // the executor decides. `cwd` is the project root either way.
        let output = match ctx
            .executor
            .exec(
                &command,
                ctx.sandbox.root(),
                Duration::from_millis(timeout_ms),
                &ctx.cancel,
            )
            .await
        {
            Ok(o) => o,
            Err(e) => return ToolOutcome::error(e),
        };

        let code = output.code;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut body = String::new();
        body.push_str(&format!(
            "exit_code: {}\n",
            code.map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into())
        ));
        if !stdout.trim().is_empty() {
            body.push_str("--- stdout ---\n");
            body.push_str(&clamp(&stdout));
            if !body.ends_with('\n') {
                body.push('\n');
            }
        }
        if !stderr.trim().is_empty() {
            body.push_str("--- stderr ---\n");
            body.push_str(&clamp(&stderr));
        }
        if stdout.trim().is_empty() && stderr.trim().is_empty() {
            body.push_str("(no output)");
        }
        let mut outcome = ToolOutcome::ok(body);
        outcome.is_error = !matches!(code, Some(0));
        outcome
    }
}

pub struct BashOutput;

#[async_trait]
impl ToolExecutor for BashOutput {
    fn name(&self) -> &str {
        "bash_output"
    }
    fn description(&self) -> &str {
        "Poll a background task started by bash(run_in_background: true). Returns its buffered \
        output so far and, once it has exited, its exit code."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string", "description": "The id returned by bash(run_in_background: true)."}
            },
            "required": ["task_id"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let task_id = match arg_str(&args, "task_id") {
            Ok(c) => c,
            Err(e) => return ToolOutcome::error(e),
        };
        let Some(status) = ctx.background.status(&task_id) else {
            return ToolOutcome::error(format!("no background task `{task_id}`"));
        };
        let mut body = format!("command: {}\n", status.command);
        match status.exit_code {
            None => body.push_str("status: running\n"),
            Some(code) => body.push_str(&format!(
                "status: finished (exit_code: {})\n",
                code.map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".into())
            )),
        }
        body.push_str("--- output ---\n");
        if status.output.trim().is_empty() {
            body.push_str("(no output yet)");
        } else {
            body.push_str(&clamp(&status.output));
        }
        ToolOutcome::ok(body)
    }
}

pub struct BashKill;

#[async_trait]
impl ToolExecutor for BashKill {
    fn name(&self) -> &str {
        "bash_kill"
    }
    fn description(&self) -> &str {
        "Stop a background task started by bash(run_in_background: true)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string", "description": "The id returned by bash(run_in_background: true)."}
            },
            "required": ["task_id"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let task_id = match arg_str(&args, "task_id") {
            Ok(c) => c,
            Err(e) => return ToolOutcome::error(e),
        };
        match ctx.background.kill(&task_id).await {
            Ok(()) => ToolOutcome::ok(format!("Stopped `{task_id}`.")),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

fn clamp(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_BYTES {
        return s.to_string();
    }
    // Keep the tail, which usually carries the error/summary.
    let start = s.len() - MAX_OUTPUT_BYTES;
    // Snap to a char boundary.
    let start = (start..s.len())
        .find(|i| s.is_char_boundary(*i))
        .unwrap_or(s.len());
    format!("… [truncated {start} leading bytes]\n{}", &s[start..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::Sandbox;
    use crate::tools::ReadTracker;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn ctx(dir: &std::path::Path) -> ToolCtx {
        ToolCtx {
            sandbox: Arc::new(Sandbox::new(dir).unwrap()),
            reads: Arc::new(Mutex::new(ReadTracker::default())),
            cancel: CancellationToken::new(),
            executor: Arc::new(crate::exec::LocalExecutor),
            background: Arc::new(crate::background::BackgroundTasks::default()),
            session: Arc::new(tokio::sync::Mutex::new(
                crate::loop_state::SessionState::default(),
            )),
        }
    }

    #[tokio::test]
    async fn runs_command_and_captures_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let out = Bash
            .invoke(json!({"command": "echo hello"}), &ctx(dir.path()))
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("exit_code: 0"));
        assert!(out.content.contains("hello"));
    }

    #[tokio::test]
    async fn nonzero_exit_is_flagged_error() {
        let dir = tempfile::tempdir().unwrap();
        let out = Bash
            .invoke(json!({"command": "exit 3"}), &ctx(dir.path()))
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("exit_code: 3"));
    }

    #[tokio::test]
    async fn runs_in_project_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("marker.txt"), "").unwrap();
        let out = Bash
            .invoke(json!({"command": "ls"}), &ctx(dir.path()))
            .await;
        assert!(out.content.contains("marker.txt"));
    }

    #[tokio::test]
    async fn cancelled_command_reports_error() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        c.cancel.cancel();
        let out = Bash.invoke(json!({"command": "sleep 5"}), &c).await;
        assert!(out.is_error);
        assert!(out.content.contains("cancel"));
    }

    #[tokio::test]
    async fn run_in_background_returns_immediately_and_bash_output_polls_it() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let started = Bash
            .invoke(
                json!({"command": "echo bg-hi", "run_in_background": true}),
                &c,
            )
            .await;
        assert!(!started.is_error, "{}", started.content);
        assert!(started.content.contains("bg-"));

        let task_id = started
            .content
            .split('`')
            .nth(1)
            .expect("task id in backticks")
            .to_string();

        // Poll until finished (background task races the assertion).
        let mut output = String::new();
        for _ in 0..100 {
            let out = BashOutput.invoke(json!({"task_id": task_id}), &c).await;
            output = out.content;
            if output.contains("finished") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(output.contains("bg-hi"), "{output}");
        assert!(output.contains("finished"), "{output}");
    }

    #[tokio::test]
    async fn bash_kill_stops_a_background_task() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let started = Bash
            .invoke(
                json!({"command": "sleep 30", "run_in_background": true}),
                &c,
            )
            .await;
        let task_id = started
            .content
            .split('`')
            .nth(1)
            .expect("task id in backticks")
            .to_string();
        let killed = BashKill.invoke(json!({"task_id": task_id}), &c).await;
        assert!(!killed.is_error, "{}", killed.content);
    }

    #[tokio::test]
    async fn bash_output_reports_unknown_task() {
        let dir = tempfile::tempdir().unwrap();
        let out = BashOutput
            .invoke(json!({"task_id": "bg-nope"}), &ctx(dir.path()))
            .await;
        assert!(out.is_error);
    }

    struct RemoteLikeExecutor;
    #[async_trait::async_trait]
    impl crate::exec::Executor for RemoteLikeExecutor {
        fn is_local(&self) -> bool {
            false
        }
        async fn read(&self, path: &std::path::Path) -> exec_core::ExecResult<Vec<u8>> {
            crate::exec::LocalExecutor.read(path).await
        }
        async fn write(&self, path: &std::path::Path, data: &[u8]) -> exec_core::ExecResult<()> {
            crate::exec::LocalExecutor.write(path, data).await
        }
        async fn create_dir_all(&self, path: &std::path::Path) -> exec_core::ExecResult<()> {
            crate::exec::LocalExecutor.create_dir_all(path).await
        }
        async fn read_dir(
            &self,
            path: &std::path::Path,
        ) -> exec_core::ExecResult<Vec<exec_core::DirEntry>> {
            crate::exec::LocalExecutor.read_dir(path).await
        }
        async fn metadata(
            &self,
            path: &std::path::Path,
        ) -> exec_core::ExecResult<exec_core::FileMeta> {
            crate::exec::LocalExecutor.metadata(path).await
        }
        async fn walk(
            &self,
            root: &std::path::Path,
        ) -> exec_core::ExecResult<Vec<exec_core::WalkEntry>> {
            crate::exec::LocalExecutor.walk(root).await
        }
        async fn exec(
            &self,
            command: &str,
            cwd: &std::path::Path,
            timeout: std::time::Duration,
            cancel: &CancellationToken,
        ) -> exec_core::ExecResult<exec_core::ExecOutput> {
            crate::exec::LocalExecutor
                .exec(command, cwd, timeout, cancel)
                .await
        }
    }

    #[tokio::test]
    async fn run_in_background_is_rejected_for_non_local_executors() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = ctx(dir.path());
        c.executor = Arc::new(RemoteLikeExecutor);
        let out = Bash
            .invoke(json!({"command": "echo hi", "run_in_background": true}), &c)
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("remote"), "{}", out.content);
    }
}
