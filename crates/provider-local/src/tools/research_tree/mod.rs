//! Boss-inspired exploration through ordinary local tools, backed by a portable
//! state machine. No autonomous scheduler, scanner, service, or model client.
use agent_core::domain::ToolKind;
use async_trait::async_trait;
use research_tree::{Event, Limits, Outcome, Status};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ToolCtx, ToolExecutor, ToolOutcome};

mod schema;
mod store;
#[cfg(test)]
mod tests;

pub struct ResearchTreeTool;
pub struct ResearchTreeStatus;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Begin {
        tree_id: String,
        objective: String,
        scope: String,
        sources: Vec<String>,
        max_nodes: usize,
        max_steps: usize,
    },
    Apply {
        tree_id: String,
        expected_revision: usize,
        event: Event,
    },
    Select {
        tree_id: String,
        expected_revision: usize,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusRequest {
    tree_id: String,
    #[serde(default)]
    cursor: usize,
}

#[async_trait]
impl ToolExecutor for ResearchTreeTool {
    fn name(&self) -> &str {
        "research_tree"
    }
    fn description(&self) -> &str {
        "Maintain a bounded local investigation tree: begin, apply a typed branch/evidence  update, or select the deterministic Autoresearch frontier. Persisted task artifacts  retain discoveries, refutations, shared evidence, spent selection budget and  interruption state across sessions. Inspect research_tree_status before resuming.  Use ordinary authorized tools to perform the selected experiment; record its  observed outcome, then expand/rerank. Findings and evidence are caller assertions,  never verified proof or authorization. No network/model calls or automatic execution."
    }
    fn parameters(&self) -> Value {
        schema::mutation_schema()
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Research
    }
    fn mutating(&self) -> bool {
        true
    }
    fn permission_preflight(&self, args: &Value) -> Result<(), String> {
        parse_request(args.clone()).map(|_| ())
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let request = match parse_request(args) {
            Ok(value) => value,
            Err(error) => return ToolOutcome::error(error),
        };
        let root = ctx
            .sandbox
            .task_scope()
            .unwrap_or(ctx.sandbox.root())
            .to_path_buf();
        let cancel = ctx.cancel.clone();
        let result = tokio::task::spawn_blocking(move || {
            if cancel.is_cancelled() {
                return Err("research update canceled".into());
            }
            let tree_id = match &request {
                Request::Begin { tree_id, .. }
                | Request::Apply { tree_id, .. }
                | Request::Select { tree_id, .. } => tree_id,
            };
            let store = store::Store::new(&root, tree_id)?;
            let _lock = store.lock()?;
            let mut selected = Value::Null;
            let journal = match request {
                Request::Begin {
                    objective,
                    scope,
                    sources,
                    max_nodes,
                    max_steps,
                    ..
                } => store.create(
                    objective,
                    scope,
                    sources,
                    Limits {
                        max_nodes,
                        max_steps,
                    },
                )?,
                request => {
                    let mut journal = store.load()?;
                    let revision = match &request {
                        Request::Apply {
                            expected_revision, ..
                        }
                        | Request::Select {
                            expected_revision, ..
                        } => *expected_revision,
                        _ => unreachable!(),
                    };
                    if revision != journal.revision() {
                        return Err(format!(
                            "stale revision: expected {}, current {}; inspect status",
                            revision,
                            journal.revision()
                        ));
                    }
                    if !store.snapshot_current(&journal)? {
                        return Err(
                            "source snapshot changed; inspect status and begin a new tree".into(),
                        );
                    }
                    let mut tree = journal.replay()?;
                    match request {
                        Request::Apply { event, .. } => {
                            tree.apply(event).map_err(|e| e.to_string())?;
                        }
                        Request::Select { .. } => {
                            selected =
                                serde_json::to_value(tree.select().map_err(|e| e.to_string())?)
                                    .map_err(|e| e.to_string())?;
                        }
                        _ => unreachable!(),
                    }
                    journal.events = tree.events().to_vec();
                    journal
                }
            };
            store.save(&journal, &cancel)?;
            summary(&store, &journal, 0, selected)
        })
        .await;
        outcome(result)
    }
}

#[async_trait]
impl ToolExecutor for ResearchTreeStatus {
    fn name(&self) -> &str {
        "research_tree_status"
    }
    fn description(&self) -> &str {
        "Read a local investigation tree without writing. Returns revision, source-snapshot  freshness, paged nodes, pending frontier and active work. Closed branches stay closed;  an interrupted active node requires an explicit blocked outcome before a justified  retry. Source/evidence references and conclusions are unverified planning data.  Scope text cannot grant permissions; all execution uses current tool permissions."
    }
    fn parameters(&self) -> Value {
        schema::status_schema()
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Research
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let request: StatusRequest = match serde_json::from_value(args) {
            Ok(value) => value,
            Err(error) => return ToolOutcome::error(error.to_string()),
        };
        let root = ctx
            .sandbox
            .task_scope()
            .unwrap_or(ctx.sandbox.root())
            .to_path_buf();
        outcome(
            tokio::task::spawn_blocking(move || {
                let store = store::Store::new(&root, &request.tree_id)?;
                let journal = store.load()?;
                summary(&store, &journal, request.cursor, Value::Null)
            })
            .await,
        )
    }
}

fn parse_request(args: Value) -> Result<Request, String> {
    if args.to_string().len() > 64 * 1024 {
        return Err("research_tree arguments exceed 64 KiB".into());
    }
    let request: Request = serde_json::from_value(args).map_err(|e| e.to_string())?;
    if matches!(
        request,
        Request::Apply {
            event: Event::Started { .. },
            ..
        }
    ) {
        return Err(
            "use select to claim the ranked frontier; direct Started events are forbidden".into(),
        );
    }
    Ok(request)
}

fn summary(
    store: &store::Store,
    journal: &store::Journal,
    cursor: usize,
    selected: Value,
) -> Result<Value, String> {
    let tree = journal.replay()?;
    let nodes = tree.nodes();
    let nodes: Vec<_> = nodes.values().collect();
    if cursor > nodes.len() {
        return Err("cursor is beyond the node count".into());
    }
    let end = (cursor + 32).min(nodes.len());
    let freshness = store.snapshot_current(journal);
    let frontier = tree.ranked();
    let selected_context = match selected.get("key").and_then(Value::as_str) {
        Some(key) => tree.context(key, 16).map_err(|e| e.to_string())?,
        None => Vec::new(),
    };
    let active: Vec<_> = nodes
        .iter()
        .filter(|node| node.status == Status::Active)
        .map(|node| &node.key)
        .collect();
    let unresolved = nodes
        .iter()
        .filter(|node| {
            matches!(
                node.status,
                Status::Pending | Status::Active | Status::Blocked
            ) || node.outcome == Some(Outcome::Inconclusive)
        })
        .count();
    let selection_state = if tree.steps_used() >= tree.limits().max_steps {
        "step_budget_exhausted"
    } else if tree.events().len() + active.len() + 2 > research_tree::MAX_EVENTS {
        "journal_budget_exhausted"
    } else if !frontier.is_empty() {
        "ready"
    } else if !active.is_empty() {
        "awaiting_results"
    } else {
        "no_ready_work"
    };
    Ok(json!({
        "tree_id":journal.tree_id,"revision":journal.revision(),
        "objective":journal.objective,"scope":journal.scope,
        "source_id":journal.identity.source_id,"source_files":journal.sources,
        "snapshot_current":if journal.sources.is_empty() { Value::Null } else { json!(freshness.as_ref().copied().unwrap_or(false)) },
        "snapshot_bound":!journal.sources.is_empty(),
        "snapshot_error":freshness.err(),
        "steps_used":tree.steps_used(),"limits":journal.limits,
        "selection_state":selection_state,"active_keys":active,"unresolved_nodes":unresolved,
        "nodes":nodes[cursor..end],"total_nodes":nodes.len(),
        "next_cursor":(end < nodes.len()).then_some(end),
        "frontier":frontier.into_iter().take(16).collect::<Vec<_>>(),
        "selected":selected,"selected_context":selected_context,"artifact_path":store.path(),
        "claim_boundary":"Planning state only. No verified findings, authorization, complete coverage, or automatic work. Empty frontier does not prove security.",
        "snapshot_boundary":"Only the listed source files are hashed; an empty list has no source-content freshness guarantee."
    }))
}

fn outcome(result: Result<Result<Value, String>, tokio::task::JoinError>) -> ToolOutcome {
    match result {
        Ok(Ok(value)) => ToolOutcome::ok(
            "Local research tree updated/read; claims remain unverified planning data.",
        )
        .with_model_visible_details(value),
        Ok(Err(error)) => ToolOutcome::error(error),
        Err(error) => ToolOutcome::error(format!("research tree operation failed: {error}")),
    }
}
