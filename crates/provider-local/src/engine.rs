//! The local agent loop: stream the model, run its tool calls on the local
//! machine, feed results back, repeat until it stops calling tools.
//!
//! Everything mutable lives behind `Arc<Mutex<…>>` because the loop runs in a
//! spawned task while [`crate::provider::LocalAgentProvider`] keeps serving
//! `respond`/`cancel` on the same session.

use std::collections::HashMap;
use std::sync::Arc;

use agent_core::domain::{
    AgentEvent, ContentBlock, PermissionOption, PermissionOptionKind, PermissionRequest, Role,
    RunOutcome, RunStatus, ToolCall, ToolCallPatch, ToolStatus,
};
use agent_core::ids::{PermissionRequestId, RunId, SessionId, ToolCallId};
use async_channel::Sender;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};

use crate::llm::{ChatMessage, LlmClient, LlmError, WireToolCall};
use crate::mcp::is_mcp_tool;
use crate::safety::{classify_command, CommandRisk};
use crate::tools::{PermissionMode, ToolCtx, ToolRegistry};

/// Per-session conversation state that persists across turns.
pub(crate) struct SessionState {
    pub transcript: Vec<ChatMessage>,
    /// Per-tool permission policy; "always allow/reject" mutate it in place.
    pub policy: HashMap<String, PermissionMode>,
    /// Shell-command prefixes the user always allows (skip the gate). Honored
    /// only for Safe/Caution commands, so a trusted prefix can't carry a
    /// destructive suffix past the gate. "Always allow this command" appends here.
    pub allow_commands: Vec<String>,
    /// Shell-command prefixes that are always refused.
    pub deny_commands: Vec<String>,
}

/// Live control surface for the *current* run, reachable from `respond`/`cancel`.
#[derive(Default)]
pub(crate) struct RunControl {
    pending: Option<Pending>,
}

struct Pending {
    id: PermissionRequestId,
    responder: oneshot::Sender<Decision>,
}

impl RunControl {
    /// Deliver a user's answer to the in-flight permission request. Returns
    /// `true` if a request was actually waiting.
    pub fn resolve(&mut self, id: &PermissionRequestId, decision: Decision) -> bool {
        match self.pending.take() {
            Some(p) if &p.id == id || id.as_str().is_empty() => {
                let _ = p.responder.send(decision);
                true
            }
            other => {
                self.pending = other;
                false
            }
        }
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }
}

/// How the user resolved a permission prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

impl Decision {
    pub fn from_option(option: &str) -> Decision {
        match option {
            "allow_always" => Decision::AllowAlways,
            "reject_always" => Decision::RejectAlways,
            "reject_once" | "reject" | "deny" => Decision::RejectOnce,
            // Default the affirmative/unknown case to a single allow.
            _ => Decision::AllowOnce,
        }
    }
    fn approved(self) -> bool {
        matches!(self, Decision::AllowOnce | Decision::AllowAlways)
    }
}

/// Everything `run_turn` needs, bundled to keep the signature sane.
pub(crate) struct TurnContext {
    pub llm: LlmClient,
    pub registry: Arc<ToolRegistry>,
    pub ctx: ToolCtx,
    pub session: Arc<Mutex<SessionState>>,
    pub control: Arc<Mutex<RunControl>>,
    pub session_id: SessionId,
    pub max_iterations: u32,
}

