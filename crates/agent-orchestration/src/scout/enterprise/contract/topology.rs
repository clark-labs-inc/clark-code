use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::classification::{is_default_classification, EnterpriseClassification};
use super::ids::{
    canonical_digest, validate_digest, validate_evidence, validate_string_set, validate_text,
    EnterpriseEdgeId, EnterpriseEntityId, EnterpriseId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnterpriseEntityKind {
    Organization,
    OrganizationUnit,
    CloudFolder,
    BusinessUnit,
    Product,
    Capability,
    Journey,
    Team,
    Owner,
    Actor,
    IdentityTenant,
    AuthContext,
    Principal,
    IdentityGroup,
    Role,
    Policy,
    CloudAccount,
    CloudProject,
    CloudResource,
    Environment,
    Region,
    Repository,
    Component,
    Pipeline,
    Artifact,
    IacStack,
    Deployment,
    Service,
    Function,
    Job,
    Api,
    Endpoint,
    Cluster,
    Namespace,
    Host,
    Network,
    Subnet,
    Firewall,
    Database,
    Dataset,
    Cache,
    ObjectStore,
    Queue,
    Topic,
    EventSchema,
    SecretReference,
    TraceService,
    LogSource,
    Metric,
    Monitor,
    Alarm,
    IncidentRoute,
    Runbook,
    BackupPlan,
    BackupVault,
    Snapshot,
    RestoreTest,
    DnsZone,
    Certificate,
    Vendor,
    SaasIntegration,
    Webhook,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnterpriseEdgeKind {
    Contains,
    Owns,
    OwnedBy,
    Implements,
    SourceFor,
    Builds,
    Publishes,
    DeploysTo,
    Provisions,
    RunsOn,
    RoutesTo,
    Calls,
    PublishesTo,
    ConsumesFrom,
    Reads,
    Writes,
    AuthenticatesVia,
    MemberOf,
    Grants,
    Assumes,
    ConfiguredBy,
    MonitoredBy,
    AlertsTo,
    DependsOn,
    ConnectedTo,
    BackedUpBy,
    Restores,
    ReplicatesTo,
    EntersThrough,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AuthorityRef {
    pub provider_namespace: String,
    pub authority_scope: String,
    pub native_id: String,
}

impl AuthorityRef {
    pub fn new(
        provider_namespace: impl Into<String>,
        authority_scope: impl Into<String>,
        native_id: impl Into<String>,
    ) -> Result<Self, String> {
        let value = Self {
            provider_namespace: provider_namespace.into(),
            authority_scope: authority_scope.into(),
            native_id: native_id.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        validate_text("authority provider namespace", &self.provider_namespace, 64)?;
        if !self
            .provider_namespace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
        {
            return Err(
                "authority provider namespace contains non-portable identifier characters".into(),
            );
        }
        validate_text("authority scope", &self.authority_scope, 1_024)?;
        validate_text("provider-native id", &self.native_id, 4_096)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EnterpriseProvenance {
    pub machine_id: String,
    pub run_id: String,
    pub adapter_instance_id: String,
    pub auth_context_id: String,
    pub discovery_epoch: String,
    pub discovery_epoch_sequence: u64,
    pub source_sequence: u64,
    pub observed_at_ms: u64,
    pub source_fingerprint: String,
}

impl EnterpriseProvenance {
    pub fn validate(&self) -> Result<(), String> {
        validate_text("machine id", &self.machine_id, 256)?;
        validate_text("run id", &self.run_id, 256)?;
        validate_text("adapter instance id", &self.adapter_instance_id, 256)?;
        validate_text("authentication context id", &self.auth_context_id, 256)?;
        validate_text("discovery epoch", &self.discovery_epoch, 256)?;
        if self.discovery_epoch_sequence == 0 {
            return Err("discovery epoch sequence must be positive".into());
        }
        if self.source_sequence == 0 {
            return Err("source sequence must be positive".into());
        }
        if self.observed_at_ms == 0 {
            return Err("observation time must be positive".into());
        }
        validate_digest("source fingerprint", &self.source_fingerprint)
    }

    pub(crate) fn source_position(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}",
            self.machine_id,
            self.run_id,
            self.adapter_instance_id,
            self.auth_context_id,
            self.discovery_epoch,
            self.discovery_epoch_sequence,
            self.source_sequence
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEntityObservation {
    pub entity_id: EnterpriseEntityId,
    pub kind: EnterpriseEntityKind,
    pub authority: AuthorityRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_resource_type: Option<String>,
    #[serde(default)]
    pub labels: BTreeSet<String>,
    #[serde(default)]
    pub environments: BTreeSet<String>,
    #[serde(default)]
    pub critical: bool,
    #[serde(default, skip_serializing_if = "is_default_classification")]
    pub classification: EnterpriseClassification,
    pub evidence_digests: BTreeSet<String>,
}

impl GraphEntityObservation {
    pub fn new(
        enterprise_id: &EnterpriseId,
        kind: EnterpriseEntityKind,
        authority: AuthorityRef,
        labels: BTreeSet<String>,
        evidence_digests: BTreeSet<String>,
    ) -> Result<Self, String> {
        let entity_id = EnterpriseEntityId::derive(enterprise_id, kind, &authority)?;
        let value = Self {
            entity_id,
            kind,
            authority,
            provider_resource_type: None,
            labels,
            environments: BTreeSet::new(),
            critical: false,
            classification: EnterpriseClassification::Internal,
            evidence_digests,
        };
        value.validate(enterprise_id)?;
        Ok(value)
    }

    pub(super) fn validate(&self, enterprise_id: &EnterpriseId) -> Result<(), String> {
        self.authority.validate()?;
        if let Some(resource_type) = &self.provider_resource_type {
            validate_text("provider resource type", resource_type, 1_024)?;
        }
        if self.entity_id != EnterpriseEntityId::derive(enterprise_id, self.kind, &self.authority)?
        {
            return Err("enterprise entity id does not match its authority tuple".into());
        }
        validate_string_set("entity label", &self.labels, 64, 1_024)?;
        validate_string_set("entity environment", &self.environments, 32, 256)?;
        self.classification.validate_persistable()?;
        validate_evidence(&self.evidence_digests)
    }
}

impl EnterpriseEntityId {
    pub fn derive(
        enterprise_id: &EnterpriseId,
        kind: EnterpriseEntityKind,
        authority: &AuthorityRef,
    ) -> Result<Self, String> {
        authority.validate()?;
        let digest =
            canonical_digest(&("scout-enterprise-entity-v2", enterprise_id, kind, authority))?;
        Self::new(format!("ent:{digest}"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdgeObservation {
    pub edge_id: EnterpriseEdgeId,
    pub from: EnterpriseEntityId,
    pub to: EnterpriseEntityId,
    pub kind: EnterpriseEdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    #[serde(default, skip_serializing_if = "is_default_classification")]
    pub classification: EnterpriseClassification,
    pub evidence_digests: BTreeSet<String>,
}

impl GraphEdgeObservation {
    pub fn new(
        enterprise_id: &EnterpriseId,
        from: EnterpriseEntityId,
        to: EnterpriseEntityId,
        kind: EnterpriseEdgeKind,
        qualifier: Option<String>,
        evidence_digests: BTreeSet<String>,
    ) -> Result<Self, String> {
        let edge_id =
            EnterpriseEdgeId::derive(enterprise_id, &from, &to, kind, qualifier.as_deref())?;
        let value = Self {
            edge_id,
            from,
            to,
            kind,
            qualifier,
            classification: EnterpriseClassification::Internal,
            evidence_digests,
        };
        value.validate(enterprise_id)?;
        Ok(value)
    }

    pub(super) fn validate(&self, enterprise_id: &EnterpriseId) -> Result<(), String> {
        if self.from == self.to {
            return Err("enterprise graph edges cannot be self-referential".into());
        }
        if let Some(qualifier) = &self.qualifier {
            validate_text("edge qualifier", qualifier, 1_024)?;
        }
        if self.edge_id
            != EnterpriseEdgeId::derive(
                enterprise_id,
                &self.from,
                &self.to,
                self.kind,
                self.qualifier.as_deref(),
            )?
        {
            return Err("enterprise edge id does not match its endpoint tuple".into());
        }
        self.classification.validate_persistable()?;
        validate_evidence(&self.evidence_digests)
    }
}

impl EnterpriseEdgeId {
    pub fn derive(
        enterprise_id: &EnterpriseId,
        from: &EnterpriseEntityId,
        to: &EnterpriseEntityId,
        kind: EnterpriseEdgeKind,
        qualifier: Option<&str>,
    ) -> Result<Self, String> {
        if let Some(qualifier) = qualifier {
            validate_text("edge qualifier", qualifier, 1_024)?;
        }
        let digest = canonical_digest(&(
            "scout-enterprise-edge-v1",
            enterprise_id,
            from,
            to,
            kind,
            qualifier,
        ))?;
        Self::new(format!("edge:{digest}"))
    }
}
