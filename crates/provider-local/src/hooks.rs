//! `PreToolUse`/`PostToolUse` hooks — v1, command-type only. Configured via
//! `.clark/settings.json`'s `hooks` key ([`crate::project_settings::HooksConfig`]).
//!
//! Fired from `DesktopToolAdapter::execute` (`agent_adapter.rs`): `PreToolUse`
//! right before the permission gate (can deny or rewrite the tool's args
//! before it ever reaches the gate/tool); `PostToolUse` right after the tool
//! runs (observe-only — can only append context to the result, never block).
//!
//! Hook commands run through the same hard-floor safety check
//! (`safety::classify_command`) `bash` uses, but — deliberately — bypass the
//! interactive permission gate: the hook config is a file the user/repo
//! already wrote (and had to approve as a `write_file`/`edit_file` edit to get
//! there in the first place), the same trust model Claude Code's own hooks use.
//!
//! The JSON payload (`{tool_name, tool_input, ...}`) rides the hook's stdin,
//! delivered by shelling `printf '%s' <json> | (<hook command>)` through the
//! same `Executor::exec()` primitive every other tool uses — no new Executor
//! primitive needed, and it works for remote/SSH projects too.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::exec::Executor;
use crate::project_settings::HookEntry;
use crate::safety::{classify_command, CommandRisk};

const HOOK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Default, Deserialize)]
struct HookDecision {
    /// `"allow"` | `"deny"`. Absent/unrecognized = allow.
    decision: Option<String>,
    reason: Option<String>,
    /// `PreToolUse` only: replaces the tool's args for the rest of the chain.
    updated_input: Option<Value>,
    /// `PostToolUse` only: appended to the tool result as extra context.
    additional_context: Option<String>,
}

pub enum PreToolUseResult {
    Allow { args: Value },
    Deny { reason: String },
}

/// Run every `PreToolUse` hook matching `tool_name`, in order. The first
/// explicit deny wins; each hook that rewrites `updated_input` passes its
/// result to the next. A hook that fails to run or returns unparseable output
/// is treated as a no-op (allow) — a broken hook shouldn't wedge every tool call.
pub async fn run_pre_tool_use(
    exec: &dyn Executor,
    cwd: &Path,
    hooks: &[HookEntry],
    tool_name: &str,
    args: Value,
    cancel: &CancellationToken,
) -> PreToolUseResult {
    let mut args = args;
    for hook in hooks.iter().filter(|h| matches(h, tool_name)) {
        if let Some(why) = hard_floor_violation(&hook.command) {
            return PreToolUseResult::Deny {
                reason: format!("hook command refused: {why}"),
            };
        }
        let payload = json!({ "tool_name": tool_name, "tool_input": args });
        let Ok(decision) = run_hook(exec, cwd, &hook.command, &payload, cancel).await else {
            continue;
        };
        if decision.decision.as_deref() == Some("deny") {
            return PreToolUseResult::Deny {
                reason: decision
                    .reason
                    .unwrap_or_else(|| format!("blocked by hook: {}", hook.command)),
            };
        }
        if let Some(updated) = decision.updated_input {
            args = updated;
        }
    }
    PreToolUseResult::Allow { args }
}

/// Run every `PostToolUse` hook matching `tool_name`, collecting any
/// `additional_context` strings to append to the tool result. Observe-only —
/// nothing here can change what already ran.
pub async fn run_post_tool_use(
    exec: &dyn Executor,
    cwd: &Path,
    hooks: &[HookEntry],
    tool_name: &str,
    args: &Value,
    outcome_content: &str,
    cancel: &CancellationToken,
) -> Vec<String> {
    let mut extra = Vec::new();
    for hook in hooks.iter().filter(|h| matches(h, tool_name)) {
        if hard_floor_violation(&hook.command).is_some() {
            continue;
        }
        let payload = json!({
            "tool_name": tool_name,
            "tool_input": args,
            "tool_output": outcome_content,
        });
        if let Ok(decision) = run_hook(exec, cwd, &hook.command, &payload, cancel).await {
            if let Some(ctx) = decision.additional_context.filter(|c| !c.is_empty()) {
                extra.push(ctx);
            }
        }
    }
    extra
}

fn matches(hook: &HookEntry, tool_name: &str) -> bool {
    hook.matcher == "*" || hook.matcher == tool_name
}

/// The same hard floor `bash` itself can't bypass — a hook can't run anything
/// `classify_command` would refuse for an ordinary shell call.
fn hard_floor_violation(command: &str) -> Option<String> {
    let c = classify_command(command);
    matches!(c.risk, CommandRisk::Blocked)
        .then(|| c.reason.unwrap_or_else(|| "blocked for safety".to_string()))
}

