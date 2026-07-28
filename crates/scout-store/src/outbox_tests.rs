use std::collections::BTreeSet;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseBatchId, EnterpriseEntityKind, EnterpriseEvent,
    EnterpriseFact, EnterpriseGrantScope, EnterpriseId, EnterpriseProvenance,
    EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseSigningKey,
    EnterpriseTrustChain, EnterpriseTrustManifest, GraphEntityObservation,
};

use crate::{
    dispatch, OutboxEntry, OutboxPage, OutboxResolution, OutboxState, OutboxStateFilter,
    ScoutStoreRequest, ScoutStoreResponse, SERVICE_NAME,
};

struct Fixture {
    enterprise_id: EnterpriseId,
    root: tempfile::TempDir,
    envelope: EnterpriseSignedBatch,
}

impl Fixture {
    fn new() -> Self {
        let enterprise_id = EnterpriseId::new("outbox-enterprise").unwrap();
        let coordinator = EnterpriseSigningKey::from_seed([0x51; 32]);
        let manifest = EnterpriseTrustManifest::initial(
            enterprise_id.clone(),
            "trust:00000000-0000-4000-8000-000000000051".into(),
            100,
            100_000,
            &coordinator,
        )
        .unwrap();
        let chain = EnterpriseTrustChain {
            anchor_manifest_id: manifest.manifest_id.clone(),
            manifests: vec![manifest.clone()],
        };
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("trust")).unwrap();
        std::fs::create_dir_all(root.path().join("private")).unwrap();
        std::fs::write(
            root.path().join("trust/chain.json"),
            serde_json::to_vec(&chain).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.path().join("private/anchor-manifest-id"),
            chain.anchor_manifest_id.as_bytes(),
        )
        .unwrap();

        let observation = GraphEntityObservation::new(
            &enterprise_id,
            EnterpriseEntityKind::Service,
            AuthorityRef::new("fixture", "tenant:fixture", "service:outbox").unwrap(),
            BTreeSet::from(["outbox-service".into()]),
            BTreeSet::from(["a".repeat(64)]),
        )
        .unwrap();
        let event = EnterpriseEvent::new(
            enterprise_id.clone(),
            EnterpriseProvenance {
                machine_id: "machine-outbox".into(),
                run_id: "run-outbox".into(),
                adapter_instance_id: "fixture-adapter".into(),
                auth_context_id: "fixture-auth".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                source_sequence: 1,
                observed_at_ms: 1_000,
                source_fingerprint: "f".repeat(64),
            },
            EnterpriseFact::EntityObserved(observation),
        )
        .unwrap();
        let batch = EnterpriseBatch::new(enterprise_id.clone(), [event]).unwrap();
        let grant = EnterpriseSignerGrant::issue(
            &manifest,
            coordinator.signer_id(),
            coordinator.public_key_hex(),
            BTreeSet::from([
                EnterpriseSignerRole::Collector,
                EnterpriseSignerRole::Coordinator,
            ]),
            EnterpriseGrantScope {
                machine_id: "machine-outbox".into(),
                run_id: "run-outbox".into(),
                adapter_instance_id: "fixture-adapter".into(),
                auth_context_id: "fixture-auth".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                first_source_sequence: 1,
                last_source_sequence: 1,
            },
            100,
            100_000,
            &[&coordinator],
        )
        .unwrap();
        let envelope =
            EnterpriseSignedBatch::sign(batch, &manifest, grant, 1_000, &coordinator).unwrap();
        let fixture = Self {
            enterprise_id,
            root,
            envelope,
        };
        fixture
            .call(ScoutStoreRequest::Ingest {
                enterprise_id: fixture.enterprise_id.clone(),
                envelope: Box::new(fixture.envelope.clone()),
            })
            .unwrap();
        fixture
    }

    fn batch_id(&self) -> EnterpriseBatchId {
        self.envelope.batch.batch_id.clone()
    }

    fn call(&self, request: ScoutStoreRequest) -> Result<ScoutStoreResponse, String> {
        let response = dispatch(
            SERVICE_NAME,
            self.root.path(),
            &serde_json::to_vec(&request).unwrap(),
        )?;
        serde_json::from_slice(&response).map_err(|error| error.to_string())
    }

    fn enqueue(&self) -> Result<(OutboxEntry, bool), String> {
        updated(self.call(ScoutStoreRequest::EnqueueOutbox {
            enterprise_id: self.enterprise_id.clone(),
            batch_id: self.batch_id(),
        })?)
    }

    fn begin(
        &self,
        attempt_id: &str,
        previous_attempt_id: Option<&str>,
    ) -> Result<(OutboxEntry, bool), String> {
        updated(self.call(ScoutStoreRequest::BeginOutboxDelivery {
            enterprise_id: self.enterprise_id.clone(),
            batch_id: self.batch_id(),
            attempt_id: attempt_id.into(),
            previous_attempt_id: previous_attempt_id.map(str::to_owned),
        })?)
    }

    fn resolve(
        &self,
        attempt_id: &str,
        resolution: OutboxResolution,
        resolution_id: &str,
    ) -> Result<(OutboxEntry, bool), String> {
        updated(self.call(ScoutStoreRequest::ResolveOutboxDelivery {
            enterprise_id: self.enterprise_id.clone(),
            batch_id: self.batch_id(),
            attempt_id: attempt_id.into(),
            resolution,
            resolution_id: resolution_id.into(),
        })?)
    }

    fn outbox_directory(&self) -> std::path::PathBuf {
        let directory = self.root.path().join("private/central-ingestion-outbox");
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn seed_entry(
        &self,
        enterprise_id: EnterpriseId,
        digit: char,
        state: OutboxState,
    ) -> OutboxEntry {
        let batch_id = EnterpriseBatchId::new(reference("batch:", digit)).unwrap();
        let entry = OutboxEntry {
            enterprise_id,
            batch_id: batch_id.clone(),
            revision: 1,
            state,
        };
        let digest = batch_id.as_str().strip_prefix("batch:").unwrap();
        std::fs::write(
            self.outbox_directory().join(format!("{digest}.json")),
            serde_json::to_vec(&entry).unwrap(),
        )
        .unwrap();
        entry
    }

    fn list(
        &self,
        enterprise_id: EnterpriseId,
        filter: OutboxStateFilter,
        cursor: Option<String>,
        limit: usize,
    ) -> Result<OutboxPage, String> {
        listed(self.call(ScoutStoreRequest::ListOutbox {
            enterprise_id,
            filter,
            cursor,
            limit,
        })?)
    }
}

