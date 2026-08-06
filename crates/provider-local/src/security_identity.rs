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
        .map(|path| {
            let content_sha256 = hex_digest(path.as_bytes());
            (path, content_sha256)
        })
        .collect::<Vec<_>>();
    inventory_snapshot_digest(scope, &snapshot)
}

/// Digest the inventoried target as (path, content SHA-256) pairs. Metadata
/// such as mtime is deliberately excluded: touching a file or rewriting
/// identical bytes must not rotate the id mid-scan, while any real content
/// change still does.
pub(super) fn inventory_snapshot_digest(scope: &str, snapshot: &[(String, String)]) -> String {
    let mut input = format!("clark-security-inventory-v2\0{scope}").into_bytes();
    for (path, content_sha256) in snapshot {
        input.push(0);
        input.extend_from_slice(path.as_bytes());
        input.push(0);
        input.extend_from_slice(content_sha256.as_bytes());
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
