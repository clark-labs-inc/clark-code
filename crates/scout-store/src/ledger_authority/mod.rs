//! Authenticated transactional authority for the immutable Scout ledger.
//!
//! This module deliberately does not update the materialized graph or checkpoint
//! store. Callers must verify the enterprise trust chain before calling
//! [`LedgerAuthority::append_verified`].

mod accumulator;
mod append;
mod database;
mod recovery;
mod seal;

use std::path::{Path, PathBuf};

use agent_orchestration::{
    EnterpriseBatchId, EnterpriseId, EnterpriseLedgerCommitment, EnterpriseSignedBatch,
    VerifiedEnterpriseBatch,
};
use fs2::FileExt;
use scout_accumulator::PartitionedAccumulatorHead;
use serde::{Deserialize, Serialize};

pub const LEDGER_DATABASE_NAME: &str = "ledger-v1.sqlite3";
pub const LEDGER_AUTHORITY_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerHead {
    pub schema_version: u16,
    pub enterprise_id: EnterpriseId,
    pub generation: u64,
    pub head_id: String,
    pub previous_head_id: Option<String>,
    pub trust_chain_digest: String,
    pub batch_count: u64,
    pub event_count: u64,
    pub batch_accumulator: PartitionedAccumulatorHead,
    pub event_accumulator: PartitionedAccumulatorHead,
}

