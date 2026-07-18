use std::collections::BTreeSet;
use std::sync::Arc;

use agent_core::domain::ToolKind;
use agent_orchestration::{AgentPath, OrchestrationId, ReportDecision, ReportStatus, TaskId};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::schema::resolve_schema;
use super::support::event_sink;
use super::{SharedState, StoredOrchestration};
use crate::tools::{ReadCheck, ToolCtx, ToolExecutor, ToolOutcome};

pub(super) fn tool(shared: Arc<SharedState>) -> Arc<dyn ToolExecutor> {
    Arc::new(ResolveDelegation { shared })
}

struct ResolveDelegation {
    shared: Arc<SharedState>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveArgs {
    orchestration_id: String,
    decisions: Vec<DecisionArgs>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionArgs {
    task_id: String,
    decision: ReportDecision,
    feedback: Option<String>,
}

#[async_trait]
impl ToolExecutor for ResolveDelegation {
    fn name(&self) -> &str {
        "resolve_delegation"
    }

    fn description(&self) -> &str {
        "Resolve structured read-only agent reports. Accept evidence only after checking it, or request bounded rework with concrete feedback. Rework reuses the same agent context and cannot exceed the orchestration attempt cap."
    }

    fn parameters(&self) -> Value {
        resolve_schema()
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: ResolveArgs = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::error(format!("invalid resolution: {error}")),
        };
        let orchestration_id = match OrchestrationId::new(args.orchestration_id.clone()) {
            Ok(id) => id,
            Err(error) => return ToolOutcome::error(error),
        };
        let stored = {
            let map = self
                .shared
                .orchestrations
                .lock()
                .expect("orchestration lock");
            let Some(stored) = map.get(&args.orchestration_id) else {
                return ToolOutcome::error("unknown orchestration id");
            };
            StoredOrchestration {
                coordinator: stored.coordinator.clone(),
                parent_context: stored.parent_context.clone(),
            }
        };
        let snapshot = stored.coordinator.snapshot();
        let expected = snapshot
            .agents
            .values()
            .filter(|agent| agent.report_status == ReportStatus::Reported)
            .map(|agent| agent.task.id.0.clone())
            .collect::<BTreeSet<_>>();
        let mut seen = BTreeSet::new();
        let mut prepared = Vec::with_capacity(args.decisions.len());
        for decision in args.decisions {
            let task_id = match TaskId::new(decision.task_id) {
                Ok(id) => id,
                Err(error) => return ToolOutcome::error(error),
            };
            if !seen.insert(task_id.0.clone()) {
                return ToolOutcome::error(format!("duplicate decision for task {task_id}"));
            }
            let path = match AgentPath::root().child(&task_id) {
                Ok(path) => path,
                Err(error) => return ToolOutcome::error(error),
            };
            if decision.decision == ReportDecision::Rework
                && decision
                    .feedback
                    .as_deref()
                    .is_none_or(|feedback| feedback.trim().is_empty())
            {
                return ToolOutcome::error("rework decisions require non-empty feedback");
            }
            if decision.decision == ReportDecision::Accept {
                if let Err(error) = verify_parent_reads(&stored, &path, ctx).await {
                    return ToolOutcome::error(error);
                }
            }
            prepared.push((path, decision.decision, decision.feedback));
        }
        if seen != expected {
            return ToolOutcome::error(format!(
                "decisions must cover every currently reported task exactly once; expected {expected:?}"
            ));
        }
        let root_execution = ctx.session.lock().await.active_execution.clone();
        let (events, captured) = event_sink(ctx, root_execution);
        for (path, decision, feedback) in prepared {
            let result = match decision {
                ReportDecision::Accept => stored.coordinator.accept(&path, &events).map(|_| ()),
                ReportDecision::Rework => {
                    let feedback = feedback.expect("rework feedback validated above");
                    stored
                        .coordinator
                        .rework(
                            orchestration_id.clone(),
                            &path,
                            stored.parent_context.clone(),
                            feedback,
                            events.clone(),
                        )
                        .await
                }
            };
            if let Err(error) = result {
                return ToolOutcome::error(error.to_string());
            }
        }
        let snapshot = stored.coordinator.snapshot();
        let settled = snapshot.agents.values().all(|agent| {
            matches!(
                agent.report_status,
                ReportStatus::Accepted | ReportStatus::Failed
            )
        });
        if settled {
            self.shared
                .orchestrations
                .lock()
                .expect("orchestration lock")
                .remove(&args.orchestration_id);
        }
        ToolOutcome::ok(
            serde_json::to_string_pretty(&snapshot).unwrap_or_else(|error| error.to_string()),
        )
        .with_details(json!({
            "orchestration_id": orchestration_id,
            "state": snapshot,
            "events": captured.lock().expect("event lock").clone()
        }))
    }
}

async fn verify_parent_reads(
    stored: &StoredOrchestration,
    path: &AgentPath,
    ctx: &ToolCtx,
) -> Result<(), String> {
    let record = stored
        .coordinator
        .snapshot()
        .agents
        .get(path)
        .cloned()
        .ok_or_else(|| format!("unknown reported task: {path}"))?;
    let report = record
        .report
        .ok_or_else(|| format!("task {path} has no report to accept"))?;
    for claim in report.claims {
        let reference = claim.evidence_ref.trim();
        let candidate = reference
            .rsplit_once(':')
            .filter(|(_, line)| line.parse::<u32>().is_ok())
            .map_or(reference, |(file, _)| file);
        let Ok(file) = ctx.sandbox.resolve_existing(candidate) else {
            continue;
        };
        let Some(modified) = ctx.executor.mtime(&file).await else {
            return Err(format!("cannot verify cited evidence: {}", file.display()));
        };
        let check = ctx
            .reads
            .lock()
            .map(|reads| reads.check(&file, modified))
            .unwrap_or(ReadCheck::NotRead);
        if check != ReadCheck::Fresh {
            return Err(format!(
                "read cited evidence before accepting the report: {}",
                file.display()
            ));
        }
    }
    Ok(())
}
