use serde::{Deserialize, Serialize};

use super::validation::{
    canonical_json, validate_namespace, validate_prefixed_digest, validate_text,
};
use super::TerminalDisposition;
use crate::cartography::crypto::sha256_hex;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityIdentity {
    pub entity_kind: String,
    pub provider_namespace: String,
    pub authority_scope: String,
    pub provider_native_id: String,
}

impl EntityIdentity {
    pub fn validate(&self) -> Result<(), String> {
        validate_namespace("entity kind", &self.entity_kind)?;
        validate_namespace("entity provider namespace", &self.provider_namespace)?;
        validate_text("entity authority scope", &self.authority_scope, 1, 1_024)?;
        validate_text(
            "entity provider-native id",
            &self.provider_native_id,
            1,
            2_048,
        )
    }

    pub fn entity_id(&self) -> Result<String, String> {
        self.validate()?;
        Ok(format!("entity:{}", sha256_hex(&canonical_json(self)?)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeIdentity {
    pub edge_kind: String,
    pub source: EntityIdentity,
    pub target: EntityIdentity,
    pub qualifier: Option<String>,
}

impl EdgeIdentity {
    pub fn validate(&self) -> Result<(), String> {
        validate_namespace("edge kind", &self.edge_kind)?;
        self.source.validate()?;
        self.target.validate()?;
        if let Some(qualifier) = &self.qualifier {
            validate_text("edge qualifier", qualifier, 1, 512)?;
        }
        Ok(())
    }

    pub fn edge_id(&self) -> Result<String, String> {
        self.validate()?;
        Ok(format!("edge:{}", sha256_hex(&canonical_json(self)?)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClaimTarget {
    Entity { entity: EntityIdentity },
    Edge { edge: EdgeIdentity },
}

impl ClaimTarget {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Entity { entity } => entity.validate(),
            Self::Edge { edge } => edge.validate(),
        }
    }

    pub fn object_id(&self) -> Result<String, String> {
        match self {
            Self::Entity { entity } => entity.entity_id(),
            Self::Edge { edge } => edge.edge_id(),
        }
    }

    pub fn object_type(&self) -> &'static str {
        match self {
            Self::Entity { .. } => "entity",
            Self::Edge { .. } => "edge",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimIdentity {
    pub claim_kind: String,
    pub target: ClaimTarget,
    pub predicate: String,
}

impl ClaimIdentity {
    pub fn validate(&self) -> Result<(), String> {
        validate_namespace("claim kind", &self.claim_kind)?;
        self.target.validate()?;
        validate_namespace("claim predicate", &self.predicate)
    }

    pub fn claim_id(&self) -> Result<String, String> {
        self.validate()?;
        Ok(format!("claim:{}", sha256_hex(&canonical_json(self)?)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObservationSubject {
    Entity {
        entity: EntityIdentity,
    },
    Edge {
        edge: EdgeIdentity,
    },
    Claim {
        claim: ClaimIdentity,
    },
    Coverage {
        coverage_key: String,
        disposition: TerminalDisposition,
        complete: bool,
        continuation_handle: Option<String>,
    },
    Retraction {
        target_event_id: String,
        reason: String,
    },
}

impl ObservationSubject {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Entity { entity } => entity.validate(),
            Self::Edge { edge } => edge.validate(),
            Self::Claim { claim } => claim.validate(),
            Self::Coverage {
                coverage_key,
                disposition,
                complete,
                continuation_handle,
            } => {
                validate_text("coverage key", coverage_key, 1, 512)?;
                if let Some(handle) = continuation_handle {
                    validate_text("coverage continuation handle", handle, 1, 512)?;
                }
                match disposition {
                    TerminalDisposition::Supported | TerminalDisposition::Empty if !complete => {
                        Err("supported or empty coverage must be complete".into())
                    }
                    TerminalDisposition::Truncated if *complete => {
                        Err("truncated coverage cannot be complete".into())
                    }
                    _ => Ok(()),
                }
            }
            Self::Retraction {
                target_event_id,
                reason,
            } => {
                validate_prefixed_digest("retraction target event", target_event_id, "event:")?;
                validate_text("retraction reason", reason, 1, 2_048)
            }
        }
    }

    pub fn subject_type(&self) -> &'static str {
        match self {
            Self::Entity { .. } => "entity",
            Self::Edge { .. } => "edge",
            Self::Claim { .. } => "claim",
            Self::Coverage { .. } => "coverage",
            Self::Retraction { .. } => "retraction",
        }
    }
}