impl LedgerHead {
    pub fn ledger_commitment(&self) -> Result<EnterpriseLedgerCommitment, String> {
        if self.generation == 0 {
            return Err("an empty Scout ledger has no checkpoint commitment".into());
        }
        EnterpriseLedgerCommitment::new(
            &self.enterprise_id,
            self.generation,
            accumulator::root_id("scout-batch-set-v1", &self.batch_accumulator),
            accumulator::root_id("scout-event-set-v1", &self.event_accumulator),
            self.batch_count,
            self.event_count,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerAuthorityWork {
    pub head_rows_read: usize,
    pub history_rows_read: usize,
    pub batch_lookups: usize,
    pub event_lookups: usize,
    pub accumulator_node_lookups: usize,
    pub accumulator_nodes_written: usize,
    pub accumulator_nodes_deleted: usize,
    pub envelope_rows_read: usize,
    pub envelope_bytes_read: usize,
    pub batch_rows_written: usize,
    pub event_rows_written: usize,
    pub head_rows_written: usize,
}

impl LedgerAuthorityWork {
    pub fn merge(&mut self, other: Self) {
        self.head_rows_read = self.head_rows_read.saturating_add(other.head_rows_read);
        self.history_rows_read = self
            .history_rows_read
            .saturating_add(other.history_rows_read);
        self.batch_lookups = self.batch_lookups.saturating_add(other.batch_lookups);
        self.event_lookups = self.event_lookups.saturating_add(other.event_lookups);
        self.accumulator_node_lookups = self
            .accumulator_node_lookups
            .saturating_add(other.accumulator_node_lookups);
        self.accumulator_nodes_written = self
            .accumulator_nodes_written
            .saturating_add(other.accumulator_nodes_written);
        self.accumulator_nodes_deleted = self
            .accumulator_nodes_deleted
            .saturating_add(other.accumulator_nodes_deleted);
        self.envelope_rows_read = self
            .envelope_rows_read
            .saturating_add(other.envelope_rows_read);
        self.envelope_bytes_read = self
            .envelope_bytes_read
            .saturating_add(other.envelope_bytes_read);
        self.batch_rows_written = self
            .batch_rows_written
            .saturating_add(other.batch_rows_written);
        self.event_rows_written = self
            .event_rows_written
            .saturating_add(other.event_rows_written);
        self.head_rows_written = self
            .head_rows_written
            .saturating_add(other.head_rows_written);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerHeadRead {
    pub head: LedgerHead,
    pub work: LedgerAuthorityWork,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerHeadAtGenerationRead {
    Found(Box<LedgerHeadRead>),
    Missing {
        requested_generation: u64,
        current_generation: u64,
        work: LedgerAuthorityWork,
    },
    Pruned {
        requested_generation: u64,
        oldest_available_generation: u64,
        work: LedgerAuthorityWork,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgerAppendOutcome {
    Inserted,
    AlreadyPresent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerAppendReceipt {
    pub outcome: LedgerAppendOutcome,
    pub head: LedgerHead,
    pub work: LedgerAuthorityWork,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerEnvelopeRead {
    pub envelope: Option<EnterpriseSignedBatch>,
    pub work: LedgerAuthorityWork,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerGenerationEnvelope {
    pub generation: u64,
    pub envelope: EnterpriseSignedBatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerEnvelopeRangeRead {
    pub envelopes: Vec<LedgerGenerationEnvelope>,
    pub work: LedgerAuthorityWork,
}

#[derive(Clone)]
pub struct LedgerAuthority {
    root: PathBuf,
    enterprise_id: EnterpriseId,
    trust_chain_digest: String,
    auth_key: [u8; database::AUTH_KEY_BYTES],
}

pub struct LedgerAuthorityOpen {
    pub authority: LedgerAuthority,
    pub head: LedgerHead,
    pub work: LedgerAuthorityWork,
}

impl LedgerAuthority {
    pub fn open(
        root: impl AsRef<Path>,
        enterprise_id: EnterpriseId,
        trust_chain_digest: impl Into<String>,
    ) -> Result<Self, String> {
        Ok(Self::open_with_head(root, enterprise_id, trust_chain_digest)?.authority)
    }

    pub fn open_with_head(
        root: impl AsRef<Path>,
        enterprise_id: EnterpriseId,
        trust_chain_digest: impl Into<String>,
    ) -> Result<LedgerAuthorityOpen, String> {
        let root = root.as_ref().to_path_buf();
        let trust_chain_digest = trust_chain_digest.into();
        database::validate_hex_digest("trust chain", &trust_chain_digest)?;
        database::prepare_root(&root)?;
        let lock = database::open_lock(&root)?;
        FileExt::lock_exclusive(&lock).map_err(|error| error.to_string())?;
        let auth_key = database::load_or_create_auth_key(&root)?;
        let database_existed = seal::database_exists(&root)?;
        let expected_seal = if database_existed {
            let sealed = seal::read_authenticated(&root, &auth_key, &enterprise_id)?;
            if !seal::matches_current(&root, &sealed)? {
                recovery::recover_exact_successor(
                    &root,
                    &auth_key,
                    &enterprise_id,
                    &trust_chain_digest,
                    &sealed,
                )?;
            }
            Some(seal::validate(&root, &auth_key, &enterprise_id)?)
        } else {
            None
        };
        let mut connection = database::open_connection(&root)?;
        if let Some(expected) = &expected_seal {
            seal::validate_unchanged(&root, &auth_key, &enterprise_id, expected)?;
        }
        let (head, work) = database::initialize(
            &mut connection,
            &auth_key,
            &enterprise_id,
            &trust_chain_digest,
        )?;
        if let Some(expected) = expected_seal {
            seal::require_head(&expected, &head)?;
        } else {
            seal::write(&root, &auth_key, &head)?;
        }
        Ok(LedgerAuthorityOpen {
            authority: Self {
                root,
                enterprise_id,
                trust_chain_digest,
                auth_key,
            },
            head,
            work,
        })
    }

    pub fn read_head(&self) -> Result<LedgerHeadRead, String> {
        let lock = database::open_lock(&self.root)?;
        FileExt::lock_shared(&lock).map_err(|error| error.to_string())?;
        let sealed = seal::validate(&self.root, &self.auth_key, &self.enterprise_id)?;
        let connection = database::open_connection(&self.root)?;
        seal::validate_unchanged(&self.root, &self.auth_key, &self.enterprise_id, &sealed)?;
        let mut work = LedgerAuthorityWork::default();
        let head = database::read_head(
            &connection,
            &self.auth_key,
            &self.enterprise_id,
            &self.trust_chain_digest,
            &mut work,
        )?;
        seal::require_head(&sealed, &head)?;
        Ok(LedgerHeadRead { head, work })
    }

    pub fn read_head_at_generation(
        &self,
        generation: u64,
    ) -> Result<LedgerHeadAtGenerationRead, String> {
        if generation == 0 {
            return Err("Scout ledger checkpoint generations start at one".into());
        }
        let lock = database::open_lock(&self.root)?;
        FileExt::lock_shared(&lock).map_err(|error| error.to_string())?;
        let sealed = seal::validate(&self.root, &self.auth_key, &self.enterprise_id)?;
        let connection = database::open_connection(&self.root)?;
        seal::validate_unchanged(&self.root, &self.auth_key, &self.enterprise_id, &sealed)?;
        let mut work = LedgerAuthorityWork::default();
        let current = database::read_head(
            &connection,
            &self.auth_key,
            &self.enterprise_id,
            &self.trust_chain_digest,
            &mut work,
        )?;
        seal::require_head(&sealed, &current)?;
        if generation == current.generation {
            return Ok(LedgerHeadAtGenerationRead::Found(Box::new(
                LedgerHeadRead {
                    head: current,
                    work,
                },
            )));
        }
        if generation > current.generation {
            return Ok(LedgerHeadAtGenerationRead::Missing {
                requested_generation: generation,
                current_generation: current.generation,
                work,
            });
        }
        let oldest_available_generation = current.generation.saturating_sub(1);
        if generation < oldest_available_generation {
            return Ok(LedgerHeadAtGenerationRead::Pruned {
                requested_generation: generation,
                oldest_available_generation,
                work,
            });
        }
        let heads = database::read_head_history_generation(
            &connection,
            &self.auth_key,
            &self.enterprise_id,
            &self.trust_chain_digest,
            generation,
        )?;
        work.history_rows_read = work.history_rows_read.saturating_add(heads.len());
        let [previous] = heads.as_slice() else {
            return match heads.len() {
                0 => Ok(LedgerHeadAtGenerationRead::Missing {
                    requested_generation: generation,
                    current_generation: current.generation,
                    work,
                }),
                _ => Err("Scout ledger history contains a forked generation".into()),
            };
        };
        if current.previous_head_id.as_deref() != Some(previous.head_id.as_str())
            || previous.generation != oldest_available_generation
            || previous.batch_count.saturating_add(1) != current.batch_count
        {
            return Err("Scout ledger history does not directly precede the current head".into());
        }
        Ok(LedgerHeadAtGenerationRead::Found(Box::new(
            LedgerHeadRead {
                head: previous.clone(),
                work,
            },
        )))
    }

    pub fn read_envelope(
        &self,
        batch_id: &EnterpriseBatchId,
    ) -> Result<LedgerEnvelopeRead, String> {
        let lock = database::open_lock(&self.root)?;
        FileExt::lock_shared(&lock).map_err(|error| error.to_string())?;
        let sealed = seal::validate(&self.root, &self.auth_key, &self.enterprise_id)?;
        let connection = database::open_connection(&self.root)?;
        seal::validate_unchanged(&self.root, &self.auth_key, &self.enterprise_id, &sealed)?;
        let mut work = LedgerAuthorityWork::default();
        let head = database::read_head(
            &connection,
            &self.auth_key,
            &self.enterprise_id,
            &self.trust_chain_digest,
            &mut work,
        )?;
        seal::require_head(&sealed, &head)?;
        let row = database::read_batch(
            &connection,
            &self.auth_key,
            &self.enterprise_id,
            batch_id.as_str(),
            &mut work,
        )?;
        let envelope = if let Some(row) = row {
            if row.generation > head.generation {
                return Err("ledger batch generation is ahead of the authenticated head".into());
            }
            let envelope = row.decode()?;
            if envelope.batch.enterprise_id != self.enterprise_id
                || envelope.batch.batch_id != *batch_id
            {
                return Err("authenticated ledger batch row has inconsistent identity".into());
            }
            Some(envelope)
        } else {
            None
        };
        Ok(LedgerEnvelopeRead { envelope, work })
    }

    pub fn read_envelope_range(
        &self,
        first_generation: u64,
        last_generation: u64,
    ) -> Result<LedgerEnvelopeRangeRead, String> {
        if first_generation == 0 {
            return Err("Scout ledger generations start at one".into());
        }
        let lock = database::open_lock(&self.root)?;
        FileExt::lock_shared(&lock).map_err(|error| error.to_string())?;
        let sealed = seal::validate(&self.root, &self.auth_key, &self.enterprise_id)?;
        let connection = database::open_connection(&self.root)?;
        seal::validate_unchanged(&self.root, &self.auth_key, &self.enterprise_id, &sealed)?;
        let mut work = LedgerAuthorityWork::default();
        let head = database::read_head(
            &connection,
            &self.auth_key,
            &self.enterprise_id,
            &self.trust_chain_digest,
            &mut work,
        )?;
        seal::require_head(&sealed, &head)?;
        if last_generation > head.generation {
            return Err("Scout ledger range exceeds the authenticated head".into());
        }
        let envelopes = database::read_batch_range(
            &connection,
            &self.auth_key,
            &self.enterprise_id,
            first_generation,
            last_generation,
            &mut work,
        )?;
        Ok(LedgerEnvelopeRangeRead { envelopes, work })
    }

    pub fn read_all_envelopes(&self) -> Result<LedgerEnvelopeRangeRead, String> {
        let lock = database::open_lock(&self.root)?;
        FileExt::lock_shared(&lock).map_err(|error| error.to_string())?;
        let sealed = seal::validate(&self.root, &self.auth_key, &self.enterprise_id)?;
        let connection = database::open_connection(&self.root)?;
        seal::validate_unchanged(&self.root, &self.auth_key, &self.enterprise_id, &sealed)?;
        let mut work = LedgerAuthorityWork::default();
        let head = database::read_head(
            &connection,
            &self.auth_key,
            &self.enterprise_id,
            &self.trust_chain_digest,
            &mut work,
        )?;
        seal::require_head(&sealed, &head)?;
        if head.generation == 0 {
            return Ok(LedgerEnvelopeRangeRead {
                envelopes: Vec::new(),
                work,
            });
        }
        let envelopes = database::read_batch_range(
            &connection,
            &self.auth_key,
            &self.enterprise_id,
            1,
            head.generation,
            &mut work,
        )?;
        Ok(LedgerEnvelopeRangeRead { envelopes, work })
    }

    pub fn append_verified(
        &self,
        verified: &VerifiedEnterpriseBatch,
    ) -> Result<LedgerAppendReceipt, String> {
        append::append(self, verified, append::AppendFailpoint::None)
    }
}

#[cfg(test)]
mod tests;
