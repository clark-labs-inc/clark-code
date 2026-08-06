use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::fingerprint::canonical_sha256;
use crate::validate::{
    validate_fields, validate_identifier, validate_safe_text, validate_string_set,
    MAX_SAFE_TEXT_BYTES,
};
use crate::{AdapterId, ProtocolError, ProtocolResult, RecordId};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SafeFieldValue {
    Text(String),
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
    TextSet(BTreeSet<String>),
}

impl SafeFieldValue {
    pub fn validate(&self) -> ProtocolResult<()> {
        match self {
            Self::Text(value) => validate_safe_text("field_value", value, MAX_SAFE_TEXT_BYTES),
            Self::TextSet(values) => {
                validate_string_set("field_value", values, 256, MAX_SAFE_TEXT_BYTES)
            }
            Self::Boolean(_) | Self::Signed(_) | Self::Unsigned(_) => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedLink {
    pub relationship_type: String,
    pub target_provider_namespace: String,
    pub target_provider_type: String,
    pub target_authority_scope: String,
    pub target_native_id: String,
    pub qualifier: Option<String>,
}

impl NormalizedLink {
    pub fn validate(&self) -> ProtocolResult<()> {
        validate_identifier("relationship_type", &self.relationship_type, 128)?;
        validate_identifier(
            "target_provider_namespace",
            &self.target_provider_namespace,
            64,
        )?;
        validate_safe_text("target_provider_type", &self.target_provider_type, 256)?;
        validate_provider_binding(
            "target_provider_namespace",
            &self.target_provider_namespace,
            "target_provider_type",
            &self.target_provider_type,
        )?;
        validate_safe_text("target_authority_scope", &self.target_authority_scope, 512)?;
        validate_safe_text("target_native_id", &self.target_native_id, 1_024)?;
        if let Some(qualifier) = &self.qualifier {
            validate_safe_text("qualifier", qualifier, 512)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedRecord {
    pub record_id: RecordId,
    pub adapter_id: AdapterId,
    pub provider_namespace: String,
    pub provider_type: String,
    pub identity_authority_scope: String,
    pub native_id: String,
    pub semantic_kind: Option<String>,
    pub labels: BTreeSet<String>,
    pub fields: BTreeMap<String, SafeFieldValue>,
    pub links: BTreeSet<NormalizedLink>,
}

impl NormalizedRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        adapter_id: AdapterId,
        provider_namespace: String,
        provider_type: String,
        identity_authority_scope: String,
        native_id: String,
        semantic_kind: Option<String>,
        labels: BTreeSet<String>,
        fields: BTreeMap<String, SafeFieldValue>,
        links: BTreeSet<NormalizedLink>,
    ) -> ProtocolResult<Self> {
        let record_id = derive_record_id(
            &adapter_id,
            &provider_namespace,
            &provider_type,
            &identity_authority_scope,
            &native_id,
            &semantic_kind,
            &labels,
            &fields,
            &links,
        )?;
        let record = Self {
            record_id,
            adapter_id,
            provider_namespace,
            provider_type,
            identity_authority_scope,
            native_id,
            semantic_kind,
            labels,
            fields,
            links,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> ProtocolResult<()> {
        self.record_id.validate()?;
        self.adapter_id.validate()?;
        validate_identifier("provider_namespace", &self.provider_namespace, 64)?;
        validate_safe_text("provider_type", &self.provider_type, 256)?;
        validate_provider_binding(
            "provider_namespace",
            &self.provider_namespace,
            "provider_type",
            &self.provider_type,
        )?;
        validate_safe_text(
            "identity_authority_scope",
            &self.identity_authority_scope,
            512,
        )?;
        validate_safe_text("native_id", &self.native_id, 1_024)?;
        if let Some(semantic_kind) = &self.semantic_kind {
            validate_identifier("semantic_kind", semantic_kind, 128)?;
        }
        validate_string_set("labels", &self.labels, 128, 256)?;
        validate_fields(&self.fields, 128)?;
        if self.links.len() > 256 {
            return Err(ProtocolError::invalid(
                "links",
                "exceeds the 256-item limit",
            ));
        }
        for link in &self.links {
            link.validate()?;
        }
        let expected = derive_record_id(
            &self.adapter_id,
            &self.provider_namespace,
            &self.provider_type,
            &self.identity_authority_scope,
            &self.native_id,
            &self.semantic_kind,
            &self.labels,
            &self.fields,
            &self.links,
        )?;
        if self.record_id != expected {
            return Err(ProtocolError::invalid(
                "record_id",
                "does not match normalized record content",
            ));
        }
        Ok(())
    }

    pub fn fingerprint_sha256(&self) -> ProtocolResult<String> {
        self.validate()?;
        Ok(self
            .record_id
            .as_str()
            .strip_prefix("record:")
            .expect("validated record identifiers have a fixed prefix")
            .to_owned())
    }
}

fn validate_provider_binding(
    namespace_field: &'static str,
    namespace: &str,
    provider_type_field: &'static str,
    provider_type: &str,
) -> ProtocolResult<()> {
    if !provider_type
        .strip_prefix(namespace)
        .is_some_and(|suffix| suffix.starts_with('.'))
    {
        return Err(ProtocolError::invalid(
            provider_type_field,
            format!("must be rooted in {namespace_field} `{namespace}`"),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn derive_record_id(
    adapter_id: &AdapterId,
    provider_namespace: &str,
    provider_type: &str,
    identity_authority_scope: &str,
    native_id: &str,
    semantic_kind: &Option<String>,
    labels: &BTreeSet<String>,
    fields: &BTreeMap<String, SafeFieldValue>,
    links: &BTreeSet<NormalizedLink>,
) -> ProtocolResult<RecordId> {
    let digest = canonical_sha256(&(
        adapter_id,
        provider_namespace,
        provider_type,
        identity_authority_scope,
        native_id,
        semantic_kind,
        labels,
        fields,
        links,
    ))?;
    RecordId::from_digest(digest)
}
