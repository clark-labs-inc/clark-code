use std::collections::BTreeSet;

use scout_ingest_protocol::cartography::{
    BatchEnvelope, Classification, CollectorSigningKey, EntityIdentity, EvidenceCommitRequest,
    EvidenceObjectRef, EvidenceUploadRequest, ObservationEvent, ObservationFact,
    ObservationSubject, TaskClaimRequest, TaskCompletion, TerminalDisposition,
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn portable_signing_matches_the_backend_wire_fixture() {
    let organization_id = Uuid::from_u128(1);
    let workspace_id = Uuid::from_u128(2);
    let run_id = Uuid::from_u128(3);
    let source_id = Uuid::from_u128(4);
    let machine_id = Uuid::from_u128(5);
    let task_id = Uuid::from_u128(6);
    let signer = CollectorSigningKey::from_seed([7; 32]);
    assert_eq!(
        signer.public_key_hex(),
        "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c"
    );
    let claim = TaskClaimRequest::sign(
        organization_id,
        workspace_id,
        run_id,
        machine_id,
        "0123456789abcdef".into(),
        1_700_000_000_000,
        60,
        &signer,
    )
    .unwrap();
    assert_eq!(
        claim.request_id,
        "task-claim:f68c9ae8cf296e85ce6afe58b90befc351c9ff6b89bfb3f6caa936f3d41ba0c9"
    );
    assert_eq!(
        claim.signature,
        "2e4e7c6667dadc42c48de0fca88e18c4050c415869f82cad80350b6108cdba72c90a342132df33e9d00d18c690cb6f1f8590e1aa8ec6a36a9e56c203bf64590d"
    );

    let upload = EvidenceUploadRequest::sign(
        organization_id,
        workspace_id,
        run_id,
        source_id,
        machine_id,
        task_id,
        9,
        "fedcba9876543210".into(),
        1_700_000_000_001,
        "application/zstd".into(),
        512,
        "ab".repeat(32),
        &signer,
    )
    .unwrap();
    assert_eq!(
        upload.evidence_id,
        "evidence:6e4f317f3faf7d2c15bd3723d85bd795418f4d21114d04f8eac2aa69a8e1f200"
    );
    assert_eq!(
        upload.signature,
        "9fc5530f70b0b25cae7ff98e2da3fc999a56a063bb62fe561ca8863182dae533c3c361ddd9d85c25ed296b76d36dd3342af383490155a3c72c8ff6b92f186904"
    );
    let commit = EvidenceCommitRequest::sign(
        organization_id,
        workspace_id,
        run_id,
        machine_id,
        task_id,
        9,
        upload.evidence_id.clone(),
        "0011223344556677".into(),
        1_700_000_000_002,
        &signer,
    )
    .unwrap();
    assert_eq!(
        commit.request_id,
        "evidence-commit:9ad628c1278412edcd5d768e8269e9353beb0291bf53c6e41f845f7cf789ea89"
    );
    assert_eq!(
        commit.signature,
        "2841fec2eb9d76a76e4e368a9434f872bc4ce484f2287d5ba8e7508486076e9304759595305d4ed80ff12ee7fb8a30687f5a2219aa29cf07ca4891edcedfa809"
    );

    let event = ObservationEvent::new(
        source_id,
        task_id,
        9,
        1,
        1_700_000_000_003,
        Classification::Internal,
        EvidenceObjectRef {
            evidence_id: upload.evidence_id,
            bucket: "clark-artifacts-test".into(),
            key: "system-cartography/v1/fixed/evidence.zst".into(),
            sha256: "ab".repeat(32),
            size_bytes: 512,
            version_id: Some("version-1".into()),
        },
        ObservationFact {
            subject: ObservationSubject::Entity {
                entity: EntityIdentity {
                    entity_kind: "runtime.service".into(),
                    provider_namespace: "aws".into(),
                    authority_scope: "aws:account/123/us-west-2".into(),
                    provider_native_id: "service/example".into(),
                },
            },
            attributes: json!({"environment_variable_names": ["DATABASE_URL"]}),
            evidence_digests: BTreeSet::from(["ab".repeat(32)]),
        },
    )
    .unwrap();
    assert_eq!(
        event.event_id,
        "event:9db40b9d625580452b13313710e2ff5985aa73a4734a0226471df8e81499ad5d"
    );
    let batch = BatchEnvelope::sign(
        organization_id,
        workspace_id,
        run_id,
        machine_id,
        "attempt-fixed".into(),
        1_700_000_000_004,
        vec![event],
        vec![TaskCompletion {
            task_id,
            fence: 9,
            disposition: TerminalDisposition::Supported,
            evidence_sha256: Some("ab".repeat(32)),
            detail: None,
        }],
        &signer,
    )
    .unwrap();
    assert_eq!(
        batch.batch_id,
        "batch:0b887b0cb55bc6a4649a9aecbb3079ca4f7bbaa39c1c0e407c35eebacb2bf441"
    );
    assert_eq!(
        batch.signature,
        "bf206d775c1ddcc58a520510538bdf44e84dcfaae46634ce219751fc3000556ae2e35fbfd55c06b0d2387ace006c920b863f88eee3b75ceb3097f6a2d3d0050b"
    );
}
