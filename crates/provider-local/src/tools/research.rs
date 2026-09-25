//! Stateless prioritization through the shared autoresearch library. Execution,
//! authorization, evidence collection, and persistence remain ordinary agent work.
use std::collections::HashSet;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use clark_autoresearch::{rank_opportunities, ResearchBias, ResearchOpportunity};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ToolCtx, ToolExecutor, ToolOutcome};

const MAX_OPPORTUNITIES: usize = 64;
const MAX_INPUT_BYTES: usize = 128 * 1024;

pub struct ResearchRank;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    opportunities: Vec<Opportunity>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Opportunity {
    node_id: String,
    surface: clark_autoresearch::SurfaceKind,
    evidence_refs: Vec<String>,
    rationale: String,
    in_scope: bool,
    requires_validation: bool,
    priority: f64,
    novelty: f64,
    confidence: f64,
    impact: f64,
}

#[async_trait]
impl ToolExecutor for ResearchRank {
    fn name(&self) -> &str {
        "research_rank"
    }

    fn description(&self) -> &str {
        "Rank up to 64 research hypotheses, targets, or evidence follow-ups with \
         clark-autoresearch. Supply observed evidence references and rationale before \
         estimated scores. Returns scope-filtered exploration/probing/validation hints; \
         scores are caller estimates, not verified findings or authorization. Optional \
         planning aid for security scans and other investigations. No execution, network, \
         model calls, or persisted state; investigate the hints with ordinary tools and \
         rerank after new evidence."
    }

