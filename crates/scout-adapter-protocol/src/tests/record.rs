use std::collections::{BTreeMap, BTreeSet};

use crate::{AdapterId, NormalizedLink, NormalizedRecord};

fn record(adapter: &str, namespace: &str) -> crate::ProtocolResult<NormalizedRecord> {
    NormalizedRecord::new(
        AdapterId::new(adapter).unwrap(),
        namespace.to_owned(),
        "github.repository".to_owned(),
        "acme".to_owned(),
        "repository:42".to_owned(),
        Some("code_repository".to_owned()),
        BTreeSet::new(),
        BTreeMap::new(),
        BTreeSet::new(),
    )
}

#[test]
fn stable_provider_namespace_is_explicit_but_record_evidence_remains_versioned() {
    let version_one = record("clark/github-organization@1", "github").unwrap();
    let version_two = record("clark/github-organization@2", "github").unwrap();

    assert_eq!(
        version_one.provider_namespace,
        version_two.provider_namespace
    );
    assert_ne!(version_one.record_id, version_two.record_id);
}

#[test]
fn provider_types_must_be_rooted_in_their_portable_namespace() {
    assert!(record("clark/github-organization@1", "aws").is_err());
    assert!(record("clark/github-organization@1", "github/unsafe").is_err());

    let link = NormalizedLink {
        relationship_type: "depends_on".to_owned(),
        target_provider_namespace: "aws".to_owned(),
        target_provider_type: "gcp.project".to_owned(),
        target_authority_scope: "account:prod".to_owned(),
        target_native_id: "project:42".to_owned(),
        qualifier: None,
    };
    assert!(link.validate().is_err());
}
