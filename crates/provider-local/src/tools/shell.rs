//! `bash` — run a shell command in the project root. This is the deliberate hole
//! in the sandbox (a command can reach anywhere), which is why it is `mutating`
//! and defaults to requiring user confirmation. Output is captured completely.

use std::sync::Arc;
use std::time::Duration;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{arg_str, ToolCtx, ToolExecutor, ToolOutcome};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
const DEFAULT_WAIT_POLL_MS: u64 = 250;
const MIN_WAIT_POLL_MS: u64 = 50;
const MAX_WAIT_POLL_MS: u64 = 2_000;

pub struct Bash;

#[async_trait]
impl ToolExecutor for Bash {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "Run a shell command in the project root and return its complete output and exit code. Use it for builds, tests, git, and other tooling. On Windows, commands use PowerShell without user profiles (with CMD only as a fallback), so use PowerShell syntax and spell native utilities explicitly, for example `where.exe`. Avoid using it to search or read files: prefer grep over `grep`/`rg`, glob over `find`/`ls`, read_file over `cat`/`head`/`tail`, and edit_file/write_file over `sed`/`echo >`. The shell does not persist state (cwd, env) between calls; each command starts fresh in the project root."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The shell command to run."},
                "workdir": {"type": "string", "description": "Optional working directory inside the project, relative to the project root."},
                "sandbox_permissions": {
                    "type": "string",
                    "enum": ["use_default", "require_escalated"],
                    "description": "Use require_escalated only when the command needs network access, Git metadata writes, or another host resource outside the project sandbox. Clark Code will request approval unless Full access is active."
                },
                "justification": {"type": "string", "description": "Short user-facing reason escalated access is needed."},
                "effect": {
                    "type": "string",
                    "enum": ["none", "create", "update", "publish", "send", "delete", "mutate"],
                    "description": "Declare the durable or externally visible effect of this command. Use none for inspection. For a mutation across the host/network boundary, choose its generic action so Clark Code requires canonical read-back before completion."
                },
                "effect_target": {"type": "string", "description": "Optional user-facing target hint for the declared effect, such as a resource URL, deployment, branch, or message recipient. Do not include secrets."},
                "run_in_background": {"type": "boolean", "description": "Start a long-lived command (e.g. a dev server) without blocking; returns a task id immediately. Poll with bash_output, send input with bash_input, and stop with bash_kill."},
                "timeout_ms": {"type": "integer", "description": "Timeout in milliseconds (default 120000, max 600000). Ignored when run_in_background is true."}
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
    fn effect_intent(&self, args: &Value) -> Option<crate::effects::EffectIntent> {
        let command = args.get("command").and_then(Value::as_str)?;
        if args.get("effect").and_then(Value::as_str) == Some("none") {
            return None;
        }
        let declared_action = match args.get("effect").and_then(Value::as_str) {
            Some("create") => Some(crate::effects::EffectAction::Create),
            Some("update") => Some(crate::effects::EffectAction::Update),
            Some("publish") => Some(crate::effects::EffectAction::Publish),
            Some("send") => Some(crate::effects::EffectAction::Send),
            Some("delete") => Some(crate::effects::EffectAction::Delete),
            Some("mutate") => Some(crate::effects::EffectAction::Mutate),
            _ => None,
        };
        let explicitly_escalated = args
            .get("sandbox_permissions")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "require_escalated");
        let crosses_host_boundary =
            explicitly_escalated || crate::safety::command_requires_host(command);
        if !crosses_host_boundary || crate::safety::is_read_only_command(command) {
            return None;
        }
        let target_hint = args
            .get("effect_target")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        Some(crate::effects::EffectIntent::declared_external(
            declared_action.unwrap_or(crate::effects::EffectAction::Mutate),
            target_hint,
            "an opaque host or external command reported a successful mutation",
        ))
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let command = match arg_str(&args, "command") {
            Ok(c) => c,
            Err(e) => return ToolOutcome::error(e),
        };
        let workdir = args.get("workdir").and_then(Value::as_str).unwrap_or(".");
        let cwd = match ctx.sandbox.resolve_existing(workdir) {
            Ok(path) => path,
            Err(error) => return ToolOutcome::error(error),
        };
        let executor = command_executor(&args, &command, ctx);
        match executor.metadata(&cwd).await {
            Ok(meta) if meta.is_dir => {}
            Ok(_) => return ToolOutcome::error(format!("{workdir} is not a directory")),
            Err(error) => return ToolOutcome::error(error),
        }

        if args
            .get("run_in_background")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return match ctx.background.spawn(executor, command, &cwd).await {
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

        // Agent-authored commands are non-interactive. Keep them on the
        // pipe-backed streaming path so GUI-hosted Windows runs cannot create a
        // visible console or Windows Terminal tab. Interactive terminal
        // consumers use `exec_streaming_pty` explicitly at their own boundary.
        // The executor still decides whether this runs locally or remotely.
        let output = match executor
            .exec_streaming(
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
            body.push_str(&stdout);
            if !body.ends_with('\n') {
                body.push('\n');
            }
        }
        if !stderr.trim().is_empty() {
            body.push_str("--- stderr ---\n");
            body.push_str(&stderr);
        }
        if stdout.trim().is_empty() && stderr.trim().is_empty() {
            body.push_str("(no output)");
        }
        let mut outcome = ToolOutcome::ok(body);
        outcome.is_error = !matches!(code, Some(0));
        outcome
    }
}

