//! `bash` — run a shell command in the project root. This is the deliberate hole
//! in the sandbox (a command can reach anywhere), which is why it is `mutating`
//! and defaults to requiring user confirmation. Output is captured and bounded.

use std::process::Stdio;
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
                "timeout_ms": {"type": "integer", "description": "Timeout in milliseconds (default 120000, max 600000)."}
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
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(&command)
            .current_dir(ctx.sandbox.root())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return ToolOutcome::error(format!("failed to spawn shell: {e}")),
        };

        let wait = child.wait_with_output();
        let output = tokio::select! {
            _ = ctx.cancel.cancelled() => {
                return ToolOutcome::error("command cancelled");
            }
            res = tokio::time::timeout(Duration::from_millis(timeout_ms), wait) => res,
        };

        let output = match output {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => return ToolOutcome::error(format!("command failed: {e}")),
            Err(_) => {
                return ToolOutcome::error(format!("command timed out after {timeout_ms} ms"))
            }
        };

        let code = output.status.code();
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
}
