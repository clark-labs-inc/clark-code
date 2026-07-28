use std::collections::{BTreeMap, BTreeSet};

use scout_adapter_protocol::{NormalizedRecord, SafeFieldValue};
use sha2::{Digest, Sha256};

use crate::model::{
    CapsuleRequest, CapsuleResponse, NormalizationReceipt, NormalizedPage, RecordParts,
    RECEIPT_SCHEMA, REQUEST_SCHEMA, RESPONSE_SCHEMA,
};
use crate::scan::scan_json;
use crate::{CapsuleError, CapsuleLimits, CapsuleResult, CAPSULE_ABI_VERSION};

pub fn normalize_json(input: &[u8], limits: CapsuleLimits) -> CapsuleResult<Vec<u8>> {
    let limits = limits.validate()?;
    scan_json(input, limits)?;
    let request: CapsuleRequest =
        serde_json::from_slice(input).map_err(|error| CapsuleError::MalformedJson {
            line: error.line(),
            column: error.column(),
        })?;
    validate_request(&request, limits)?;

    let input_records = request.records.len();
    let mut duplicate_labels_removed = 0usize;
    let mut duplicate_links_removed = 0usize;
    let mut normalized = Vec::with_capacity(input_records);
    for candidate in request.records {
        let labels = candidate.labels.iter().cloned().collect::<BTreeSet<_>>();
        let links = candidate.links.iter().cloned().collect::<BTreeSet<_>>();
        let parts = normalize_record_parts(
            candidate.fields,
            candidate.labels.len(),
            &labels,
            candidate.links.len(),
            &links,
        )?;
        duplicate_labels_removed += parts.duplicate_labels_removed;
        duplicate_links_removed += parts.duplicate_links_removed;

        let record = NormalizedRecord::new(
            request.adapter_id.clone(),
            request.provider_namespace.clone(),
            candidate.provider_type,
            candidate.identity_authority_scope,
            candidate.native_id,
            candidate.semantic_kind,
            labels,
            parts.fields,
            links,
        )
        .map_err(|_| {
            CapsuleError::invalid(
                "records",
                "protocol validation rejected a normalized record",
            )
        })?;
        normalized.push(record);
    }
    normalized.sort_by(|left, right| left.record_id.cmp(&right.record_id));
    if normalized
        .windows(2)
        .any(|pair| pair[0].record_id == pair[1].record_id)
    {
        return Err(CapsuleError::Duplicate {
            field: "normalized record",
        });
    }

    let page = NormalizedPage::new(request.adapter_id, request.provider_namespace, normalized);
    let page_bytes = serde_json::to_vec(&page).map_err(|_| CapsuleError::Serialization)?;
    let receipt = NormalizationReceipt {
        abi_version: CAPSULE_ABI_VERSION,
        schema: RECEIPT_SCHEMA.to_owned(),
        input_sha256: sha256(input),
        normalized_page_sha256: sha256(&page_bytes),
        input_records,
        normalized_records: page.records.len(),
        duplicate_labels_removed,
        duplicate_links_removed,
    };
    let output = serde_json::to_vec(&CapsuleResponse {
        abi_version: CAPSULE_ABI_VERSION,
        schema: RESPONSE_SCHEMA.to_owned(),
        page,
        receipt,
    })
    .map_err(|_| CapsuleError::Serialization)?;
    if output.len() > limits.max_output_bytes {
        return Err(CapsuleError::limit(
            "output_bytes",
            limits.max_output_bytes,
            output.len(),
        ));
    }
    Ok(output)
}

fn validate_request(request: &CapsuleRequest, limits: CapsuleLimits) -> CapsuleResult<()> {
    if request.abi_version != CAPSULE_ABI_VERSION {
        return Err(CapsuleError::invalid(
            "abi_version",
            format!("must equal {CAPSULE_ABI_VERSION}"),
        ));
    }
    if request.schema != REQUEST_SCHEMA {
        return Err(CapsuleError::invalid(
            "schema",
            format!("must equal {REQUEST_SCHEMA}"),
        ));
    }
    request
        .adapter_id
        .validate()
        .map_err(|_| CapsuleError::invalid("adapter_id", "protocol validation rejected it"))?;
    if request.provider_namespace.is_empty()
        || request.provider_namespace.len() > 64
        || request.provider_namespace.trim() != request.provider_namespace
        || !request
            .provider_namespace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(CapsuleError::invalid(
            "provider_namespace",
            "must be a portable identifier of at most 64 bytes",
        ));
    }
    if request.records.len() > limits.max_records {
        return Err(CapsuleError::limit(
            "records",
            limits.max_records,
            request.records.len(),
        ));
    }
    Ok(())
}

fn normalize_record_parts(
    fields: Vec<crate::CandidateField>,
    label_count: usize,
    labels: &BTreeSet<String>,
    link_count: usize,
    links: &BTreeSet<scout_adapter_protocol::NormalizedLink>,
) -> CapsuleResult<RecordParts> {
    let mut normalized_fields = BTreeMap::<String, SafeFieldValue>::new();
    for field in fields {
        if normalized_fields.insert(field.name, field.value).is_some() {
            return Err(CapsuleError::Duplicate {
                field: "field name",
            });
        }
    }
    Ok(RecordParts {
        fields: normalized_fields,
        duplicate_labels_removed: label_count - labels.len(),
        duplicate_links_removed: link_count - links.len(),
    })
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
