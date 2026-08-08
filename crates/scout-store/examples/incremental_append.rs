use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseEntityKind, EnterpriseEvent, EnterpriseFact,
    EnterpriseGrantScope, EnterpriseId, EnterpriseProvenance, EnterpriseSignedBatch,
    EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseSigningKey, EnterpriseTrustChain,
    EnterpriseTrustManifest, GraphEntityObservation,
};
use scout_store::{request, ScoutStoreRequest, ScoutStoreResponse};

fn main() -> Result<(), String> {
    let arguments = Arguments::parse()?;
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let fixture = Fixture::new(root.path(), arguments.seed_batches as u64 + 1)?;
    for sequence in 1..=arguments.seed_batches as u64 {
        fixture.ingest_seed(sequence)?;
    }
    fixture.force_cold_rebuild()?;

    let cold_started = Instant::now();
    let cold = request(
        root.path(),
        ScoutStoreRequest::Rebuild {
            enterprise_id: fixture.enterprise.clone(),
        },
    )?;
    let cold_ms = cold_started.elapsed().as_millis();
    let ScoutStoreResponse::Rebuilt(cold_receipt) = cold else {
        return Err("benchmark cold rebuild returned the wrong response".into());
    };

    let append_started = Instant::now();
    let append = request(
        root.path(),
        ScoutStoreRequest::Ingest {
            enterprise_id: fixture.enterprise.clone(),
            envelope: Box::new(fixture.envelope(arguments.seed_batches as u64 + 1)?),
        },
    )?;
    let append_ms = append_started.elapsed().as_millis();
    let ScoutStoreResponse::Ingested {
        receipt: append_receipt,
        ..
    } = append
    else {
        return Err("benchmark append returned the wrong response".into());
    };
    let index_bytes = std::fs::metadata(root.path().join("index-v4.sqlite3"))
        .map_err(|error| error.to_string())?
        .len();
    let receipt = serde_json::json!({
        "schema_version": 1,
        "seed_batches": arguments.seed_batches,
        "seed_events": arguments.seed_batches,
        "cold_rebuild_ms": cold_ms,
        "incremental_append_ms": append_ms,
        "index_bytes": index_bytes,
        "cold_receipt": cold_receipt,
        "append_receipt": append_receipt,
        "assertions": {
            "no_prior_envelope_rows_read_on_append": append_receipt.ledger_authority_work.envelope_rows_read == 0,
            "no_prior_derived_batches_replayed": append_receipt.derived_batches_read == 0,
            "no_prior_events_replayed_for_a_new_key": append_receipt.events_replayed == 0,
            "one_affected_projection_row": append_receipt.affected_projection_rows == 1,
            "two_index_rows_written": append_receipt.projection_rows_written == 2,
            "no_full_projection_fallback": !append_receipt.full_projection_fallback,
            "event_root_advanced": append_receipt.event_root != cold_receipt.event_root,
        },
        "remaining_cost": "Every append still scans authenticated materialized entity/edge rows and cached event ids to preserve the exact v2 event root and graph digest. It no longer deserializes or replays prior event bodies for a new projection key."
    });
    let encoded = serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?;
    if let Some(output) = arguments.output {
        std::fs::write(output, &encoded).map_err(|error| error.to_string())?;
    }
    println!(
        "{}",
        String::from_utf8(encoded).map_err(|error| error.to_string())?
    );
    Ok(())
}

struct Arguments {
    seed_batches: usize,
    output: Option<PathBuf>,
}

impl Arguments {
    fn parse() -> Result<Self, String> {
        let mut values = std::env::args().skip(1);
        let seed_batches = values
            .next()
            .unwrap_or_else(|| "5000".into())
            .parse::<usize>()
            .map_err(|_| "usage: incremental_append [seed-batches] [receipt.json]".to_string())?;
        if seed_batches == 0 || seed_batches >= 100_000 {
            return Err("seed-batches must be in 1..100000".into());
        }
        Ok(Self {
            seed_batches,
            output: values.next().map(PathBuf::from),
        })
    }
}

