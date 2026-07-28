use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ids::{
    canonical_digest, validate_evidence, validate_text, CoverageCellId, EnterpriseEdgeId,
    EnterpriseEntityId, EnterpriseId, FrontierTaskId,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CoverageKey {
    pub adapter: String,
    pub auth_context_id: String,
    pub tenant: String,
    pub region_or_project: String,
    pub resource_kind: String,
}

impl CoverageKey {
    pub fn new(
        adapter: impl Into<String>,
        auth_context_id: impl Into<String>,
        tenant: impl Into<String>,
        region_or_project: impl Into<String>,
        resource_kind: impl Into<String>,
    ) -> Result<Self, String> {
        let value = Self {
            adapter: adapter.into(),
            auth_context_id: auth_context_id.into(),
            tenant: tenant.into(),
            region_or_project: region_or_project.into(),
            resource_kind: resource_kind.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn id(&self, enterprise_id: &EnterpriseId) -> Result<CoverageCellId, String> {
        self.validate()?;
        CoverageCellId::new(format!(
            "coverage:{}",
            canonical_digest(&("scout-coverage-v1", enterprise_id, self))?
        ))
    }

    fn validate(&self) -> Result<(), String> {
        validate_text("coverage adapter", &self.adapter, 128)?;
        validate_text(
            "coverage authentication context",
            &self.auth_context_id,
            256,
        )?;
        validate_text("coverage tenant", &self.tenant, 1_024)?;
        validate_text("coverage region or project", &self.region_or_project, 1_024)?;
        validate_text("coverage resource kind", &self.resource_kind, 256)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Supported,
    Empty,
    Denied,
    Unreachable,
    Unsupported,
    Unsafe,
    Stale,
    Truncated,
    Untested,
}

impl CoverageStatus {
    pub fn is_complete(self) -> bool {
        matches!(self, Self::Supported | Self::Empty)
    }

    pub fn blocks_complete(self) -> bool {
        !self.is_complete()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageObservation {
    pub cell_id: CoverageCellId,
    pub key: CoverageKey,
    pub status: CoverageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub enumerated_count: u64,
    #[serde(default)]
    pub enumerated_edge_count: u64,
    pub evidence_digests: BTreeSet<String>,
}

impl CoverageObservation {
    pub fn new(
        enterprise_id: &EnterpriseId,
        key: CoverageKey,
        status: CoverageStatus,
        next_cursor: Option<String>,
        enumerated_count: u64,
        evidence_digests: BTreeSet<String>,
    ) -> Result<Self, String> {
        let cell_id = key.id(enterprise_id)?;
        let value = Self {
            cell_id,
            key,
            status,
            next_cursor,
            enumerated_count,
            enumerated_edge_count: 0,
            evidence_digests,
        };
        value.validate(enterprise_id)?;
        Ok(value)
    }

    pub(super) fn validate(&self, enterprise_id: &EnterpriseId) -> Result<(), String> {
        if self.cell_id != self.key.id(enterprise_id)? {
            return Err("coverage cell id does not match its typed key".into());
        }
        validate_optional_cursor_handle("coverage cursor", self.next_cursor.as_deref())?;
        if self.status.is_complete() && self.next_cursor.is_some() {
            return Err("complete coverage cannot retain a next-page cursor".into());
        }
        if self.status == CoverageStatus::Empty
            && (self.enumerated_count != 0 || self.enumerated_edge_count != 0)
        {
            return Err("empty coverage must have zero entity and edge counts".into());
        }
        validate_evidence(&self.evidence_digests)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FrontierKey {
    pub coverage: CoverageKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl FrontierKey {
    pub fn new(coverage: CoverageKey, cursor: Option<String>) -> Result<Self, String> {
        let value = Self { coverage, cursor };
        value.validate()?;
        Ok(value)
    }

    pub fn id(&self, enterprise_id: &EnterpriseId) -> Result<FrontierTaskId, String> {
        self.validate()?;
        FrontierTaskId::new(format!(
            "frontier:{}",
            canonical_digest(&("scout-frontier-v1", enterprise_id, self))?
        ))
    }

    fn validate(&self) -> Result<(), String> {
        self.coverage.validate()?;
        validate_optional_cursor_handle("frontier cursor", self.cursor.as_deref())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FrontierState {
    Pending,
    Leased {
        owner: String,
        expires_at_ms: u64,
    },
    PageComplete {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_cursor: Option<String>,
        discovered: u64,
    },
    Terminal {
        status: CoverageStatus,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontierObservation {
    pub task_id: FrontierTaskId,
    pub key: FrontierKey,
    #[serde(default = "default_transition_sequence")]
    pub transition_sequence: u64,
    pub state: FrontierState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<FrontierTaskId>,
    #[serde(default)]
    pub discovered_entity_ids: BTreeSet<EnterpriseEntityId>,
    #[serde(default)]
    pub discovered_edge_ids: BTreeSet<EnterpriseEdgeId>,
    #[serde(default)]
    pub evidence_digests: BTreeSet<String>,
}

impl FrontierObservation {
    pub fn new(
        enterprise_id: &EnterpriseId,
        key: FrontierKey,
        state: FrontierState,
        evidence_digests: BTreeSet<String>,
    ) -> Result<Self, String> {
        let task_id = key.id(enterprise_id)?;
        let value = Self {
            task_id,
            key,
            transition_sequence: 1,
            state,
            parent_task_id: None,
            discovered_entity_ids: BTreeSet::new(),
            discovered_edge_ids: BTreeSet::new(),
            evidence_digests,
        };
        value.validate(enterprise_id)?;
        Ok(value)
    }

    pub(super) fn validate(&self, enterprise_id: &EnterpriseId) -> Result<(), String> {
        if self.task_id != self.key.id(enterprise_id)? {
            return Err("frontier task id does not match its typed key".into());
        }
        if self.transition_sequence == 0 {
            return Err("frontier transition sequence must be positive".into());
        }
        if self.discovered_entity_ids.len() > 100_000 || self.discovered_edge_ids.len() > 100_000 {
            return Err("frontier page exceeds the 100000-member limit".into());
        }
        match &self.state {
            FrontierState::Pending => {}
            FrontierState::Leased {
                owner,
                expires_at_ms,
            } => {
                validate_text("frontier lease owner", owner, 256)?;
                if *expires_at_ms == 0 {
                    return Err("frontier lease expiry must be positive".into());
                }
            }
            FrontierState::PageComplete { next_cursor, .. } => {
                validate_optional_cursor_handle("frontier next cursor", next_cursor.as_deref())?;
                if self.evidence_digests.is_empty() {
                    return Err("completed frontier pages require evidence".into());
                }
            }
            FrontierState::Terminal { status, reason } => {
                validate_text("frontier terminal reason", reason, 2_048)?;
                if matches!(status, CoverageStatus::Untested) {
                    return Err("untested frontier work cannot be terminal".into());
                }
                validate_evidence(&self.evidence_digests)?;
            }
        }
        Ok(())
    }
}

fn default_transition_sequence() -> u64 {
    1
}

fn validate_optional_cursor_handle(label: &str, value: Option<&str>) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_text(label, value, 64)?;
    let Some(raw_uuid) = value.strip_prefix("cursor:") else {
        return Err(format!(
            "{label} must be a host-issued cursor:<uuid> handle, never a provider cursor"
        ));
    };
    let parsed = Uuid::parse_str(raw_uuid)
        .map_err(|_| format!("{label} must contain a canonical UUID handle"))?;
    if parsed.hyphenated().to_string() != raw_uuid {
        return Err(format!("{label} must contain a canonical UUID handle"));
    }
    Ok(())
}