    fn parameters(&self) -> Value {
        let score = json!({"type":"number", "minimum":0, "maximum":1});
        json!({
            "type":"object",
            "properties": {
                "opportunities": {
                    "type":"array", "minItems":1, "maxItems":MAX_OPPORTUNITIES,
                    "items": {
                        "type":"object",
                        "properties": {
                            "node_id":{"type":"string", "minLength":1, "maxLength":160, "description":"Unique local hypothesis or work-item identifier; no domain or server node needed."},
                            "surface":{"type":"string", "enum":["target","endpoint","finding","hypothesis","evidence"]},
                            "evidence_refs":{"type":"array", "maxItems":8, "items":{"type":"string", "minLength":1, "maxLength":512}, "description":"Observed file:line, receipt, or document references. Empty means no observed evidence yet."},
                            "rationale":{"type":"string", "minLength":1, "maxLength":1024, "description":"What is known, uncertain, and worth investigating; explain the score estimates."},
                            "in_scope":{"type":"boolean", "description":"Whether this candidate is within the user's requested scope. False candidates are excluded; true grants no permissions."},
                            "requires_validation":{"type":"boolean"},
                            "priority":score.clone(), "novelty":score.clone(),
                            "confidence":score.clone(), "impact":score
                        },
                        "required":["node_id","surface","evidence_refs","rationale","in_scope","requires_validation","priority","novelty","confidence","impact"],
                        "additionalProperties":false
                    }
                }
            },
            "required":["opportunities"], "additionalProperties":false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Research
    }

    async fn invoke(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        rank(args)
    }
}

fn rank(args: Value) -> ToolOutcome {
    match rank_checked(args) {
        Ok(result) => ToolOutcome::ok(
            "Planning hints only. Evidence and estimates are caller-supplied, not verified. \
             Use ordinary authorized tools to investigate; rerank when evidence changes.",
        )
        .with_model_visible_details(result),
        Err(error) => ToolOutcome::error(error),
    }
}

fn rank_checked(args: Value) -> Result<Value, String> {
    if args.to_string().len() > MAX_INPUT_BYTES {
        return Err("research_rank input exceeds 128 KiB".into());
    }
    let request: Request = serde_json::from_value(args).map_err(|e| e.to_string())?;
    if request.opportunities.is_empty() || request.opportunities.len() > MAX_OPPORTUNITIES {
        return Err("supply 1 to 64 opportunities".into());
    }
    let mut ids = HashSet::new();
    for item in &request.opportunities {
        if !bounded_text(&item.node_id, 160) || !ids.insert(&item.node_id) {
            return Err("node_id must be unique, nonempty, and at most 160 bytes".into());
        }
        if !bounded_text(&item.rationale, 1024)
            || item.evidence_refs.len() > 8
            || item.evidence_refs.iter().any(|r| !bounded_text(r, 512))
        {
            return Err("provide a rationale up to 1024 bytes and at most 8 evidence references up to 512 bytes each".into());
        }
        if [item.priority, item.novelty, item.confidence, item.impact]
            .iter()
            .any(|score| !score.is_finite() || !(0.0..=1.0).contains(score))
        {
            return Err(
                "priority, novelty, confidence, and impact must be finite values from 0 to 1"
                    .into(),
            );
        }
    }
    let opportunities: Vec<_> = request
        .opportunities
        .iter()
        .map(|item| ResearchOpportunity {
            node_id: item.node_id.clone(),
            surface: item.surface,
            priority: item.priority,
            novelty: item.novelty,
            confidence: item.confidence,
            impact: item.impact,
            in_scope: item.in_scope,
            requires_validation: item.requires_validation,
        })
        .collect();
    let ranked = rank_opportunities(&opportunities, &ResearchBias::default());
    let hints: Vec<_> = ranked.into_iter().map(|hint| {
        let source = request.opportunities.iter().find(|item| item.node_id == hint.node_id)
            .expect("library returns only supplied identifiers");
        json!({"hint":hint, "evidence_refs":source.evidence_refs, "caller_rationale":source.rationale})
    }).collect();
    Ok(json!({
        "engine":"clark-autoresearch/0.2.0",
        "excluded_out_of_scope":opportunities.iter().filter(|item| !item.in_scope).count(),
        "ranked":hints
    }))
}

fn bounded_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opportunity(id: &str, validation: bool) -> Value {
        json!({"node_id":id,"surface":"hypothesis","evidence_refs":["src/auth.rs:21"],
            "rationale":"Observed a missing ownership comparison; needs a negative control.",
            "in_scope":true,"requires_validation":validation,
            "priority":0.8,"novelty":0.2,"confidence":0.7,"impact":0.9})
    }

    #[test]
    fn research_rank_prioritizes_validation_and_preserves_evidence() {
        let mut outside = opportunity("outside", true);
        outside["in_scope"] = json!(false);
        let result = rank(
            json!({"opportunities":[opportunity("probe",false),outside,opportunity("prove",true)]}),
        );
        assert!(!result.is_error);
        assert_eq!(result.details["excluded_out_of_scope"], 1);
        assert_eq!(result.details["ranked"].as_array().unwrap().len(), 2);
        assert_eq!(result.details["ranked"][0]["hint"]["node_id"], "prove");
        assert_eq!(result.details["ranked"][0]["hint"]["mode"], "validate");
        assert_eq!(
            result.details["ranked"][0]["evidence_refs"][0],
            "src/auth.rs:21"
        );
        assert!(result.content.contains("src/auth.rs:21"));
        assert!(result.content.contains("caller-supplied"));
    }

    #[test]
    fn research_rank_rejects_ambiguous_or_unbounded_inputs() {
        let candidate = opportunity("one", false);
        for candidates in [
            vec![],
            vec![candidate.clone(); 65],
            vec![candidate.clone(); 2],
        ] {
            assert!(rank(json!({"opportunities":candidates})).is_error);
        }
        for (field, value) in [
            ("priority", json!(1.1)),
            ("node_id", json!(" ")),
            ("rationale", json!("x".repeat(1025))),
            ("in_scope", Value::Null),
            ("evidence_refs", json!(["x".repeat(513)])),
        ] {
            let mut invalid = candidate.clone();
            invalid[field] = value;
            assert!(rank(json!({"opportunities":[invalid]})).is_error, "{field}");
        }
        assert!(rank(json!({"opportunities":[candidate],"require_in_scope":false})).is_error);
    }

    #[tokio::test]
    async fn research_rank_is_discoverable_and_callable_without_product_context() {
        use super::super::{ReadTracker, ToolRegistry};
        use std::sync::{Arc, Mutex};
        let registry = ToolRegistry::new(None);
        let directory = tempfile::tempdir().unwrap();
        let session = Arc::new(tokio::sync::Mutex::new(
            crate::loop_state::SessionState::default(),
        ));
        let ctx = ToolCtx {
            sandbox: Arc::new(crate::sandbox::Sandbox::new(directory.path()).unwrap()),
            executor: Arc::new(crate::exec::LocalExecutor),
            reads: Arc::new(Mutex::new(ReadTracker::default())),
            cancel: tokio_util::sync::CancellationToken::new(),
            background: Arc::new(crate::background::BackgroundTasks::default()),
            session: session.clone(),
            progress: None,
            agent_progress: None,
            call_progress: None,
            model_override: None,
        };
        let search = registry
            .get("tool_search")
            .unwrap()
            .invoke(json!({"query":"research_rank"}), &ctx)
            .await;
        assert!(!search.is_error);
        assert!(session
            .lock()
            .await
            .deferred_tools
            .contains("research_rank"));
        let result = registry
            .get("research_rank")
            .unwrap()
            .invoke(
                json!({"opportunities":[opportunity("local-hypothesis",true)]}),
                &ctx,
            )
            .await;
        assert!(!result.is_error);
        assert_eq!(
            result.details["ranked"][0]["hint"]["node_id"],
            "local-hypothesis"
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn research_rank_registry_is_read_only_and_evidence_precedes_scores() {
        let registry = super::super::ToolRegistry::new(None);
        let tool = registry.get("research_rank").unwrap();
        assert!(!tool.mutating());
        assert_eq!(
            tool.permission_class(),
            super::super::ToolPermissionClass::LocalRead
        );
        let wire = serde_json::to_string(&tool.parameters()).unwrap();
        let schema: Value = serde_json::from_str(&wire).unwrap();
        let keys: Vec<_> = schema["properties"]["opportunities"]["items"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            &keys[..4],
            &["node_id", "surface", "evidence_refs", "rationale"]
        );
        assert_eq!(keys[6], "priority");
    }
}
