use crate::model::{
    ContextReceipt, EvidenceSource, KnowledgeDelivery, Lane, RetrievalTreatmentReceipt,
    TrajectoryReceipt,
};
use agent_core::domain::{AgentEvent, ToolStatus};

pub fn retrieval_treatment(
    lane: &Lane,
    context: &ContextReceipt,
    trajectory: &TrajectoryReceipt,
) -> RetrievalTreatmentReceipt {
    retrieval_treatment_for_sources(
        &lane.planner_sources,
        lane.knowledge_delivery(),
        context,
        trajectory,
    )
}

pub fn retrieval_treatment_for_sources(
    sources: &[EvidenceSource],
    knowledge_delivery: KnowledgeDelivery,
    context: &ContextReceipt,
    trajectory: &TrajectoryReceipt,
) -> RetrievalTreatmentReceipt {
    let offered_sources = sources
        .iter()
        .copied()
        .filter(|source| {
            matches!(
                source,
                EvidenceSource::Project | EvidenceSource::Org | EvidenceSource::Scout
            )
        })
        .collect::<Vec<_>>();
    let completed_tool_calls = trajectory
        .events
        .iter()
        .filter_map(|receipt| match &receipt.event {
            AgentEvent::ToolCallUpdate { id, patch, .. }
                if patch.status == Some(ToolStatus::Completed) =>
            {
                Some(id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let project_recalled = trajectory.events.iter().any(|receipt| {
        matches!(
            &receipt.event,
            AgentEvent::ToolCall { call, .. }
                if call.tool_name.as_deref() == Some("memory")
                    && call.raw_input.as_ref().is_some_and(|input| {
                        input["action"].as_str() == Some("recall")
                    })
                    && completed_tool_calls.contains(&&call.id)
        ) || matches!(
            &receipt.event,
            AgentEvent::ToolCall { call, .. }
                if call.tool_name.as_deref() == Some("read_file")
                    && call.raw_input.as_ref().is_some_and(|input| {
                        input["path"].as_str().is_some_and(|path| {
                            path.starts_with(".clark/memory/project-")
                                && path.ends_with(".md")
                        })
                    })
                    && completed_tool_calls.contains(&&call.id)
        )
    });
    let org_recalled = context.retrievals.iter().any(|receipt| {
        receipt.source == "org"
            && receipt.status == "ok"
            && !receipt.returned_evidence_ids.is_empty()
    });
    let scout_recalled = context.retrievals.iter().any(|receipt| {
        receipt.source == "scout"
            && receipt.operation == "snapshots.query"
            && receipt.status == "ok"
            && !receipt.returned_evidence_ids.is_empty()
    });
    let prefetched = |source: EvidenceSource| {
        let prefix = match source {
            EvidenceSource::Project => "PROJECT-",
            EvidenceSource::Org => "ORG-",
            EvidenceSource::Scout => "SCOUT-",
            _ => return false,
        };
        context
            .injected_evidence_ids
            .iter()
            .any(|id| id.starts_with(prefix))
    };
    let successful_sources = offered_sources
        .iter()
        .copied()
        .filter(|source| match knowledge_delivery {
            KnowledgeDelivery::PrefetchedCapsule => prefetched(*source),
            _ => match source {
                EvidenceSource::Project => project_recalled,
                EvidenceSource::Org => org_recalled,
                EvidenceSource::Scout => scout_recalled,
                _ => false,
            },
        })
        .collect::<Vec<_>>();
    let missing_sources = offered_sources
        .iter()
        .copied()
        .filter(|source| !successful_sources.contains(source))
        .collect::<Vec<_>>();
    let applicable = !offered_sources.is_empty();
    RetrievalTreatmentReceipt {
        applicable,
        knowledge_delivery,
        compliant: applicable && missing_sources.is_empty(),
        offered_sources,
        successful_sources,
        missing_sources,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::sha256;
    use serde_json::json;

    #[test]
    fn prefetched_capsule_requires_exact_source_ids() {
        let context = ContextReceipt {
            assigned_evidence_ids: vec![
                "PROJECT-ONE".into(),
                "ORG-TWO".into(),
                "SCOUT-THREE".into(),
            ],
            injected_evidence_ids: vec![
                "PROJECT-ONE".into(),
                "ORG-TWO".into(),
                "SCOUT-THREE".into(),
            ],
            injected_context: "capsule".into(),
            context_sha256: sha256("capsule"),
            retrievals: Vec::new(),
        };
        let receipt = retrieval_treatment_for_sources(
            &[
                EvidenceSource::Project,
                EvidenceSource::Org,
                EvidenceSource::Scout,
            ],
            KnowledgeDelivery::PrefetchedCapsule,
            &context,
            &TrajectoryReceipt::default(),
        );
        assert!(receipt.compliant);
        assert_eq!(receipt.successful_sources.len(), 3);
    }

    #[test]
    fn completed_direct_project_memory_read_counts_as_retrieval() {
        let context = ContextReceipt {
            assigned_evidence_ids: vec!["PROJECT-AUDIT-01".into()],
            injected_evidence_ids: Vec::new(),
            injected_context: String::new(),
            context_sha256: sha256(""),
            retrievals: Vec::new(),
        };
        let trajectory = serde_json::from_value(json!({
            "events": [
                {
                    "stream_sequence": 1,
                    "elapsed_ms": 1,
                    "event": {
                        "event": "tool_call",
                        "run": "run-1",
                        "call": {
                            "id": "read-project",
                            "tool_name": "read_file",
                            "title": "Read project memory",
                            "kind": "read",
                            "status": "pending",
                            "raw_input": {
                                "path": ".clark/memory/project-audit-01.md"
                            }
                        }
                    }
                },
                {
                    "stream_sequence": 2,
                    "elapsed_ms": 2,
                    "event": {
                        "event": "tool_call_update",
                        "run": "run-1",
                        "id": "read-project",
                        "patch": {
                            "status": "completed"
                        }
                    }
                }
            ]
        }))
        .expect("trajectory fixture should deserialize");

        let receipt = retrieval_treatment_for_sources(
            &[EvidenceSource::Project],
            KnowledgeDelivery::DeferredDiscovery,
            &context,
            &trajectory,
        );

        assert!(receipt.compliant);
        assert_eq!(receipt.successful_sources, vec![EvidenceSource::Project]);
    }

    #[test]
    fn failed_direct_project_memory_read_does_not_count_as_retrieval() {
        let context = ContextReceipt {
            assigned_evidence_ids: vec!["PROJECT-AUDIT-01".into()],
            injected_evidence_ids: Vec::new(),
            injected_context: String::new(),
            context_sha256: sha256(""),
            retrievals: Vec::new(),
        };
        let trajectory = serde_json::from_value(json!({
            "events": [
                {
                    "stream_sequence": 1,
                    "elapsed_ms": 1,
                    "event": {
                        "event": "tool_call",
                        "run": "run-1",
                        "call": {
                            "id": "read-project",
                            "tool_name": "read_file",
                            "title": "Read project memory",
                            "kind": "read",
                            "status": "pending",
                            "raw_input": {
                                "path": ".clark/memory/project-audit-01.md"
                            }
                        }
                    }
                },
                {
                    "stream_sequence": 2,
                    "elapsed_ms": 2,
                    "event": {
                        "event": "tool_call_update",
                        "run": "run-1",
                        "id": "read-project",
                        "patch": {
                            "status": "failed"
                        }
                    }
                }
            ]
        }))
        .expect("trajectory fixture should deserialize");

        let receipt = retrieval_treatment_for_sources(
            &[EvidenceSource::Project],
            KnowledgeDelivery::DeferredDiscovery,
            &context,
            &trajectory,
        );

        assert!(!receipt.compliant);
        assert!(receipt.successful_sources.is_empty());
    }
}
