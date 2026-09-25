//! Pure research work scheduling: stable semantic keys, shared crosslinks,
//! caller-asserted evidence, bounded attempts, and replayable transitions.
//!
//! This is a work-state graph, not a verification engine or GEPA candidate
//! optimizer. `clark-autoresearch` supplies the ranking policy; the host owns
//! actual tools, authorization, source identity, durable journal I/O, and proof.
mod context;
mod model;
mod transitions;

use std::collections::BTreeMap;

use clark_autoresearch::{rank_opportunities, ResearchBias, ResearchOpportunity};
pub use model::*;

pub const MAX_EVENTS: usize = 4096;
pub const MAX_REFERENCES: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct ResearchTree {
    identity: Identity,
    limits: Limits,
    nodes: BTreeMap<String, Node>,
    events: Vec<Event>,
    steps_used: usize,
}

impl ResearchTree {
    pub fn new(identity: Identity, limits: Limits) -> Result<Self, TreeError> {
        text(&identity.scope_id, 256, "scope_id")?;
        text(&identity.source_id, 512, "source_id")?;
        if !(1..=256).contains(&limits.max_nodes) || !(1..=1000).contains(&limits.max_steps) {
            return Err(TreeError::Invalid(
                "max_nodes must be 1..256 and max_steps 1..1000".into(),
            ));
        }
        Ok(Self {
            identity,
            limits,
            nodes: BTreeMap::new(),
            events: Vec::new(),
            steps_used: 0,
        })
    }

    pub fn replay(
        identity: Identity,
        limits: Limits,
        events: impl IntoIterator<Item = Event>,
    ) -> Result<Self, TreeError> {
        let mut tree = Self::new(identity, limits)?;
        for event in events {
            tree.apply(event)?;
        }
        Ok(tree)
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
    pub fn nodes(&self) -> &BTreeMap<String, Node> {
        &self.nodes
    }
    pub fn events(&self) -> &[Event] {
        &self.events
    }
    pub fn steps_used(&self) -> usize {
        self.steps_used
    }
    pub fn journal(&self) -> Journal {
        Journal {
            identity: self.identity.clone(),
            limits: self.limits,
            events: self.events.clone(),
        }
    }

    /// Validate on a copy so every failure leaves the entire tree and journal unchanged.
    pub fn apply(&mut self, event: Event) -> Result<EventResult, TreeError> {
        if self.events.len() >= MAX_EVENTS {
            return Err(TreeError::Budget("journal event limit".into()));
        }
        // Stage only the bounded derived state, not the ever-growing journal.
        let mut next = Self {
            identity: self.identity.clone(),
            limits: self.limits,
            nodes: self.nodes.clone(),
            events: Vec::new(),
            steps_used: self.steps_used,
        };
        let result = next.transition(&event)?;
        // Every active attempt retains a slot for its terminal observation.
        // Without this reservation the final Started event could strand work.
        if self.events.len() + 1 + next.active_count() > MAX_EVENTS {
            return Err(TreeError::Budget(
                "journal slots reserved for active outcomes".into(),
            ));
        }
        self.nodes = next.nodes;
        self.steps_used = next.steps_used;
        self.events.push(event);
        Ok(result)
    }

    /// Only actionable pending nodes appear. Exhausted attempt/event budgets
    /// expose no runnable work, even when pending nodes remain in the graph.
    pub fn ranked(&self) -> Vec<DispatchHint> {
        if self.steps_used >= self.limits.max_steps
            || self.events.len() + self.active_count() + 2 > MAX_EVENTS
        {
            return Vec::new();
        }
        let opportunities: Vec<_> = self
            .nodes
            .values()
            .filter(|node| self.ready(node))
            .map(|node| {
                let a = &node.assessment;
                ResearchOpportunity {
                    node_id: node.key.clone(),
                    surface: a.surface,
                    priority: a.priority,
                    novelty: a.novelty,
                    confidence: a.confidence,
                    impact: a.impact,
                    in_scope: a.in_scope,
                    requires_validation: a.requires_validation,
                }
            })
            .collect();
        rank_opportunities(&opportunities, &ResearchBias::default())
    }

    /// Deterministically choose and activate one node, charging one attempt.
    /// Replaying its Started event enforces the identical choice.
    pub fn select(&mut self) -> Result<Option<Node>, TreeError> {
        if self.steps_used >= self.limits.max_steps {
            return Err(TreeError::Budget("step limit".into()));
        }
        if self.events.len() + self.active_count() + 2 > MAX_EVENTS {
            return Err(TreeError::Budget(
                "journal slots reserved for active outcomes".into(),
            ));
        }
        let Some(hint) = self.ranked().into_iter().next() else {
            return Ok(None);
        };
        self.apply(Event::Started {
            key: hint.node_id.clone(),
        })?;
        Ok(self.nodes.get(&hint.node_id).cloned())
    }

    fn active_count(&self) -> usize {
        self.nodes
            .values()
            .filter(|node| node.status == Status::Active)
            .count()
    }

    fn ready(&self, node: &Node) -> bool {
        node.status == Status::Pending
            && node.assessment.in_scope
            && node.dependencies.iter().all(|key| {
                self.nodes.get(key).is_some_and(|dependency| {
                    matches!(dependency.status, Status::Completed | Status::Refuted)
                })
            })
    }
}

fn text(value: &str, max: usize, field: &str) -> Result<(), TreeError> {
    if value.trim().is_empty() || value.len() > max {
        Err(TreeError::Invalid(format!(
            "{field} must be nonempty and at most {max} bytes"
        )))
    } else {
        Ok(())
    }
}

fn refs(values: &[String]) -> Result<(), TreeError> {
    if values.len() > MAX_REFERENCES {
        return Err(TreeError::Invalid("too many references".into()));
    }
    for value in values {
        text(value, 512, "reference")?;
    }
    Ok(())
}

fn merge(values: &mut Vec<String>, incoming: &[String]) -> Result<(), TreeError> {
    for value in incoming {
        if !values.contains(value) {
            values.push(value.clone());
        }
    }
    refs(values)
}

#[cfg(test)]
mod tests;
