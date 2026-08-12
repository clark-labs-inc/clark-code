//! Explicit retrieval from Scout's already-materialized enterprise graph and
//! one-off, human-confirmed implementation feedback.
//!
//! This tool cannot start or refresh Scout. Tenant and repository identity are
//! injected by the host so the model cannot redirect a query across scopes.
//! Feedback is stored separately and never starts or mutates a Scout run.

use std::sync::Arc;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ToolCtx, ToolExecutor, ToolOutcome};
use crate::platform::{
    FeatureContextFeedbackRequest, FeatureContextQueryKind, FeatureContextRequest,
    FeatureContextRevision,
};

#[derive(Clone, Debug, Default)]
pub struct FeatureContextBinding {
    pub repository_fingerprint: Option<String>,
    pub organization_id: Option<String>,
    pub workspace_id: Option<String>,
}

pub struct FeatureContextTool {
    provider: Arc<dyn crate::platform::PlatformContextProvider>,
    binding: FeatureContextBinding,
}

pub struct FeatureContextFeedbackTool {
    provider: Arc<dyn crate::platform::PlatformContextProvider>,
}

impl FeatureContextTool {
    pub fn new(
        provider: Arc<dyn crate::platform::PlatformContextProvider>,
        binding: FeatureContextBinding,
    ) -> Self {
        Self { provider, binding }
    }
}

impl FeatureContextFeedbackTool {
    pub fn new(provider: Arc<dyn crate::platform::PlatformContextProvider>) -> Self {
        Self { provider }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    action: FeatureContextQueryKind,
    #[serde(default)]
    query: String,
    #[serde(default)]
    object_ids: Vec<String>,
    #[serde(default)]
    target_object_ids: Vec<String>,
    #[serde(default)]
    changed_since_ms: Option<u64>,
    #[serde(default = "default_depth")]
    max_depth: u8,
    #[serde(default = "default_limit")]
    limit: u16,
}

fn default_depth() -> u8 {
    2
}

fn default_limit() -> u16 {
    96
}

#[async_trait]
impl ToolExecutor for FeatureContextTool {
    fn name(&self) -> &str {
        "enterprise_context"
    }