/// Drive one user turn to completion, emitting normalized events into `tx`.
pub(crate) async fn run_turn(tc: TurnContext, tx: Sender<AgentEvent>, run: RunId) {
    let cancel = tc.ctx.cancel.clone();
    let _ = tx.send(AgentEvent::RunStarted { run: run.clone() }).await;

    // Snapshot the working tree before any edits so the user can undo this run.
    // Off the async runtime (git shells out); silently skipped for non-git repos.
    let root = tc.ctx.sandbox.root().to_path_buf();
    if let Ok(Some(id)) =
        tokio::task::spawn_blocking(move || crate::checkpoint::create_checkpoint(&root)).await
    {
        let _ = tx
            .send(AgentEvent::Checkpoint {
                run: run.clone(),
                id,
            })
            .await;
    }

    let mut iterations = 0u32;
    loop {
        if cancel.is_cancelled() {
            finish(&tx, &run, RunStatus::Cancelled, None, None).await;
            return;
        }
        if iterations >= tc.max_iterations {
            let msg = format!("stopped after {} tool iterations", tc.max_iterations);
            finish(&tx, &run, RunStatus::Failed, None, Some(msg)).await;
            return;
        }
        iterations += 1;

        let messages = { tc.session.lock().await.transcript.clone() };
        let tools = tc.registry.schemas();

        // Stream the assistant turn; text deltas go straight to the UI.
        let text_tx = tx.clone();
        let text_run = run.clone();
        let turn = tc
            .llm
            .stream_chat(&messages, &tools, &cancel, |delta| {
                let _ = text_tx.try_send(AgentEvent::MessageChunk {
                    run: text_run.clone(),
                    role: Role::Agent,
                    delta: ContentBlock::text(delta),
                });
            })
            .await;

        let turn = match turn {
            Ok(t) => t,
            Err(LlmError::Cancelled) => {
                finish(&tx, &run, RunStatus::Cancelled, None, None).await;
                return;
            }
            Err(LlmError::InsufficientCredits) => {
                // Distinct signal so the UI shows an "add credits" upgrade prompt
                // (with a link to clarkchat.com) rather than a generic failure.
                let message =
                    "insufficient_credits: You're out of Clark credits. Add credits to keep coding."
                        .to_string();
                let _ = tx
                    .send(AgentEvent::Error {
                        code: "insufficient_credits".into(),
                        message: message.clone(),
                        run: Some(run.clone()),
                    })
                    .await;
                finish(&tx, &run, RunStatus::Failed, None, Some(message)).await;
                return;
            }
            Err(LlmError::Message(m)) => {
                let _ = tx
                    .send(AgentEvent::Error {
                        code: "model_error".into(),
                        message: m.clone(),
                        run: Some(run.clone()),
                    })
                    .await;
                finish(&tx, &run, RunStatus::Failed, None, Some(m)).await;
                return;
            }
        };

        // Record the assistant message (text was already streamed).
        {
            let mut s = tc.session.lock().await;
            s.transcript.push(ChatMessage {
                role: "assistant".into(),
                content: Some(turn.text.clone()).filter(|t| !t.is_empty()),
                tool_calls: turn.tool_calls.clone(),
                tool_call_id: None,
            });
        }

        // No tool calls → the model is done.
        if turn.tool_calls.is_empty() {
            finish(&tx, &run, RunStatus::Done, turn.finish_reason, None).await;
            return;
        }

        for call in &turn.tool_calls {
            if cancel.is_cancelled() {
                finish(&tx, &run, RunStatus::Cancelled, None, None).await;
                return;
            }
            match execute_call(&tc, &tx, &run, call).await {
                CallFlow::Continue => {}
                CallFlow::Cancelled => {
                    finish(&tx, &run, RunStatus::Cancelled, None, None).await;
                    return;
                }
            }
        }
    }
}

enum CallFlow {
    Continue,
    Cancelled,
}

