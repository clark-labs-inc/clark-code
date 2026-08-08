use std::collections::BTreeMap;

use scout_adapter_protocol::{AdapterId, NormalizedLink, NormalizedRecord, SafeFieldValue};
use serde::{Deserialize, Serialize};

use crate::CAPSULE_ABI_VERSION;

pub const REQUEST_SCHEMA: &str = "scout-capsule-request-v1";
pub const RESPONSE_SCHEMA: &str = "scout-capsule-response-v1";
pub const PAGE_SCHEMA: &str = "scout-capsule-normalized-page-v1";
pub const RECEIPT_SCHEMA: &str = "scout-capsule-normalization-receipt-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateField {
    pub name: String,
    pub value: SafeFieldValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateRecord {
    pub provider_type: String,
    pub identity_authority_scope: String,
    pub native_id: String,
    pub semantic_kind: Option<String>,
    pub labels: Vec<String>,
    pub fields: Vec<CandidateField>,
    pub links: Vec<NormalizedLink>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleRequest {
    pub abi_version: u16,
    pub schema: String,
    pub adapter_id: AdapterId,
    pub provider_namespace: String,
    pub records: Vec<CandidateRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedPage {
    pub abi_version: u16,
    pub schema: String,
    pub adapter_id: AdapterId,
    pub provider_namespace: String,
    pub records: Vec<NormalizedRecord>,
}

impl NormalizedPage {
    pub(crate) fn new(
        adapter_id: AdapterId,
        provider_namespace: String,
        records: Vec<NormalizedRecord>,
    ) -> Self {
        Self {
            abi_version: CAPSULE_ABI_VERSION,
            schema: PAGE_SCHEMA.to_owned(),
            adapter_id,
            provider_namespace,
            records,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizationReceipt {
    pub abi_version: u16,
    pub schema: String,
    pub input_sha256: String,
    pub normalized_page_sha256: String,
    pub input_records: usize,
    pub normalized_records: usize,
    pub duplicate_labels_removed: usize,
    pub duplicate_links_removed: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleResponse {
    pub abi_version: u16,
    pub schema: String,
    pub page: NormalizedPage,
    pub receipt: NormalizationReceipt,
}

pub(crate) struct RecordParts {
    pub fields: BTreeMap<String, SafeFieldValue>,
    pub duplicate_labels_removed: usize,
    pub duplicate_links_removed: usize,
}