fn command_executor(args: &Value, command: &str, ctx: &ToolCtx) -> Arc<dyn crate::exec::Executor> {
    if requires_scoped_host(args, command, ctx.executor.containment()) {
        // The permission gate already obtained a scoped approval for this exact
        // command. Swap only this invocation to host execution; the session's
        // normal executor remains sandboxed for subsequent tools.
        Arc::new(crate::exec::LocalExecutor)
    } else {
        ctx.executor.clone()
    }
}

fn requires_scoped_host(
    args: &Value,
    command: &str,
    containment: exec_core::ExecutionContainment,
) -> bool {
    let explicitly_requested = args
        .get("sandbox_permissions")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "require_escalated");
    let needs_host = explicitly_requested || crate::safety::command_requires_host(command);
    needs_host && containment == exec_core::ExecutionContainment::Managed
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
        let is_error =
            status.error.is_some() || matches!(status.exit_code, Some(code) if code != Some(0));
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
            body.push_str(&status.output);
        }
        let mut outcome = ToolOutcome::ok(body);
        outcome.is_error = is_error;
        outcome
    }
}

pub struct BashInput;

pub struct BashWait;

#[async_trait]
impl ToolExecutor for BashWait {
    fn name(&self) -> &str {
        "bash_wait"
    }
    fn description(&self) -> &str {
        "Wait inside the host for a background task to finish or emit a readiness marker. Use this instead of repeatedly calling bash_output: the host polls without additional model turns or tokens. A timeout does not stop the task."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string", "description": "The id returned by bash(run_in_background: true)."},
                "output_contains": {"type": "string", "description": "Optional exact text marker that means the task is ready. Without it, wait for process exit."},
                "timeout_ms": {"type": "integer", "description": "Maximum host wait in milliseconds (default 120000, max 600000). The process keeps running after a timeout."},
                "poll_interval_ms": {"type": "integer", "description": "Host polling interval in milliseconds (default 250, range 50-2000)."}
            },
            "required": ["task_id"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let task_id = match arg_str(&args, "task_id") {
            Ok(id) => id,
            Err(error) => return ToolOutcome::error(error),
        };
        let output_contains = args
            .get("output_contains")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);
        let poll_ms = args
            .get("poll_interval_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_WAIT_POLL_MS)
            .clamp(MIN_WAIT_POLL_MS, MAX_WAIT_POLL_MS);
        let waited = match ctx
            .background
            .wait(
                &task_id,
                output_contains.as_deref(),
                Duration::from_millis(timeout_ms),
                Duration::from_millis(poll_ms),
                &ctx.cancel,
            )
            .await
        {
            Ok(waited) => waited,
            Err(error) => return ToolOutcome::error(error),
        };
        match waited.outcome {
            crate::background::TaskWaitOutcome::Ready => {
                waited_outcome(&task_id, waited.status, true, waited.waited)
            }
            crate::background::TaskWaitOutcome::Finished => {
                let mut outcome =
                    waited_outcome(&task_id, waited.status, false, waited.waited);
                if output_contains.is_some() {
                    outcome.is_error = true;
                    outcome
                        .content
                        .push_str("\nreadiness marker was not observed");
                }
                outcome
            }
            crate::background::TaskWaitOutcome::TimedOut => ToolOutcome::error(format!(
                "timed out after {} ms waiting for `{task_id}`; the task is still running and was not stopped",
                waited.waited.as_millis()
            ))
            .with_details(json!({
                "task_id": task_id,
                "status": "timed_out",
                "process_still_running": true,
                "waited_ms": waited.waited.as_millis()
            })),
        }
    }
}

fn waited_outcome(
    task_id: &str,
    status: crate::background::TaskStatus,
    marker_seen: bool,
    elapsed: Duration,
) -> ToolOutcome {
    let state = if marker_seen { "ready" } else { "finished" };
    let exit_code = status.exit_code.flatten();
    let mut body = format!(
        "task_id: {task_id}\ncommand: {}\nstatus: {state}\nwaited_ms: {}\n",
        status.command,
        elapsed.as_millis()
    );
    if let Some(code) = exit_code {
        body.push_str(&format!("exit_code: {code}\n"));
    } else if status.exit_code == Some(None) {
        body.push_str("exit_code: signal\n");
    }
    if let Some(error) = &status.error {
        body.push_str(&format!("error: {error}\n"));
    }
    body.push_str("--- output ---\n");
    if status.output.trim().is_empty() {
        body.push_str("(no output yet)");
    } else {
        body.push_str(&status.output);
    }
    let is_error =
        status.error.is_some() || matches!(status.exit_code, Some(code) if code != Some(0));
    let mut outcome = ToolOutcome::ok(body).with_details(json!({
        "task_id": task_id,
        "status": state,
        "marker_seen": marker_seen,
        "process_finished": status.exit_code.is_some(),
        "exit_code": exit_code,
        "waited_ms": elapsed.as_millis()
    }));
    outcome.is_error = is_error;
    outcome
}

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

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
