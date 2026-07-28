use std::collections::{BTreeMap, BTreeSet};

use scout_adapter_protocol::{AdapterId, TargetId};
use serde::{Deserialize, Serialize};

use crate::model::{
    canonical_sha256, validate_identifier, validate_prefixed_digest, validate_text, SchedulerTaskId,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RouteKind {
    pub adapter_id: AdapterId,
    pub operation: String,
    pub provider_resource_type: String,
}

impl RouteKind {
    pub fn new(
        adapter_id: AdapterId,
        operation: impl Into<String>,
        provider_resource_type: impl Into<String>,
    ) -> Result<Self, String> {
        let route = Self {
            adapter_id,
            operation: operation.into(),
            provider_resource_type: provider_resource_type.into(),
        };
        route.validate()?;
        Ok(route)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        self.adapter_id
            .validate()
            .map_err(|error| error.to_string())?;
        validate_identifier("route operation", &self.operation, 128)?;
        validate_text(
            "route provider resource type",
            &self.provider_resource_type,
            256,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QuotaKey(String);

impl QuotaKey {
    pub(crate) fn new(
        target_id: &TargetId,
        adapter_id: &AdapterId,
        authority_scope: &str,
    ) -> Result<Self, String> {
        target_id.validate().map_err(|error| error.to_string())?;
        adapter_id.validate().map_err(|error| error.to_string())?;
        validate_text("quota authority scope", authority_scope, 512)?;
        Ok(Self(format!(
            "scheduler-quota:{}",
            canonical_sha256(&(
                "scout-scheduler-quota-v1",
                target_id,
                adapter_id,
                authority_scope,
            ))?
        )))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_prefixed_digest("scheduler quota key", &self.0, "scheduler-quota:")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaPolicy {
    pub max_in_flight: u16,
    pub min_start_interval_ms: u64,
    pub lease_duration_ms: u64,
    pub base_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub max_attempts: u16,
}

impl QuotaPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_in_flight == 0 || self.max_in_flight > 1_024 {
            return Err("scheduler max_in_flight must be in 1..=1024".into());
        }
        if self.min_start_interval_ms > 3_600_000 {
            return Err("scheduler min_start_interval_ms exceeds one hour".into());
        }
        if !(1_000..=3_600_000).contains(&self.lease_duration_ms) {
            return Err("scheduler lease_duration_ms must be in 1000..=3600000".into());
        }
        if self.base_backoff_ms == 0
            || self.base_backoff_ms > self.max_backoff_ms
            || self.max_backoff_ms > 86_400_000
        {
            return Err("scheduler backoff bounds are invalid".into());
        }
        if self.max_attempts == 0 || self.max_attempts > 100 {
            return Err("scheduler max_attempts must be in 1..=100".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpansionRule {
    pub rule_id: String,
    pub parent: RouteKind,
    pub child: RouteKind,
    pub same_target: bool,
    pub max_children_per_parent: u32,
}

impl ExpansionRule {
    pub fn validate(&self) -> Result<(), String> {
        validate_identifier("scheduler expansion rule", &self.rule_id, 128)?;
        self.parent.validate()?;
        self.child.validate()?;
        if self.max_children_per_parent == 0 || self.max_children_per_parent > 1_000_000 {
            return Err("scheduler expansion child limit must be in 1..=1000000".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleManifest {
    pub manifest_id: String,
    pub enterprise_id: String,
    pub charter_id: String,
    pub discovery_epoch: u64,
    pub root_task_ids: BTreeSet<SchedulerTaskId>,
    pub quota_policies: BTreeMap<QuotaKey, QuotaPolicy>,
    pub expansion_rules: BTreeMap<String, ExpansionRule>,
}

impl ScheduleManifest {
    pub fn new(
        enterprise_id: impl Into<String>,
        charter_id: impl Into<String>,
        discovery_epoch: u64,
        root_task_ids: BTreeSet<SchedulerTaskId>,
        quota_policies: BTreeMap<QuotaKey, QuotaPolicy>,
        expansion_rules: BTreeMap<String, ExpansionRule>,
    ) -> Result<Self, String> {
        let mut manifest = Self {
            manifest_id: String::new(),
            enterprise_id: enterprise_id.into(),
            charter_id: charter_id.into(),
            discovery_epoch,
            root_task_ids,
            quota_policies,
            expansion_rules,
        };
        manifest.manifest_id = derive_manifest_id(&manifest)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_prefixed_digest(
            "scheduler manifest id",
            &self.manifest_id,
            "scheduler-manifest:",
        )?;
        validate_text("scheduler enterprise id", &self.enterprise_id, 256)?;
        validate_text("scheduler charter id", &self.charter_id, 256)?;
        if self.root_task_ids.is_empty() {
            return Err("scheduler manifest requires at least one root task".into());
        }
        for task_id in &self.root_task_ids {
            task_id.validate()?;
        }
        if self.quota_policies.is_empty() {
            return Err("scheduler manifest requires explicit quota policies".into());
        }
        for (key, policy) in &self.quota_policies {
            key.validate()?;
            policy.validate()?;
        }
        for (rule_id, rule) in &self.expansion_rules {
            rule.validate()?;
            if rule_id != &rule.rule_id {
                return Err("scheduler expansion rule key disagrees with its id".into());
            }
        }
        if self.manifest_id != derive_manifest_id(self)? {
            return Err("scheduler manifest id does not match its content".into());
        }
        Ok(())
    }
}

fn derive_manifest_id(manifest: &ScheduleManifest) -> Result<String, String> {
    Ok(format!(
        "scheduler-manifest:{}",
        canonical_sha256(&(
            "scout-scheduler-manifest-v1",
            &manifest.enterprise_id,
            &manifest.charter_id,
            manifest.discovery_epoch,
            &manifest.root_task_ids,
            &manifest.quota_policies,
            &manifest.expansion_rules,
        ))?
    ))
}
