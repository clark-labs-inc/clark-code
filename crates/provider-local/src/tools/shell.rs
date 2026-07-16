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
        "Run a shell command in the project root and return its output and exit code. Use it for builds, tests, git, and other tooling. Avoid using it to search or read files: prefer grep over `grep`/`rg`, glob over `find`/`ls`, read_file over `cat`/`head`/`tail`, and edit_file/write_file over `sed`/`echo >`. Output is already bounded. The shell does not persist state (cwd, env) between calls; each command starts fresh in the project root."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The shell command to run."},
                "timeout_ms": {"type": "integer", "description": "Timeout in milliseconds (default 120000, max 600000). Ignored when run_in_background is true."},
                "workdir": {"type": "string", "description": "Optional working directory inside the project, relative to the project root."},
                "run_in_background": {"type": "boolean", "description": "Start a long-lived command (e.g. a dev server) without blocking; returns a task id immediately. Poll with bash_output, send input with bash_input, and stop with bash_kill."}
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
        let workdir = args
            .get("workdir")
            .and_then(Value::as_str)
            .unwrap_or(".");
        let cwd = match ctx.sandbox.resolve_existing(workdir) {
            Ok(path) => path,
            Err(error) => return ToolOutcome::error(error),
        };
        match ctx.executor.metadata(&cwd).await {
            Ok(meta) if meta.is_dir => {}
            Ok(_) => return ToolOutcome::error(format!("{workdir} is not a directory")),
            Err(error) => return ToolOutcome::error(error),
        }

        if args
            .get("run_in_background")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return match ctx
                .background
                .spawn(ctx.executor.clone(), command, &cwd)
                .await
            {
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
        // the executor decides. `cwd` is the project root either way. Output
        // chunks stream to the UI's tool row as the command produces them.
        let output = match ctx
            .executor
            .exec_streaming_pty(
                &command,
                &cwd,
                Duration::from_millis(timeout_ms),
                &ctx.cancel,
                &|_is_stderr, chunk| ctx.report(String::from_utf8_lossy(chunk).into_owned()),
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
        let Some(status) = ctx.background.status(&task_id).await else {
            return ToolOutcome::error(format!("no background task `{task_id}`"));
        };
        let is_error = status.error.is_some()
            || matches!(status.exit_code, Some(code) if code != Some(0));
        let mut body = format!("command: {}\n", status.command);
        match status.exit_code {
            None => body.push_str("status: running\n"),
            Some(code) => body.push_str(&format!(
                "status: finished (exit_code: {})\n",
                code.map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".into())
            )),
        }
        if let Some(error) = status.error {
            body.push_str(&format!("error: {error}\n"));
        }
        body.push_str("--- output ---\n");
        if status.output.trim().is_empty() {
            body.push_str("(no output yet)");
        } else {
            body.push_str(&clamp(&status.output));
        }
        let mut outcome = ToolOutcome::ok(body);
        outcome.is_error = is_error;
        outcome
    }
}

pub struct BashInput;

#[async_trait]
impl ToolExecutor for BashInput {
    fn name(&self) -> &str {
        "bash_input"
    }
    fn description(&self) -> &str {
        "Send text to a running background task's stdin. Set close=true when the process should receive EOF."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string", "description": "The id returned by bash(run_in_background: true)."},
                "text": {"type": "string", "description": "Text to write to stdin."},
                "close": {"type": "boolean", "description": "Close stdin after writing (default false)."}
            },
            "required": ["task_id"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let task_id = match arg_str(&args, "task_id") {
            Ok(id) => id,
            Err(error) => return ToolOutcome::error(error),
        };
        let text = args.get("text").and_then(Value::as_str).unwrap_or("");
        let close = args.get("close").and_then(Value::as_bool).unwrap_or(false);
        match ctx.background.write(&task_id, text.as_bytes(), close).await {
            Ok(()) => ToolOutcome::ok(format!("Sent input to `{task_id}`.")),
            Err(error) => ToolOutcome::error(error),
        }
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
#[path = "shell_tests.rs"]
mod tests;