/// Run a single tool call: gate it, execute it, emit its lifecycle, and append
/// its result to the transcript so the next model call sees it.
async fn execute_call(
    tc: &TurnContext,
    tx: &Sender<AgentEvent>,
    run: &RunId,
    call: &WireToolCall,
) -> CallFlow {
    let name = call.function.name.clone();
    let tool_id = ToolCallId::new(call.id.clone());
    let args = parse_args(&call.function.arguments);

    let exec = tc.registry.get(&name);
    let kind = exec.as_ref().map(|e| e.kind()).unwrap_or_default();

    let _ = tx
        .send(AgentEvent::ToolCall {
            run: run.clone(),
            call: ToolCall {
                id: tool_id.clone(),
                title: tool_title(&name, &args),
                kind,
                status: ToolStatus::Pending,
                locations: Vec::new(),
                content: Vec::new(),
                raw_input: Some(args.clone()),
            },
        })
        .await;

    let Some(exec) = exec else {
        return finish_call(
            tc,
            tx,
            run,
            &tool_id,
            &call.id,
            ToolStatus::Failed,
            format!("unknown tool `{name}`"),
            Vec::new(),
        )
        .await;
    };

    // Permission gate for mutating tools — with an authoritative safety floor for
    // shell commands, enforced regardless of the UI's permission mode.
    if exec.mutating() {
        let mut info = gate_info(&name, &args);
        // For edits, replace the bare path with the actual diff (computed without
        // touching disk) so the gate is a real review-before-apply.
        if let Some(diff) = exec.preview(&args, &tc.ctx) {
            info.detail = Some(diff);
        }

        // Hard refusals: catastrophic commands and the user's denylist never run,
        // even under "Full access".
        if let Some(why) = hard_refusal(tc, &name, &info).await {
            return finish_call(
                tc,
                tx,
                run,
                &tool_id,
                &call.id,
                ToolStatus::Failed,
                format!("Refused: {why}. The command was not run."),
                Vec::new(),
            )
            .await;
        }

        // Allowlisted Safe/Caution commands skip the gate entirely; everything
        // else is subject to the per-tool policy (Ask emits the gate).
        if !command_preapproved(tc, &name, &info).await {
            let mode = {
                let s = tc.session.lock().await;
                s.policy.get(&name).copied().unwrap_or(PermissionMode::Ask)
            };
            let approved = match mode {
                PermissionMode::Allow => true,
                PermissionMode::Deny => false,
                PermissionMode::Ask => match ask_permission(tc, tx, &tool_id, &name, &info).await {
                    Some(decision) => {
                        apply_policy(tc, &name, &info, decision).await;
                        decision.approved()
                    }
                    None => return CallFlow::Cancelled,
                },
            };
            if !approved {
                return finish_call(
                    tc,
                    tx,
                    run,
                    &tool_id,
                    &call.id,
                    ToolStatus::Failed,
                    format!("The user denied permission to run `{name}`."),
                    Vec::new(),
                )
                .await;
            }
        }
    }

    let args = match args {
        Value::Object(_) => args,
        Value::Null => json!({}),
        other => other,
    };

    let _ = tx
        .send(AgentEvent::ToolCallUpdate {
            run: run.clone(),
            id: tool_id.clone(),
            patch: ToolCallPatch {
                status: Some(ToolStatus::InProgress),
                ..Default::default()
            },
        })
        .await;

    let outcome = exec.invoke(args, &tc.ctx).await;
    let status = if outcome.is_error {
        ToolStatus::Failed
    } else {
        ToolStatus::Completed
    };
    let locations = outcome.locations.clone();
    finish_call(
        tc,
        tx,
        run,
        &tool_id,
        &call.id,
        status,
        outcome.content,
        locations,
    )
    .await
}

/// Emit the terminal tool update and append the tool result to the transcript.
#[allow(clippy::too_many_arguments)]
async fn finish_call(
    tc: &TurnContext,
    tx: &Sender<AgentEvent>,
    run: &RunId,
    tool_id: &ToolCallId,
    call_id: &str,
    status: ToolStatus,
    content: String,
    locations: Vec<agent_core::domain::FsLocation>,
) -> CallFlow {
    let _ = tx
        .send(AgentEvent::ToolCallUpdate {
            run: run.clone(),
            id: tool_id.clone(),
            patch: ToolCallPatch {
                status: Some(status),
                locations: (!locations.is_empty()).then_some(locations),
                append_content: vec![ContentBlock::text(content.clone())],
                ..Default::default()
            },
        })
        .await;
    tc.session
        .lock()
        .await
        .transcript
        .push(ChatMessage::tool(call_id, content));
    CallFlow::Continue
}

/// Arm a permission request, emit it, and await the user's decision. Returns
/// `None` if the run was cancelled while waiting.
async fn ask_permission(
    tc: &TurnContext,
    tx: &Sender<AgentEvent>,
    tool_id: &ToolCallId,
    tool_name: &str,
    info: &GateInfo,
) -> Option<Decision> {
    let request_id = PermissionRequestId::new(format!("perm-{}", tool_id.as_str()));
    let (responder, rx) = oneshot::channel();
    {
        let mut control = tc.control.lock().await;
        control.pending = Some(Pending {
            id: request_id.clone(),
            responder,
        });
    }

    let _ = tx
        .send(AgentEvent::PermissionRequest {
            request: PermissionRequest {
                id: request_id,
                session: tc.session_id.clone(),
                tool_call: Some(tool_id.clone()),
                title: permission_title(tool_name),
                options: permission_options(tool_name),
                detail: info.detail.clone(),
                risk: if info.external {
                    Some("external".to_string())
                } else {
                    info.risk.map(|r| r.as_str().to_string())
                },
                reason: info.reason.clone(),
            },
        })
        .await;

    let decision = tokio::select! {
        _ = tc.ctx.cancel.cancelled() => None,
        d = rx => d.ok(),
    };
    if decision.is_none() {
        tc.control.lock().await.clear();
    }
    decision
}