    fn description(&self) -> &str {
        "Query Scout's existing evidence-backed enterprise graph for feature work. Supports task-oriented packets, exact identity resolution, search, neighborhoods, paths, impact, changes since a pinned time, and status. This is read-only: it never starts, schedules, refreshes, or mutates a Scout run. Organization, workspace, and local repository identity are fixed by the host, not model arguments."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["task", "resolve", "search", "neighborhood", "paths", "impact", "changed_since", "status"],
                    "description": "Choose the bounded graph read before supplying selectors."
                },
                "query": {
                    "type": "string",
                    "description": "Feature question or search text. Required for task and search."
                },
                "object_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "maxItems": 32,
                    "description": "Exact graph object IDs used as source selectors."
                },
                "target_object_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "maxItems": 32,
                    "description": "Exact destination IDs for path queries."
                },
                "changed_since_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Required for changed_since; business-effective Unix milliseconds."
                },
                "max_depth": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 4,
                    "default": 2
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 256,
                    "default": 96
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Research
    }

    async fn invoke(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        let args: Args = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(error) => {
                return ToolOutcome::error(format!("invalid enterprise context query: {error}"))
            }
        };
        if args.object_ids.len() > 32 || args.target_object_ids.len() > 32 {
            return ToolOutcome::error("enterprise context selectors accept at most 32 object IDs");
        }
        if !(1..=4).contains(&args.max_depth) || !(1..=256).contains(&args.limit) {
            return ToolOutcome::error(
                "enterprise context depth or limit is outside its bounded range",
            );
        }
        if matches!(
            args.action,
            FeatureContextQueryKind::Task | FeatureContextQueryKind::Search
        ) && args.query.trim().is_empty()
        {
            return ToolOutcome::error("task and search require a non-empty query");
        }
        if matches!(args.action, FeatureContextQueryKind::ChangedSince)
            && args.changed_since_ms.is_none()
        {
            return ToolOutcome::error("changed_since requires changed_since_ms");
        }
        let request = FeatureContextRequest {
            action: args.action,
            query: args.query,
            repository_fingerprint: self.binding.repository_fingerprint.clone(),
            organization_id: self.binding.organization_id.clone(),
            workspace_id: self.binding.workspace_id.clone(),
            object_ids: args.object_ids,
            target_object_ids: args.target_object_ids,
            changed_since_ms: args.changed_since_ms,
            max_depth: args.max_depth,
            pinned_revision: None,
            max_objects: args.limit,
        };
        match self.provider.feature_context(&request).await {
            Ok(response) => match crate::platform::feature_context_section(&response) {
                Some(section) => ToolOutcome::ok(section),
                None => ToolOutcome::ok(format!(
                    "No enterprise context is available ({})",
                    response
                        .unavailable_reason
                        .as_deref()
                        .unwrap_or("no matches")
                )),
            },
            Err(error) => ToolOutcome::error(error),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeedbackArgs {
    outcome: String,
    summary: String,
    #[serde(default)]
    evidence_refs: Vec<String>,
}

fn feedback_args(value: Value) -> Result<FeedbackArgs, String> {
    let args: FeedbackArgs = serde_json::from_value(value)
        .map_err(|error| format!("invalid Scout feedback: {error}"))?;
    let valid_outcome = matches!(
        args.outcome.as_str(),
        "verified" | "diverged" | "partially_verified" | "not_verified"
    );
    let valid_evidence = args.evidence_refs.len() <= 32
        && args.evidence_refs.iter().all(|reference| {
            !reference.is_empty()
                && reference.len() <= 512
                && reference.chars().all(|character| {
                    character.is_ascii_alphanumeric()
                        || matches!(character, '-' | '_' | ':' | '.' | '/' | '@' | '#')
                })
        });
    if !valid_outcome
        || args.summary.trim().is_empty()
        || args.summary.len() > 4_096
        || !valid_evidence
    {
        return Err("Scout feedback outcome, summary, or evidence references are invalid".into());
    }
    Ok(args)
}

#[async_trait]
impl ToolExecutor for FeatureContextFeedbackTool {
    fn name(&self) -> &str {
        "enterprise_context_feedback"
    }

    fn description(&self) -> &str {
        "Submit an implementation outcome against the exact enterprise context revision pinned in the human-approved plan. This always requires a fresh human confirmation, cannot be remembered or preapproved, and records a separate feedback receipt rather than mutating Scout's authoritative graph or starting a scan."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "outcome": {
                    "type": "string",
                    "enum": ["verified", "diverged", "partially_verified", "not_verified"]
                },
                "summary": {
                    "type": "string",
                    "description": "Concise implementation result for the human to review before submission."
                },
                "evidence_refs": {
                    "type": "array",
                    "items": { "type": "string" },
                    "maxItems": 32,
                    "description": "Non-secret test, commit, deployment, or artifact receipt references."
                }
            },
            "required": ["outcome", "summary"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    fn mutating(&self) -> bool {
        true
    }

    fn permission_class(&self) -> super::ToolPermissionClass {
        super::ToolPermissionClass::External
    }

    fn permission_preflight(&self, args: &Value) -> Result<(), String> {
        feedback_args(args.clone()).map(|_| ())
    }

    fn permission_scope(&self, _args: &Value) -> Option<super::PermissionScope> {
        Some(super::PermissionScope {
            key: "enterprise_context_feedback:one_off".into(),
            title: Some("Submit this result to Scout?".into()),
            always_label: None,
            reason: Some(
                "This records a durable organization/workspace feedback receipt. It does not start or refresh Scout."
                    .into(),
            ),
            risk: Some("confirm".into()),
            remember: false,
            preapproved: false,
        })
    }

    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        args.get("summary")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args = match feedback_args(args) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::error(error),
        };
        let session = ctx.session.lock().await;
        let Some(plan) = session
            .planning
            .proposed_plan
            .as_ref()
            .filter(|plan| plan.status == agent_core::domain::ProposedPlanStatus::Approved)
        else {
            return ToolOutcome::error("Scout feedback requires a human-approved plan");
        };
        let Some(pin) = plan
            .context_revisions
            .iter()
            .find(|revision| revision.context_kind == "enterprise_feature_context")
        else {
            return ToolOutcome::error("the approved plan has no pinned enterprise context");
        };
        let (Some(organization_id), Some(workspace_id)) =
            (pin.organization_id.clone(), pin.workspace_id.clone())
        else {
            return ToolOutcome::error("the pinned enterprise context has no tenant binding");
        };
        let request = FeatureContextFeedbackRequest {
            organization_id,
            workspace_id,
            revision: FeatureContextRevision {
                effective_at_ms: pin.effective_at_ms,
                known_at_ms: pin.known_at_ms,
                selector_sha256: pin.selector_sha256.clone(),
            },
            plan_id: plan.id.clone(),
            outcome: args.outcome,
            summary: args.summary.trim().to_string(),
            evidence_refs: args.evidence_refs,
        };
        drop(session);
        match self
            .provider
            .submit_feature_context_feedback(&request)
            .await
        {
            Ok(receipt) => ToolOutcome::ok(format!(
                "The human-approved Scout feedback receipt was recorded: {}",
                receipt.feedback_id
            )),
            Err(error) => ToolOutcome::error(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use agent_core::domain::{PlanContextRevision, ProposedPlan, ProposedPlanStatus};
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[derive(Default)]
    struct CapturingContext {
        feedback: Mutex<Option<FeatureContextFeedbackRequest>>,
    }

    #[async_trait]
    impl crate::platform::PlatformContextProvider for CapturingContext {
        async fn personal_memories(&self) -> Result<Vec<crate::platform::PersonalMemory>, String> {
            Ok(Vec::new())
        }

        async fn repository_context(
            &self,
            _fingerprint: &str,
            _query: &str,
        ) -> Result<crate::platform::RepositoryContext, String> {
            Err("not configured".into())
        }

        async fn organization_knowledge(
            &self,
            query: &str,
            _organization_id: Option<&str>,
            _limit: i64,
        ) -> Result<crate::platform::OrganizationKnowledgeResponse, String> {
            Ok(crate::platform::OrganizationKnowledgeResponse {
                query: query.into(),
                organizations: Vec::new(),
            })
        }

        async fn feature_context(
            &self,
            request: &crate::platform::FeatureContextRequest,
        ) -> Result<crate::platform::FeatureContextResponse, String> {
            Ok(crate::platform::FeatureContextResponse {
                query: request.query.clone(),
                packets: Vec::new(),
                unavailable_reason: None,
            })
        }

        async fn submit_feature_context_feedback(
            &self,
            request: &FeatureContextFeedbackRequest,
        ) -> Result<crate::platform::FeatureContextFeedbackReceipt, String> {
            *self.feedback.lock().unwrap() = Some(request.clone());
            Ok(crate::platform::FeatureContextFeedbackReceipt {
                feedback_id: "feedback-1".into(),
                feedback_sha256: "a".repeat(64),
                accepted_at_ms: 12,
                authority: "human_approved_feature_feedback".into(),
            })
        }
    }

    fn context(root: &std::path::Path, plan: ProposedPlan) -> ToolCtx {
        let mut session = crate::loop_state::SessionState::default();
        session.planning.proposed_plan = Some(plan);
        ToolCtx {
            sandbox: Arc::new(crate::sandbox::Sandbox::new(root).unwrap()),
            executor: Arc::new(crate::exec::LocalExecutor),
            reads: Arc::new(Mutex::new(super::super::ReadTracker::default())),
            cancel: CancellationToken::new(),
            background: Arc::new(crate::background::BackgroundTasks::default()),
            session: Arc::new(tokio::sync::Mutex::new(session)),
            progress: None,
            agent_progress: None,
            call_progress: None,
            model_override: None,
        }
    }

    #[tokio::test]
    async fn feedback_tenant_and_revision_come_only_from_the_approved_plan() {
        let provider = Arc::new(CapturingContext::default());
        let tool = FeatureContextFeedbackTool::new(provider.clone());
        let plan = ProposedPlan {
            id: "plan-7".into(),
            revision: 1,
            markdown: "Implement checkout".into(),
            status: ProposedPlanStatus::Approved,
            global_reminders: Vec::new(),
            execution_contract: Vec::new(),
            context_revisions: vec![PlanContextRevision {
                context_kind: "enterprise_feature_context".into(),
                organization_id: Some("org-host".into()),
                workspace_id: Some("workspace-host".into()),
                query: "change checkout".into(),
                effective_at_ms: 10,
                known_at_ms: 11,
                selector_sha256: "b".repeat(64),
            }],
        };
        let root = tempfile::tempdir().unwrap();

        assert!(tool
            .permission_preflight(&json!({
                "outcome": "verified",
                "summary": "Targeted tests passed.",
                "organization_id": "org-model-supplied"
            }))
            .is_err());

        let outcome = tool
            .invoke(
                json!({
                    "outcome": "verified",
                    "summary": "Targeted tests passed.",
                    "evidence_refs": ["test:checkout"]
                }),
                &context(root.path(), plan),
            )
            .await;

        assert!(!outcome.is_error);
        let feedback = provider.feedback.lock().unwrap().clone().unwrap();
        assert_eq!(feedback.organization_id, "org-host");
        assert_eq!(feedback.workspace_id, "workspace-host");
        assert_eq!(feedback.plan_id, "plan-7");
        assert_eq!(feedback.revision.effective_at_ms, 10);
        assert_eq!(feedback.revision.known_at_ms, 11);
        assert_eq!(feedback.revision.selector_sha256, "b".repeat(64));
    }
}