async fn run_hook(
    exec: &dyn Executor,
    cwd: &Path,
    hook_command: &str,
    payload: &Value,
    cancel: &CancellationToken,
) -> Result<HookDecision, String> {
    let payload_str = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    let full = format!(
        "printf '%s' {} | ({})",
        shell_single_quote(&payload_str),
        hook_command
    );
    let output = exec.exec(&full, cwd, HOOK_TIMEOUT, cancel).await?;

    // Exit code 2 is Claude Code's own "block" convention — reuse it here so
    // hook authors migrating a `.claude/settings.json` hook don't need to
    // relearn a different signal.
    if output.code == Some(2) {
        let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Ok(HookDecision {
            decision: Some("deny".to_string()),
            reason: (!reason.is_empty()).then_some(reason),
            ..Default::default()
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Ok(HookDecision::default());
    }
    Ok(serde_json::from_str(stdout).unwrap_or_default())
}

/// POSIX single-quote a string for safe embedding in a `/bin/sh -c` command.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;

    fn hook(matcher: &str, command: &str) -> HookEntry {
        HookEntry {
            matcher: matcher.to_string(),
            command: command.to_string(),
        }
    }

    #[tokio::test]
    async fn no_matching_hooks_allows_with_unchanged_args() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = vec![hook("edit_file", "exit 2")];
        let args = json!({"command": "ls"});
        let result = run_pre_tool_use(
            &LocalExecutor,
            dir.path(),
            &hooks,
            "bash",
            args.clone(),
            &CancellationToken::new(),
        )
        .await;
        match result {
            PreToolUseResult::Allow { args: a } => assert_eq!(a, args),
            PreToolUseResult::Deny { .. } => panic!("should not have matched"),
        }
    }

    #[tokio::test]
    async fn exit_code_2_denies_with_stderr_as_reason() {
        let dir = tempfile::tempdir().unwrap();
        // POSIX `sh` (dash on CI) has no `<<<` here-string; use plain redirect.
        let hooks = vec![hook("*", "echo 'nope, not that' >&2; exit 2")];
        let result = run_pre_tool_use(
            &LocalExecutor,
            dir.path(),
            &hooks,
            "bash",
            json!({"command": "rm x"}),
            &CancellationToken::new(),
        )
        .await;
        match result {
            PreToolUseResult::Deny { reason } => assert!(reason.contains("nope, not that")),
            PreToolUseResult::Allow { .. } => panic!("should have denied"),
        }
    }

    #[tokio::test]
    async fn json_deny_decision_with_reason() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = vec![hook(
            "bash",
            r#"echo '{"decision":"deny","reason":"policy violation"}'"#,
        )];
        let result = run_pre_tool_use(
            &LocalExecutor,
            dir.path(),
            &hooks,
            "bash",
            json!({"command": "x"}),
            &CancellationToken::new(),
        )
        .await;
        match result {
            PreToolUseResult::Deny { reason } => assert_eq!(reason, "policy violation"),
            PreToolUseResult::Allow { .. } => panic!("should have denied"),
        }
    }

    #[tokio::test]
    async fn updated_input_rewrites_args_and_receives_the_tool_input_on_stdin() {
        let dir = tempfile::tempdir().unwrap();
        // Echo back tool_input.command reversed-ish by just proving stdin was
        // received: the hook reads the payload and always rewrites `command`
        // to a fixed marker, regardless of input — proves the round trip works
        // without depending on any particular JSON tool (jq etc.) being present.
        let hooks = vec![hook(
            "bash",
            r#"cat >/dev/null; echo '{"updated_input":{"command":"echo rewritten"}}'"#,
        )];
        let result = run_pre_tool_use(
            &LocalExecutor,
            dir.path(),
            &hooks,
            "bash",
            json!({"command": "echo original"}),
            &CancellationToken::new(),
        )
        .await;
        match result {
            PreToolUseResult::Allow { args } => {
                assert_eq!(args["command"], "echo rewritten");
            }
            PreToolUseResult::Deny { reason } => panic!("unexpected deny: {reason}"),
        }
    }

    #[tokio::test]
    async fn hard_floor_still_blocks_a_hook_command() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = vec![hook("*", "rm -rf /")];
        let result = run_pre_tool_use(
            &LocalExecutor,
            dir.path(),
            &hooks,
            "bash",
            json!({}),
            &CancellationToken::new(),
        )
        .await;
        match result {
            PreToolUseResult::Deny { reason } => assert!(reason.contains("hook command refused")),
            PreToolUseResult::Allow { .. } => panic!("hard floor should have blocked this"),
        }
    }

    #[tokio::test]
    async fn post_tool_use_collects_additional_context() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = vec![hook(
            "*",
            r#"cat >/dev/null; echo '{"additional_context":"formatted with prettier"}'"#,
        )];
        let extra = run_post_tool_use(
            &LocalExecutor,
            dir.path(),
            &hooks,
            "write_file",
            &json!({"path": "x.ts"}),
            "wrote 10 lines",
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(extra, vec!["formatted with prettier".to_string()]);
    }

    #[tokio::test]
    async fn a_broken_hook_does_not_block_the_call() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = vec![hook("*", "no_such_binary_xyz")];
        let result = run_pre_tool_use(
            &LocalExecutor,
            dir.path(),
            &hooks,
            "bash",
            json!({"command": "ls"}),
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(result, PreToolUseResult::Allow { .. }));
    }

    #[test]
    fn shell_single_quote_escapes_embedded_quotes() {
        assert_eq!(shell_single_quote("it's"), r"'it'\''s'");
    }
}