fn updated(response: ScoutStoreResponse) -> Result<(OutboxEntry, bool), String> {
    match response {
        ScoutStoreResponse::OutboxUpdated { entry, idempotent } => Ok((entry, idempotent)),
        _ => Err("wrong outbox response kind".into()),
    }
}

fn listed(response: ScoutStoreResponse) -> Result<OutboxPage, String> {
    match response {
        ScoutStoreResponse::OutboxListed { page } => Ok(page),
        _ => Err("wrong outbox list response kind".into()),
    }
}

fn reference(prefix: &str, digit: char) -> String {
    format!("{prefix}{}", digit.to_string().repeat(64))
}

#[test]
fn outbox_transitions_are_retry_safe_and_conflicting_ack_fails_closed() {
    let fixture = Fixture::new();
    let attempt_one = reference("outbox-attempt:", '1');
    let attempt_two = reference("outbox-attempt:", '2');
    let acknowledgment = reference("central-ingestion:", 'a');

    let (pending, idempotent) = fixture.enqueue().unwrap();
    assert!(!idempotent);
    assert_eq!(pending.revision, 1);
    assert_eq!(pending.state, OutboxState::Pending);
    assert!(fixture.enqueue().unwrap().1);

    let (first, idempotent) = fixture.begin(&attempt_one, None).unwrap();
    assert!(!idempotent);
    assert_eq!(first.revision, 2);
    assert_eq!(
        first.state,
        OutboxState::InFlight {
            attempt_id: attempt_one.clone()
        }
    );
    assert!(fixture.begin(&attempt_one, None).unwrap().1);

    let (replacement, idempotent) = fixture.begin(&attempt_two, Some(&attempt_one)).unwrap();
    assert!(!idempotent);
    assert_eq!(replacement.revision, 3);
    assert!(fixture
        .resolve(&attempt_one, OutboxResolution::Acked, &acknowledgment)
        .unwrap_err()
        .contains("stale"));

    let (acked, idempotent) = fixture
        .resolve(&attempt_two, OutboxResolution::Acked, &acknowledgment)
        .unwrap();
    assert!(!idempotent);
    assert_eq!(acked.revision, 4);
    assert!(
        fixture
            .resolve(&attempt_two, OutboxResolution::Acked, &acknowledgment)
            .unwrap()
            .1
    );
    let conflicting = reference("central-ingestion:", 'b');
    let error = fixture
        .resolve(&attempt_two, OutboxResolution::Acked, &conflicting)
        .unwrap_err();
    assert!(error.contains("conflicting central ingestion acknowledgment"));
    let error = fixture
        .resolve(&attempt_two, OutboxResolution::Rejected, &acknowledgment)
        .unwrap_err();
    assert!(error.contains("conflicting central ingestion acknowledgment"));
    let status = fixture
        .call(ScoutStoreRequest::OutboxStatus {
            enterprise_id: fixture.enterprise_id.clone(),
            batch_id: fixture.batch_id(),
        })
        .unwrap();
    let ScoutStoreResponse::OutboxStatus { entry } = status else {
        panic!("wrong outbox status response");
    };
    assert_eq!(entry, Some(acked));
}

