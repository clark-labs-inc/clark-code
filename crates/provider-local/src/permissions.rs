use std::sync::Arc;

use agent_core::domain::{AgentEvent, PermissionOption, PermissionOptionKind, PermissionRequest};
use agent_core::ids::{PermissionRequestId, SessionId, ToolCallId};
use async_channel::Sender;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::loop_state::{Decision, RunControl, SessionState};
use crate::mcp::is_mcp_tool;
use crate::safety::{classify_command, CommandRisk};
use crate::tools::{PermissionMode, ToolCtx, ToolExecutor};

#[derive(Clone)]
pub(crate) struct PermissionGate {
    session: Arc<Mutex<SessionState>>,
    control: Arc<Mutex<RunControl>>,
    session_id: SessionId,
    events: Sender<AgentEvent>,
}

pub(crate) enum PermissionOutcome {
    Allowed,
    Denied(String),
    Cancelled,
}

impl PermissionGate {
    pub fn new(
        session: Arc<Mutex<SessionState>>,
        control: Arc<Mutex<RunControl>>,
        session_id: SessionId,
        events: Sender<AgentEvent>,
    ) -> Self {
        Self {
            session,
            control,
            session_id,
            events,
        }
    }

    pub async fn check(
        &self,
        tool_id: &ToolCallId,
        tool_name: &str,
        exec: &dyn ToolExecutor,
        args: &Value,
        ctx: &ToolCtx,
        signal: &CancellationToken,
    ) -> PermissionOutcome {
        if !exec.mutating() {
            return PermissionOutcome::Allowed;
        }

        let mut info = gate_info(tool_name, args);
        if let Some(diff) = exec.preview(args, ctx) {
            info.detail = Some(diff);
        }

        if let Some(why) = self.hard_refusal(tool_name, &info).await {
            return PermissionOutcome::Denied(format!("Refused: {why}. The command was not run."));
        }

        if self.command_preapproved(tool_name, &info).await {
            return PermissionOutcome::Allowed;
        }

        let mode = {
            let s = self.session.lock().await;
            s.policy
                .get(tool_name)
                .copied()
                .unwrap_or(PermissionMode::Ask)
        };
        let approved = match mode {
            PermissionMode::Allow => true,
            PermissionMode::Deny => false,
            PermissionMode::Ask => {
                match self.ask_permission(tool_id, tool_name, &info, signal).await {
                    Some(decision) => {
                        self.apply_policy(tool_name, &info, decision).await;
                        decision.approved()
                    }
                    None => return PermissionOutcome::Cancelled,
                }
            }
        };

        if approved {
            PermissionOutcome::Allowed
        } else {
            PermissionOutcome::Denied(format!("The user denied permission to run `{tool_name}`."))
        }
    }

    async fn ask_permission(
        &self,
        tool_id: &ToolCallId,
        tool_name: &str,
        info: &GateInfo,
        signal: &CancellationToken,
    ) -> Option<Decision> {
        let request_id = PermissionRequestId::new(format!("perm-{}", tool_id.as_str()));
        let (responder, rx) = tokio::sync::oneshot::channel();
        {
            let mut control = self.control.lock().await;
            control.arm(request_id.clone(), responder);
        }

        let _ = self
            .events
            .send(AgentEvent::PermissionRequest {
                request: PermissionRequest {
                    id: request_id,
                    session: self.session_id.clone(),
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
            _ = signal.cancelled() => None,
            d = rx => d.ok(),
        };
        if decision.is_none() {
            self.control.lock().await.clear();
        }
        decision
    }

    async fn hard_refusal(&self, name: &str, info: &GateInfo) -> Option<String> {
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
        let s = self.session.lock().await;
        s.deny_commands
            .iter()
            .any(|d| prefix_match(&cmd, d))
            .then(|| "on your command denylist".to_string())
    }

    async fn command_preapproved(&self, name: &str, info: &GateInfo) -> bool {
        if name != "bash"
            || !matches!(
                info.risk,
                Some(CommandRisk::Safe) | Some(CommandRisk::Caution)
            )
        {
            return false;
        }
        let cmd = info.detail.clone().unwrap_or_default();
        let s = self.session.lock().await;
        s.allow_commands.iter().any(|a| prefix_match(&cmd, a))
    }

    async fn apply_policy(&self, tool: &str, info: &GateInfo, decision: Decision) {
        let mut s = self.session.lock().await;
        match decision {
            Decision::AllowAlways => {
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
}

struct GateInfo {
    detail: Option<String>,
    risk: Option<CommandRisk>,
    reason: Option<String>,
    external: bool,
}

fn gate_info(name: &str, args: &Value) -> GateInfo {
    if is_mcp_tool(name) {
        let pretty = serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
        return GateInfo {
            detail: Some(format!("{name}\n{pretty}")),
            risk: None,
            reason: Some("external MCP tool - review its inputs".to_string()),
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

fn prefix_match(cmd: &str, entry: &str) -> bool {
    let cmd = cmd.trim();
    let entry = entry.trim();
    !entry.is_empty() && (cmd == entry || cmd.starts_with(&format!("{entry} ")))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_matching_keeps_word_boundary() {
        assert!(prefix_match("cargo test --lib", "cargo test"));
        assert!(prefix_match("cargo test", "cargo test"));
        assert!(!prefix_match("cargo testfoo", "cargo test"));
    }

    #[test]
    fn tool_titles_are_user_facing() {
        assert_eq!(permission_title("bash"), "Run a shell command?");
        assert_eq!(permission_title("write_file"), "Write this file?");
    }
}
