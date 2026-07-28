use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::discovery::CoverageKey;
use super::ids::{validate_digest, validate_evidence, EnterpriseEntityId};

const MAX_REQUIRED_COVERAGE_CELLS: usize = 100_000;
const MAX_CRITICAL_JOURNEYS: usize = 100_000;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DiscoveryCharterObservation {
    pub charter_id: String,
    pub revision: u64,
    pub max_age_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    pub required_coverage: BTreeSet<CoverageKey>,
    pub critical_journey_ids: BTreeSet<EnterpriseEntityId>,
    pub critical_runtime_ids: BTreeSet<EnterpriseEntityId>,
    pub evidence_digests: BTreeSet<String>,
}

impl DiscoveryCharterObservation {
    pub fn validate(&self) -> Result<(), String> {
        validate_charter_id(&self.charter_id)?;
        if self.revision == 0 {
            return Err("enterprise discovery charter revision must be positive".into());
        }
        if self.max_age_ms == 0 || self.max_age_ms > 31_536_000_000 {
            return Err(
                "enterprise discovery charter max_age_ms must be in 1..=31536000000".into(),
            );
        }
        if let Some(supersedes) = &self.supersedes {
            validate_charter_id(supersedes)?;
            if supersedes == &self.charter_id {
                return Err("enterprise discovery charter cannot supersede itself".into());
            }
        }
        if (self.revision == 1) != self.supersedes.is_none() {
            return Err(
                "charter revision 1 must have no predecessor and later revisions must supersede one"
                    .into(),
            );
        }
        if self.required_coverage.is_empty() {
            return Err("enterprise discovery charter requires at least one coverage cell".into());
        }
        if self.required_coverage.len() > MAX_REQUIRED_COVERAGE_CELLS {
            return Err(format!(
                "enterprise discovery charter exceeds {MAX_REQUIRED_COVERAGE_CELLS} coverage cells"
            ));
        }
        if self.critical_journey_ids.is_empty() {
            return Err(
                "enterprise discovery charter requires at least one critical business journey"
                    .into(),
            );
        }
        if self.critical_journey_ids.len() > MAX_CRITICAL_JOURNEYS {
            return Err(format!(
                "enterprise discovery charter exceeds {MAX_CRITICAL_JOURNEYS} critical journeys"
            ));
        }
        if self.critical_runtime_ids.is_empty() {
            return Err(
                "enterprise discovery charter requires at least one critical runtime".into(),
            );
        }
        if self.critical_runtime_ids.len() > MAX_CRITICAL_JOURNEYS {
            return Err(format!(
                "enterprise discovery charter exceeds {MAX_CRITICAL_JOURNEYS} critical runtimes"
            ));
        }
        validate_evidence(&self.evidence_digests)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DiscoveryPassSealObservation {
    pub pass_id: String,
    pub charter_id: String,
    pub discovery_epoch: String,
    pub discovery_epoch_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_pass_id: Option<String>,
    pub requirement_root: String,
    pub scope_root: String,
    pub topology_root: String,
    pub evidence_digests: BTreeSet<String>,
}

impl DiscoveryPassSealObservation {
    pub fn validate(&self) -> Result<(), String> {
        validate_pass_id(&self.pass_id)?;
        validate_charter_id(&self.charter_id)?;
        if self.discovery_epoch.trim().is_empty() || self.discovery_epoch.len() > 256 {
            return Err("discovery pass epoch must contain 1 to 256 characters".into());
        }
        if self.discovery_epoch_sequence == 0 {
            return Err("discovery pass epoch sequence must be positive".into());
        }
        if let Some(previous) = &self.previous_pass_id {
            validate_pass_id(previous)?;
            if previous == &self.pass_id {
                return Err("discovery pass cannot name itself as its predecessor".into());
            }
        }
        validate_digest("discovery pass requirement root", &self.requirement_root)?;
        validate_digest("discovery pass scope root", &self.scope_root)?;
        validate_digest("discovery pass topology root", &self.topology_root)?;
        validate_evidence(&self.evidence_digests)
    }
}

fn validate_charter_id(value: &str) -> Result<(), String> {
    let Some(raw_uuid) = value.strip_prefix("charter:") else {
        return Err("discovery charter id must be a coordinator-issued charter:<uuid>".into());
    };
    let parsed = Uuid::parse_str(raw_uuid)
        .map_err(|_| "discovery charter id must contain a canonical UUID".to_string())?;
    if parsed.hyphenated().to_string() != raw_uuid {
        return Err("discovery charter id must contain a canonical UUID".into());
    }
    Ok(())
}

fn validate_pass_id(value: &str) -> Result<(), String> {
    let Some(digest) = value.strip_prefix("pass:") else {
        return Err("discovery pass id must be a content-addressed pass:<digest>".into());
    };
    validate_digest("discovery pass id", digest)
}