#[test]
fn interrupted_state_write_recovers_and_rejected_delivery_is_durable() {
    let fixture = Fixture::new();
    let attempt = reference("outbox-attempt:", '3');
    let rejection = reference("central-ingestion:", 'c');
    let pending = fixture.enqueue().unwrap().0;
    let digest = fixture
        .batch_id()
        .as_str()
        .strip_prefix("batch:")
        .unwrap()
        .to_owned();
    let directory = fixture.root.path().join("private/central-ingestion-outbox");
    let temporary = directory.join(format!(".{digest}.state.pending"));
    let interrupted = OutboxEntry {
        revision: 2,
        state: OutboxState::InFlight {
            attempt_id: attempt.clone(),
        },
        ..pending
    };
    std::fs::write(&temporary, serde_json::to_vec(&interrupted).unwrap()).unwrap();

    let (in_flight, idempotent) = fixture.begin(&attempt, None).unwrap();
    assert!(!idempotent);
    assert_eq!(in_flight, interrupted);
    assert!(!temporary.exists());
    assert!(fixture.begin(&attempt, None).unwrap().1);

    let (rejected, idempotent) = fixture
        .resolve(&attempt, OutboxResolution::Rejected, &rejection)
        .unwrap();
    assert!(!idempotent);
    assert_eq!(
        rejected.state,
        OutboxState::Rejected {
            attempt_id: attempt,
            resolution_id: rejection
        }
    );
    let state_path = directory.join(format!("{digest}.json"));
    let bytes = std::fs::read(state_path).unwrap();
    assert!(!bytes.windows(b"events".len()).any(|item| item == b"events"));
    assert!(!bytes.windows(b"secret".len()).any(|item| item == b"secret"));
}

#[test]
fn outbox_enumeration_filters_enterprise_and_drainable_state_in_batch_order() {
    let fixture = Fixture::new();
    let attempt = reference("outbox-attempt:", 'a');
    let resolution = reference("central-ingestion:", 'b');
    let other_enterprise = EnterpriseId::new("other-outbox-enterprise").unwrap();

    let pending_four = fixture.seed_entry(fixture.enterprise_id.clone(), '4', OutboxState::Pending);
    fixture.seed_entry(other_enterprise, '0', OutboxState::Pending);
    fixture.seed_entry(
        fixture.enterprise_id.clone(),
        '5',
        OutboxState::Rejected {
            attempt_id: attempt.clone(),
            resolution_id: resolution.clone(),
        },
    );
    let in_flight = fixture.seed_entry(
        fixture.enterprise_id.clone(),
        '2',
        OutboxState::InFlight {
            attempt_id: attempt.clone(),
        },
    );
    fixture.seed_entry(
        fixture.enterprise_id.clone(),
        '3',
        OutboxState::Acked {
            attempt_id: attempt.clone(),
            resolution_id: resolution.clone(),
        },
    );
    let pending_one = fixture.seed_entry(fixture.enterprise_id.clone(), '1', OutboxState::Pending);
    std::fs::write(
        fixture
            .outbox_directory()
            .join(format!(".{}.state.pending", "6".repeat(64))),
        b"interrupted",
    )
    .unwrap();

    let drainable = fixture
        .list(
            fixture.enterprise_id.clone(),
            OutboxStateFilter::PendingOrInFlight,
            None,
            10,
        )
        .unwrap();
    assert_eq!(
        drainable.entries,
        vec![pending_one.clone(), in_flight.clone(), pending_four.clone()]
    );
    assert!(drainable.next_cursor.is_none());

    let pending = fixture
        .list(
            fixture.enterprise_id.clone(),
            OutboxStateFilter::Pending,
            None,
            10,
        )
        .unwrap();
    assert_eq!(pending.entries, vec![pending_one, pending_four]);

    let in_flight_page = fixture
        .list(
            fixture.enterprise_id.clone(),
            OutboxStateFilter::InFlight,
            None,
            10,
        )
        .unwrap();
    assert_eq!(in_flight_page.entries, vec![in_flight]);
}

