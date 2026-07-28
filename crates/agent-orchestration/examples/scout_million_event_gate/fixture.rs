use std::collections::BTreeSet;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseEdgeKind, EnterpriseEntityId, EnterpriseEntityKind,
    EnterpriseEvent, EnterpriseFact, EnterpriseId, EnterpriseProvenance, GraphEdgeObservation,
    GraphEntityObservation,
};
use sha2::{Digest, Sha256};

pub const EVENTS_PER_BATCH: usize = 10_000;

pub struct DeterministicFixture {
    enterprise_id: EnterpriseId,
    event_count: usize,
    service_count: usize,
}

impl DeterministicFixture {
    pub fn new(event_count: usize, service_count: usize) -> Result<Self, String> {
        let structural_events = service_count
            .checked_mul(3)
            .ok_or_else(|| "service count overflow".to_string())?;
        if event_count < structural_events {
            return Err(format!(
                "event count must be at least three times services ({structural_events})"
            ));
        }
        Ok(Self {
            enterprise_id: EnterpriseId::new("million-event-benchmark")?,
            event_count,
            service_count,
        })
    }

    pub fn enterprise_id(&self) -> &EnterpriseId {
        &self.enterprise_id
    }

    pub fn event_count(&self) -> usize {
        self.event_count
    }

    pub fn service_count(&self) -> usize {
        self.service_count
    }

    pub fn batch_count(&self) -> usize {
        self.event_count.div_ceil(EVENTS_PER_BATCH)
    }

    pub fn batch(&self, batch_index: usize) -> Result<EnterpriseBatch, String> {
        if batch_index >= self.batch_count() {
            return Err(format!("batch index {batch_index} is out of range"));
        }
        let first = batch_index * EVENTS_PER_BATCH;
        let end = (first + EVENTS_PER_BATCH).min(self.event_count);
        let mut events = Vec::with_capacity(end - first);
        for event_index in first..end {
            events.push(self.event(event_index)?);
        }
        EnterpriseBatch::new(self.enterprise_id.clone(), events)
    }

    pub fn target_service_index(&self) -> usize {
        self.service_count / 2
    }

    pub fn target_service_id(&self) -> Result<EnterpriseEntityId, String> {
        Ok(self
            .entity_observation(self.target_service_index(), EnterpriseEntityKind::Service)?
            .entity_id)
    }

    pub fn target_label(&self) -> String {
        service_label(self.target_service_index())
    }

    pub fn expected_target_supporting_events(&self) -> usize {
        let repeated = self.event_count - (self.service_count * 3);
        let target = self.target_service_index();
        let repeated_for_target = if repeated <= target {
            0
        } else {
            1 + (repeated - 1 - target) / self.service_count
        };
        1 + repeated_for_target
    }

    fn event(&self, event_index: usize) -> Result<EnterpriseEvent, String> {
        let sequence = u64::try_from(event_index + 1)
            .map_err(|_| "event sequence does not fit in u64".to_string())?;
        let fact = if event_index < self.service_count {
            EnterpriseFact::EntityObserved(
                self.entity_observation(event_index, EnterpriseEntityKind::Service)?,
            )
        } else if event_index < self.service_count * 2 {
            EnterpriseFact::EntityObserved(self.entity_observation(
                event_index - self.service_count,
                EnterpriseEntityKind::Repository,
            )?)
        } else if event_index < self.service_count * 3 {
            EnterpriseFact::EdgeObserved(
                self.source_edge_observation(event_index - self.service_count * 2)?,
            )
        } else {
            EnterpriseFact::EntityObserved(self.entity_observation(
                (event_index - self.service_count * 3) % self.service_count,
                EnterpriseEntityKind::Service,
            )?)
        };
        EnterpriseEvent::new(self.enterprise_id.clone(), provenance(sequence), fact)
    }

    fn entity_observation(
        &self,
        service_index: usize,
        kind: EnterpriseEntityKind,
    ) -> Result<GraphEntityObservation, String> {
        let (native_prefix, label_prefix) = match kind {
            EnterpriseEntityKind::Service => ("service", "service"),
            EnterpriseEntityKind::Repository => ("repository", "repository"),
            _ => return Err("million-event fixture only creates services and repositories".into()),
        };
        let mut observation = GraphEntityObservation::new(
            &self.enterprise_id,
            kind,
            AuthorityRef::new(
                "benchmark",
                "tenant-scale",
                format!("{native_prefix}:{service_index:08}"),
            )?,
            BTreeSet::from([format!("{label_prefix}-{service_index:08}")]),
            evidence(service_index, native_prefix),
        )?;
        observation.provider_resource_type = Some(format!("benchmark.{native_prefix}"));
        observation.environments = BTreeSet::from(["production".into()]);
        observation.critical = kind == EnterpriseEntityKind::Service && service_index % 100 == 0;
        Ok(observation)
    }

    fn source_edge_observation(
        &self,
        service_index: usize,
    ) -> Result<GraphEdgeObservation, String> {
        let repository = self
            .entity_observation(service_index, EnterpriseEntityKind::Repository)?
            .entity_id;
        let service = self
            .entity_observation(service_index, EnterpriseEntityKind::Service)?
            .entity_id;
        GraphEdgeObservation::new(
            &self.enterprise_id,
            repository,
            service,
            EnterpriseEdgeKind::SourceFor,
            None,
            evidence(service_index, "source-edge"),
        )
    }
}

fn service_label(service_index: usize) -> String {
    format!("service-{service_index:08}")
}

fn provenance(sequence: u64) -> EnterpriseProvenance {
    EnterpriseProvenance {
        machine_id: "benchmark-machine".into(),
        run_id: "million-event-run".into(),
        adapter_instance_id: "benchmark-adapter".into(),
        auth_context_id: "benchmark-read-only".into(),
        discovery_epoch: "epoch-1".into(),
        discovery_epoch_sequence: 1,
        source_sequence: sequence,
        observed_at_ms: 1_700_000_000_000 + sequence,
        source_fingerprint: "5".repeat(64),
    }
}

fn evidence(service_index: usize, salt: &str) -> BTreeSet<String> {
    BTreeSet::from([format!(
        "{:x}",
        Sha256::digest(format!("scout-million-event/v1/{salt}/{service_index}").as_bytes())
    )])
}