struct Fixture {
    root: PathBuf,
    enterprise: EnterpriseId,
    manifest: EnterpriseTrustManifest,
    coordinator: EnterpriseSigningKey,
    grant: EnterpriseSignerGrant,
}

impl Fixture {
    fn new(root: &Path, last_sequence: u64) -> Result<Self, String> {
        let enterprise = EnterpriseId::new("incremental-store-benchmark")?;
        let coordinator = EnterpriseSigningKey::from_seed([0x31; 32]);
        let manifest = EnterpriseTrustManifest::initial(
            enterprise.clone(),
            "trust:00000000-0000-4000-8000-000000000031".into(),
            100,
            1_000_000,
            &coordinator,
        )?;
        let grant = EnterpriseSignerGrant::issue(
            &manifest,
            coordinator.signer_id(),
            coordinator.public_key_hex(),
            BTreeSet::from([EnterpriseSignerRole::Collector]),
            EnterpriseGrantScope {
                machine_id: "benchmark-machine".into(),
                run_id: "benchmark-run".into(),
                adapter_instance_id: "benchmark-adapter".into(),
                auth_context_id: "benchmark-auth".into(),
                discovery_epoch: "benchmark-epoch".into(),
                discovery_epoch_sequence: 1,
                first_source_sequence: 1,
                last_source_sequence: last_sequence,
            },
            100,
            1_000_000,
            &[&coordinator],
        )?;
        std::fs::create_dir_all(root.join("trust")).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(root.join("private")).map_err(|error| error.to_string())?;
        let chain = EnterpriseTrustChain {
            anchor_manifest_id: manifest.manifest_id.clone(),
            manifests: vec![manifest.clone()],
        };
        std::fs::write(
            root.join("trust/chain.json"),
            serde_json::to_vec(&chain).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        std::fs::write(
            root.join("private/anchor-manifest-id"),
            chain.anchor_manifest_id.as_bytes(),
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            root: root.to_path_buf(),
            enterprise,
            manifest,
            coordinator,
            grant,
        })
    }

    fn envelope(&self, sequence: u64) -> Result<EnterpriseSignedBatch, String> {
        let observation = GraphEntityObservation::new(
            &self.enterprise,
            EnterpriseEntityKind::CloudResource,
            AuthorityRef::new("benchmark", "global", format!("resource:{sequence:020}"))?,
            BTreeSet::from([format!("resource-{sequence}")]),
            BTreeSet::from([format!("{sequence:064x}")]),
        )?;
        let event = EnterpriseEvent::new(
            self.enterprise.clone(),
            EnterpriseProvenance {
                machine_id: "benchmark-machine".into(),
                run_id: "benchmark-run".into(),
                adapter_instance_id: "benchmark-adapter".into(),
                auth_context_id: "benchmark-auth".into(),
                discovery_epoch: "benchmark-epoch".into(),
                discovery_epoch_sequence: 1,
                source_sequence: sequence,
                observed_at_ms: 1_000 + sequence,
                source_fingerprint: format!("{:064x}", sequence + 1),
            },
            EnterpriseFact::EntityObserved(observation),
        )?;
        let batch = EnterpriseBatch::new(self.enterprise.clone(), [event])?;
        EnterpriseSignedBatch::sign(
            batch,
            &self.manifest,
            self.grant.clone(),
            10_000 + sequence,
            &self.coordinator,
        )
    }

    fn ingest_seed(&self, sequence: u64) -> Result<(), String> {
        let envelope = self.envelope(sequence)?;
        let response = request(
            &self.root,
            ScoutStoreRequest::Ingest {
                enterprise_id: self.enterprise.clone(),
                envelope: Box::new(envelope),
            },
        )?;
        matches!(response, ScoutStoreResponse::Ingested { .. })
            .then_some(())
            .ok_or_else(|| "benchmark seed ingest returned the wrong response".into())
    }

    fn force_cold_rebuild(&self) -> Result<(), String> {
        let connection = rusqlite::Connection::open(self.root.join("index-v4.sqlite3"))
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE meta SET value = 'incremental-benchmark-force-cold' \
                 WHERE key = 'projection_version'",
                [],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
