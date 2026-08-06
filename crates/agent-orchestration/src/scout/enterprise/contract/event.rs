use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::charter::{DiscoveryCharterObservation, DiscoveryPassSealObservation};
use super::discovery::{CoverageObservation, FrontierObservation};
use super::ids::{
    canonical_digest, validate_evidence, validate_text, EnterpriseBatchId, EnterpriseEntityId,
    EnterpriseEventId, EnterpriseId,
};
use super::topology::{EnterpriseProvenance, GraphEdgeObservation, GraphEntityObservation};

pub const ENTERPRISE_SCHEMA_VERSION: u16 = 2;
pub const MAX_ENTERPRISE_EVENTS_PER_BATCH: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SimulationContractObservation {
    pub runtime_id: EnterpriseEntityId,
    pub inputs: bool,
    pub outputs: bool,
    pub state_effects: bool,
    pub timeouts: bool,
    pub retries: bool,
    pub idempotency: bool,
    pub failure_behavior: bool,
    pub observability: bool,
    pub recovery: bool,
    pub evidence_digests: BTreeSet<String>,
}

impl SimulationContractObservation {
    pub fn is_complete(&self) -> bool {
        self.inputs
            && self.outputs
            && self.state_effects
            && self.timeouts
            && self.retries
            && self.idempotency
            && self.failure_behavior
            && self.observability
            && self.recovery
    }

    fn validate(&self) -> Result<(), String> {
        validate_evidence(&self.evidence_digests)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fact", rename_all = "snake_case")]
pub enum EnterpriseFact {
    DiscoveryCharterObserved(DiscoveryCharterObservation),
    DiscoveryPassSealed(DiscoveryPassSealObservation),
    EntityObserved(GraphEntityObservation),
    EdgeObserved(GraphEdgeObservation),
    CoverageObserved(CoverageObservation),
    FrontierObserved(FrontierObservation),
    SimulationContractObserved(SimulationContractObservation),
    ObservationRetracted {
        target_event_id: EnterpriseEventId,
        reason: String,
        evidence_digests: BTreeSet<String>,
    },
}

impl EnterpriseFact {
    fn validate(&self, enterprise_id: &EnterpriseId) -> Result<(), String> {
        match self {
            Self::DiscoveryCharterObserved(value) => value.validate(),
            Self::DiscoveryPassSealed(value) => value.validate(),
            Self::EntityObserved(value) => value.validate(enterprise_id),
            Self::EdgeObserved(value) => value.validate(enterprise_id),
            Self::CoverageObserved(value) => value.validate(enterprise_id),
            Self::FrontierObserved(value) => value.validate(enterprise_id),
            Self::SimulationContractObserved(value) => value.validate(),
            Self::ObservationRetracted {
                target_event_id: _,
                reason,
                evidence_digests,
            } => {
                validate_text("retraction reason", reason, 2_048)?;
                validate_evidence(evidence_digests)
            }
        }
    }

    fn validate_provenance(&self, provenance: &EnterpriseProvenance) -> Result<(), String> {
        let Self::DiscoveryPassSealed(value) = self else {
            return Ok(());
        };
        if value.discovery_epoch != provenance.discovery_epoch
            || value.discovery_epoch_sequence != provenance.discovery_epoch_sequence
        {
            return Err("discovery pass seal epoch does not match event provenance".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseEvent {
    pub schema_version: u16,
    pub event_id: EnterpriseEventId,
    pub enterprise_id: EnterpriseId,
    pub provenance: EnterpriseProvenance,
    pub fact: EnterpriseFact,
}

#[derive(Serialize)]
struct EventContent<'a> {
    schema_version: u16,
    enterprise_id: &'a EnterpriseId,
    provenance: &'a EnterpriseProvenance,
    fact: &'a EnterpriseFact,
}

impl EnterpriseEvent {
    pub fn new(
        enterprise_id: EnterpriseId,
        provenance: EnterpriseProvenance,
        fact: EnterpriseFact,
    ) -> Result<Self, String> {
        provenance.validate()?;
        fact.validate(&enterprise_id)?;
        fact.validate_provenance(&provenance)?;
        let event_id = EnterpriseEventId::new(format!(
            "event:{}",
            canonical_digest(&EventContent {
                schema_version: ENTERPRISE_SCHEMA_VERSION,
                enterprise_id: &enterprise_id,
                provenance: &provenance,
                fact: &fact,
            })?
        ))?;
        Ok(Self {
            schema_version: ENTERPRISE_SCHEMA_VERSION,
            event_id,
            enterprise_id,
            provenance,
            fact,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != ENTERPRISE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported enterprise event schema {}",
                self.schema_version
            ));
        }
        self.provenance.validate()?;
        self.fact.validate(&self.enterprise_id)?;
        self.fact.validate_provenance(&self.provenance)?;
        let expected = EnterpriseEvent::new(
            self.enterprise_id.clone(),
            self.provenance.clone(),
            self.fact.clone(),
        )?;
        if expected.event_id != self.event_id {
            return Err("enterprise event content digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseBatch {
    pub schema_version: u16,
    pub batch_id: EnterpriseBatchId,
    pub enterprise_id: EnterpriseId,
    pub events: Vec<EnterpriseEvent>,
}

#[derive(Serialize)]
struct BatchContent<'a> {
    schema_version: u16,
    enterprise_id: &'a EnterpriseId,
    event_ids: Vec<&'a EnterpriseEventId>,
}

impl EnterpriseBatch {
    pub fn new(
        enterprise_id: EnterpriseId,
        events: impl IntoIterator<Item = EnterpriseEvent>,
    ) -> Result<Self, String> {
        let mut by_id = BTreeMap::new();
        for event in events {
            event.validate()?;
            if event.enterprise_id != enterprise_id {
                return Err("enterprise batch contains an event for another enterprise".into());
            }
            use std::collections::btree_map::Entry;
            match by_id.entry(event.event_id.clone()) {
                Entry::Occupied(existing) if existing.get() != &event => {
                    return Err("enterprise batch contains an event-id collision".into());
                }
                Entry::Occupied(_) => {}
                Entry::Vacant(slot) => {
                    slot.insert(event);
                }
            }
        }
        if by_id.is_empty() {
            return Err("enterprise batches must contain at least one event".into());
        }
        if by_id.len() > MAX_ENTERPRISE_EVENTS_PER_BATCH {
            return Err(format!(
                "enterprise batch exceeds the {MAX_ENTERPRISE_EVENTS_PER_BATCH}-event limit"
            ));
        }
        let events = by_id.into_values().collect::<Vec<_>>();
        let event_ids = events.iter().map(|event| &event.event_id).collect();
        let batch_id = EnterpriseBatchId::new(format!(
            "batch:{}",
            canonical_digest(&BatchContent {
                schema_version: ENTERPRISE_SCHEMA_VERSION,
                enterprise_id: &enterprise_id,
                event_ids,
            })?
        ))?;
        Ok(Self {
            schema_version: ENTERPRISE_SCHEMA_VERSION,
            batch_id,
            enterprise_id,
            events,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != ENTERPRISE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported enterprise batch schema {}",
                self.schema_version
            ));
        }
        let expected = Self::new(self.enterprise_id.clone(), self.events.clone())?;
        if expected.batch_id != self.batch_id || expected.events != self.events {
            return Err("enterprise batch is not canonically ordered or its digest changed".into());
        }
        Ok(())
    }
}