#[test]
fn outbox_enumeration_paginates_and_binds_cursor_to_enterprise_and_filter() {
    let fixture = Fixture::new();
    let entries = ('1'..='5')
        .map(|digit| fixture.seed_entry(fixture.enterprise_id.clone(), digit, OutboxState::Pending))
        .collect::<Vec<_>>();

    let first = fixture
        .list(
            fixture.enterprise_id.clone(),
            OutboxStateFilter::Pending,
            None,
            2,
        )
        .unwrap();
    assert_eq!(first.entries, entries[..2]);
    let first_cursor = first.next_cursor.clone().expect("first cursor");

    let second = fixture
        .list(
            fixture.enterprise_id.clone(),
            OutboxStateFilter::Pending,
            first.next_cursor,
            2,
        )
        .unwrap();
    assert_eq!(second.entries, entries[2..4]);
    let third = fixture
        .list(
            fixture.enterprise_id.clone(),
            OutboxStateFilter::Pending,
            second.next_cursor,
            2,
        )
        .unwrap();
    assert_eq!(third.entries, entries[4..]);
    assert!(third.next_cursor.is_none());

    let filter_error = fixture
        .list(
            fixture.enterprise_id.clone(),
            OutboxStateFilter::InFlight,
            Some(first_cursor.clone()),
            2,
        )
        .unwrap_err();
    assert!(filter_error.contains("mismatched"));
    let enterprise_error = fixture
        .list(
            EnterpriseId::new("other-outbox-enterprise").unwrap(),
            OutboxStateFilter::Pending,
            Some(first_cursor),
            2,
        )
        .unwrap_err();
    assert!(enterprise_error.contains("mismatched"));
    assert!(fixture
        .list(
            fixture.enterprise_id.clone(),
            OutboxStateFilter::Pending,
            None,
            0,
        )
        .unwrap_err()
        .contains("1..=1000"));
    assert!(fixture
        .list(
            fixture.enterprise_id.clone(),
            OutboxStateFilter::Pending,
            None,
            1_001,
        )
        .unwrap_err()
        .contains("1..=1000"));
}

#[test]
fn outbox_enumeration_fails_closed_on_corrupt_state() {
    let fixture = Fixture::new();
    fixture.seed_entry(fixture.enterprise_id.clone(), '1', OutboxState::Pending);
    let corrupt = fixture
        .outbox_directory()
        .join(format!("{}.json", "2".repeat(64)));
    std::fs::write(&corrupt, b"{").unwrap();

    let error = fixture
        .list(
            fixture.enterprise_id.clone(),
            OutboxStateFilter::PendingOrInFlight,
            None,
            10,
        )
        .unwrap_err();
    assert!(error.contains("invalid central ingestion outbox state"));
}

#[cfg(unix)]
#[test]
fn outbox_enumeration_refuses_symlink_state_without_following_it() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let entry = OutboxEntry {
        enterprise_id: fixture.enterprise_id.clone(),
        batch_id: EnterpriseBatchId::new(reference("batch:", '3')).unwrap(),
        revision: 1,
        state: OutboxState::Pending,
    };
    let target = fixture.root.path().join("outside-outbox-state.json");
    std::fs::write(&target, serde_json::to_vec(&entry).unwrap()).unwrap();
    let digest = entry.batch_id.as_str().strip_prefix("batch:").unwrap();
    symlink(
        &target,
        fixture.outbox_directory().join(format!("{digest}.json")),
    )
    .unwrap();

    let error = fixture
        .list(
            fixture.enterprise_id.clone(),
            OutboxStateFilter::Pending,
            None,
            10,
        )
        .unwrap_err();
    assert!(error.contains("symlink"));
}
