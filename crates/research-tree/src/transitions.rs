use super::*;

impl ResearchTree {
    pub(crate) fn transition(&mut self, event: &Event) -> Result<EventResult, TreeError> {
        match event {
            Event::Proposed {
                key,
                parent,
                links,
                dependencies,
                evidence_refs,
                rationale,
                assessment,
            } => {
                text(key, 160, "key")?;
                text(rationale, 4096, "rationale")?;
                refs(links)?;
                refs(dependencies)?;
                refs(evidence_refs)?;
                validate_assessment(assessment)?;
                for reference in parent.iter().chain(links).chain(dependencies) {
                    if reference == key {
                        return Err(TreeError::Invalid("self reference".into()));
                    }
                    if !self.nodes.contains_key(reference) {
                        return Err(TreeError::UnknownNode(reference.clone()));
                    }
                }
                let normalized_dependencies: Vec<_> = dependencies
                    .iter()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                if let Some(existing) = self.nodes.get_mut(key) {
                    // A semantic key denotes the same work. Deduplication may add
                    // provenance, never reset status or silently rewrite its contract.
                    if existing.assessment != *assessment
                        || existing.dependencies != normalized_dependencies
                    {
                        return Err(TreeError::Invalid(
                            "duplicate key changes assessment or dependencies".into(),
                        ));
                    }
                    merge(&mut existing.links, links)?;
                    if let Some(parent) = parent {
                        if existing.parent.as_ref() != Some(parent) {
                            merge(&mut existing.links, std::slice::from_ref(parent))?;
                        }
                    }
                    merge(&mut existing.evidence_refs, evidence_refs)?;
                    return Ok(EventResult::Merged(key.clone()));
                }
                if self.nodes.len() >= self.limits.max_nodes {
                    return Err(TreeError::Budget("node limit".into()));
                }
                let mut unique_links = Vec::new();
                let mut unique_evidence = Vec::new();
                merge(&mut unique_links, links)?;
                merge(&mut unique_evidence, evidence_refs)?;
                self.nodes.insert(
                    key.clone(),
                    Node {
                        key: key.clone(),
                        parent: parent.clone(),
                        links: unique_links,
                        dependencies: normalized_dependencies,
                        evidence_refs: unique_evidence,
                        rationale: rationale.clone(),
                        assessment: assessment.clone(),
                        status: Status::Pending,
                        outcome: None,
                    },
                );
                Ok(EventResult::Created(key.clone()))
            }
            Event::Reassessed {
                key,
                evidence_refs,
                rationale,
                assessment,
            } => {
                text(rationale, 4096, "rationale")?;
                refs(evidence_refs)?;
                validate_assessment(assessment)?;
                let node = self
                    .nodes
                    .get_mut(key)
                    .ok_or_else(|| TreeError::UnknownNode(key.clone()))?;
                if node.status != Status::Pending {
                    return Err(TreeError::Transition(
                        "reassess requires pending work".into(),
                    ));
                }
                if node.assessment.in_scope != assessment.in_scope {
                    return Err(TreeError::Invalid(
                        "reassessment cannot change declared scope".into(),
                    ));
                }
                if !evidence_refs
                    .iter()
                    .any(|reference| !node.evidence_refs.contains(reference))
                {
                    return Err(TreeError::Invalid(
                        "reassess requires a genuinely new evidence reference".into(),
                    ));
                }
                merge(&mut node.evidence_refs, evidence_refs)?;
                node.rationale = rationale.clone();
                node.assessment = assessment.clone();
                Ok(EventResult::Updated(key.clone()))
            }
            Event::Started { key } => {
                if self.steps_used >= self.limits.max_steps {
                    return Err(TreeError::Budget("step limit".into()));
                }
                let node = self
                    .nodes
                    .get(key)
                    .ok_or_else(|| TreeError::UnknownNode(key.clone()))?;
                if !self.ready(node) {
                    return Err(TreeError::Transition(
                        "node is not pending, in scope, and dependency-ready".into(),
                    ));
                }
                if self.ranked().first().map(|hint| &hint.node_id) != Some(key) {
                    return Err(TreeError::Transition(
                        "Started must select the highest ranked ready node".into(),
                    ));
                }
                self.nodes.get_mut(key).expect("node checked").status = Status::Active;
                self.steps_used += 1;
                Ok(EventResult::Updated(key.clone()))
            }
            Event::Recorded {
                key,
                outcome,
                evidence_refs,
                rationale,
            } => {
                text(rationale, 4096, "rationale")?;
                refs(evidence_refs)?;
                if *outcome == Outcome::Supports && evidence_refs.is_empty() {
                    return Err(TreeError::Invalid(
                        "supports requires asserted evidence references".into(),
                    ));
                }
                let node = self
                    .nodes
                    .get_mut(key)
                    .ok_or_else(|| TreeError::UnknownNode(key.clone()))?;
                if node.status != Status::Active {
                    return Err(TreeError::Transition(
                        "record requires an active node".into(),
                    ));
                }
                merge(&mut node.evidence_refs, evidence_refs)?;
                node.rationale = rationale.clone();
                node.outcome = Some(*outcome);
                node.status = match outcome {
                    Outcome::Refutes => Status::Refuted,
                    Outcome::Blocked => Status::Blocked,
                    Outcome::Supports | Outcome::Expands | Outcome::Inconclusive => {
                        Status::Completed
                    }
                };
                Ok(EventResult::Updated(key.clone()))
            }
            Event::Reopened {
                key,
                evidence_refs,
                rationale,
            } => {
                text(rationale, 4096, "rationale")?;
                refs(evidence_refs)?;
                let node = self
                    .nodes
                    .get_mut(key)
                    .ok_or_else(|| TreeError::UnknownNode(key.clone()))?;
                if matches!(node.status, Status::Pending | Status::Active) {
                    return Err(TreeError::Transition(
                        "only a terminal node may reopen".into(),
                    ));
                }
                if !evidence_refs
                    .iter()
                    .any(|reference| !node.evidence_refs.contains(reference))
                {
                    return Err(TreeError::Invalid(
                        "reopen requires a genuinely new evidence reference".into(),
                    ));
                }
                merge(&mut node.evidence_refs, evidence_refs)?;
                node.rationale = rationale.clone();
                node.outcome = None;
                node.status = Status::Pending;
                Ok(EventResult::Updated(key.clone()))
            }
            Event::Pruned { key, rationale } => {
                text(rationale, 4096, "rationale")?;
                let node = self
                    .nodes
                    .get_mut(key)
                    .ok_or_else(|| TreeError::UnknownNode(key.clone()))?;
                if !matches!(node.status, Status::Pending | Status::Blocked) {
                    return Err(TreeError::Transition(
                        "prune requires pending or blocked work".into(),
                    ));
                }
                node.status = Status::Pruned;
                node.rationale = rationale.clone();
                Ok(EventResult::Updated(key.clone()))
            }
        }
    }
}

fn validate_assessment(assessment: &Assessment) -> Result<(), TreeError> {
    for score in [
        assessment.priority,
        assessment.novelty,
        assessment.confidence,
        assessment.impact,
    ] {
        if !score.is_finite() || !(0.0..=1.0).contains(&score) {
            return Err(TreeError::Invalid(
                "assessment scores must be finite and within 0..1".into(),
            ));
        }
    }
    Ok(())
}