/// What a mutating call will do, used to gate it and to show the user exactly
/// what they're approving.
struct GateInfo {
    /// The shell command (bash), target path/diff (edit/write), or tool+args
    /// (MCP), shown verbatim.
    detail: Option<String>,
    /// Shell-command risk class; None for file edits (sandboxed + checkpointed).
    risk: Option<CommandRisk>,
    /// Why a command was flagged ("recursive delete").
    reason: Option<String>,
    /// An external (MCP) tool — gated on first use even in auto mode.
    external: bool,
}

fn gate_info(name: &str, args: &Value) -> GateInfo {
    if is_mcp_tool(name) {
        let pretty = serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
        return GateInfo {
            detail: Some(format!("{name}\n{pretty}")),
            risk: None,
            reason: Some("external MCP tool — review its inputs".to_string()),
            external: true,
        };
    }
    match name {
        "bash" => {
            let cmd = args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let c = classify_command(&cmd);
            GateInfo {
                detail: Some(cmd),
                risk: Some(c.risk),
                reason: c.reason,
                external: false,
            }
        }
        "write_file" | "edit_file" => GateInfo {
            detail: args.get("path").and_then(Value::as_str).map(str::to_string),
            risk: None,
            reason: None,
            external: false,
        },
        _ => GateInfo {
            detail: None,
            risk: None,
            reason: None,
            external: false,
        },
    }
}

/// A refusal reason if this call must be denied no matter the permission mode —
/// a catastrophic (`Blocked`) shell command or a user denylist hit. `None` means
/// it's subject to normal gating.
async fn hard_refusal(tc: &TurnContext, name: &str, info: &GateInfo) -> Option<String> {
    if name != "bash" {
        return None;
    }
    if matches!(info.risk, Some(CommandRisk::Blocked)) {
        return Some(
            info.reason
                .clone()
                .unwrap_or_else(|| "blocked for safety".to_string()),
        );
    }
    let cmd = info.detail.clone().unwrap_or_default();
    let s = tc.session.lock().await;
    s.deny_commands
        .iter()
        .any(|d| prefix_match(&cmd, d))
        .then(|| "on your command denylist".to_string())
}

/// Whether a Safe/Caution shell command matches the allowlist and may skip the
/// gate. The risk guard ensures a trusted prefix can't carry a destructive tail
/// (a `cargo test` allow never approves `cargo test && rm -rf …`, which is
/// classified Danger).
async fn command_preapproved(tc: &TurnContext, name: &str, info: &GateInfo) -> bool {
    if name != "bash"
        || !matches!(
            info.risk,
            Some(CommandRisk::Safe) | Some(CommandRisk::Caution)
        )
    {
        return false;
    }
    let cmd = info.detail.clone().unwrap_or_default();
    let s = tc.session.lock().await;
    s.allow_commands.iter().any(|a| prefix_match(&cmd, a))
}

/// `cmd` matches an allow/deny entry if it equals it or extends it with a space
/// (so `cargo test` matches `cargo test --workspace` but not `cargo testfoo`).
fn prefix_match(cmd: &str, entry: &str) -> bool {
    let cmd = cmd.trim();
    let entry = entry.trim();
    !entry.is_empty() && (cmd == entry || cmd.starts_with(&format!("{entry} ")))
}

async fn apply_policy(tc: &TurnContext, tool: &str, info: &GateInfo, decision: Decision) {
    let mut s = tc.session.lock().await;
    match decision {
        Decision::AllowAlways => {
            // For bash, "always" means this specific command, not all of bash.
            if tool == "bash" {
                if let Some(cmd) = info.detail.as_deref() {
                    let entry = cmd.trim().to_string();
                    if !entry.is_empty() && !s.allow_commands.contains(&entry) {
                        s.allow_commands.push(entry);
                    }
                }
            } else {
                s.policy.insert(tool.to_string(), PermissionMode::Allow);
            }
        }
        Decision::RejectAlways => {
            s.policy.insert(tool.to_string(), PermissionMode::Deny);
        }
        _ => {}
    }
}

