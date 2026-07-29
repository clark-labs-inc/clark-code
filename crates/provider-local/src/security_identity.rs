use sha2::{Digest, Sha256};

use super::SecurityCandidate;

pub(super) fn candidate_fingerprint(candidate: &SecurityCandidate) -> String {
    let semantic_anchor = [
        candidate.rule_id.as_str(),
        candidate.identity_anchor.as_str(),
        candidate.identity_instance.as_deref().unwrap_or_default(),
    ]
    .map(normalize)
    .join("\0");
    hex_digest(semantic_anchor.as_bytes())
}

#[cfg(test)]
pub(super) fn inventory_digest(scope: &str, paths: &[String]) -> String {
    let snapshot = paths
        .iter()
        .cloned()
        .map(|path| (path, 0, 0))
        .collect::<Vec<_>>();
    inventory_snapshot_digest(scope, &snapshot)
}

pub(super) fn inventory_snapshot_digest(scope: &str, snapshot: &[(String, u64, u128)]) -> String {
    let mut input = format!("clark-security-inventory-v1\0{scope}").into_bytes();
    for (path, len, modified_nanos) in snapshot {
        input.push(0);
        input.extend_from_slice(path.as_bytes());
        input.push(0);
        input.extend_from_slice(len.to_string().as_bytes());
        input.push(0);
        input.extend_from_slice(modified_nanos.to_string().as_bytes());
    }
    hex_digest(&input)
}

pub(super) fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