fn permission_title(tool: &str) -> String {
    match tool {
        "bash" => "Run a shell command?".to_string(),
        "edit_file" => "Apply this edit?".to_string(),
        "write_file" => "Write this file?".to_string(),
        t if is_mcp_tool(t) => "Run an MCP tool?".to_string(),
        other => format!("Allow `{other}` to run?"),
    }
}

fn permission_options(tool: &str) -> Vec<PermissionOption> {
    let always = if tool == "bash" {
        "Always allow this command".to_string()
    } else if is_mcp_tool(tool) {
        "Always allow this tool".to_string()
    } else {
        format!("Always allow {tool}")
    };
    vec![
        PermissionOption {
            id: "allow_once".into(),
            label: "Allow once".into(),
            kind: PermissionOptionKind::AllowOnce,
        },
        PermissionOption {
            id: "allow_always".into(),
            label: always,
            kind: PermissionOptionKind::AllowAlways,
        },
        PermissionOption {
            id: "reject_once".into(),
            label: "Reject".into(),
            kind: PermissionOptionKind::RejectOnce,
        },
    ]
}

/// Parse tool arguments; an empty/blank string means "no arguments".
fn parse_args(raw: &str) -> Value {
    if raw.trim().is_empty() {
        return json!({});
    }
    serde_json::from_str(raw).unwrap_or(Value::Null)
}

/// A short, human-readable title for a tool call from its salient argument.
fn tool_title(name: &str, args: &Value) -> String {
    let salient = ["path", "pattern", "command", "query", "old_string"]
        .iter()
        .find_map(|k| args.get(*k).and_then(Value::as_str));
    match salient {
        Some(a) => {
            let snippet: String = a.lines().next().unwrap_or("").chars().take(80).collect();
            format!("{name}: {snippet}")
        }
        None => name.to_string(),
    }
}

async fn finish(
    tx: &Sender<AgentEvent>,
    run: &RunId,
    status: RunStatus,
    stop_reason: Option<String>,
    error: Option<String>,
) {
    let _ = tx
        .send(AgentEvent::RunFinished {
            run: run.clone(),
            outcome: RunOutcome {
                status,
                stop_reason,
                error,
            },
        })
        .await;
    tx.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_maps_from_option_ids() {
        assert_eq!(Decision::from_option("allow_once"), Decision::AllowOnce);
        assert_eq!(Decision::from_option("allow_always"), Decision::AllowAlways);
        assert_eq!(Decision::from_option("reject_once"), Decision::RejectOnce);
        assert_eq!(
            Decision::from_option("reject_always"),
            Decision::RejectAlways
        );
        assert!(Decision::AllowAlways.approved());
        assert!(!Decision::RejectOnce.approved());
    }

    #[test]
    fn parse_args_handles_blank_and_invalid() {
        assert_eq!(parse_args(""), json!({}));
        assert_eq!(parse_args("  "), json!({}));
        assert_eq!(parse_args(r#"{"a":1}"#), json!({"a":1}));
        assert_eq!(parse_args("not json"), Value::Null);
    }

    #[test]
    fn tool_title_uses_salient_arg() {
        assert_eq!(
            tool_title("read_file", &json!({"path":"src/a.rs"})),
            "read_file: src/a.rs"
        );
        assert_eq!(
            tool_title("bash", &json!({"command":"ls -la"})),
            "bash: ls -la"
        );
        assert_eq!(tool_title("noargs", &json!({})), "noargs");
    }

    #[test]
    fn run_control_resolves_matching_request() {
        let mut control = RunControl::default();
        let (tx, rx) = oneshot::channel();
        let id = PermissionRequestId::new("perm-1");
        control.pending = Some(Pending {
            id: id.clone(),
            responder: tx,
        });
        assert!(control.resolve(&id, Decision::AllowOnce));
        assert_eq!(rx.blocking_recv().ok(), Some(Decision::AllowOnce));
        // Second resolve finds nothing pending.
        assert!(!control.resolve(&id, Decision::AllowOnce));
    }
}
