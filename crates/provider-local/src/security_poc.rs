use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{SecurityCandidate, SecurityDisposition};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityPocControl {
    Positive,
    Negative,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityPocOutcome {
    Reproduced,
    PartiallyReproduced,
    NotReproduced,
    Blocked,
    UnsafeToExecute,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityPocEvidence {
    pub goal: String,
    pub outcome: SecurityPocOutcome,
    #[serde(default)]
    pub positive_receipt_id: Option<String>,
    #[serde(default)]
    pub negative_receipt_id: Option<String>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityPocReceipt {
    pub contract_version: u32,
    pub receipt_id: String,
    pub scan_id: String,
    pub candidate_id: String,
    pub inventory_id: String,
    pub control: SecurityPocControl,
    pub language: String,
    pub script_sha256: String,
    pub expected_observation_sha256: String,
    pub workspace_sha256: String,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
    pub expected_exit_code: i32,
    pub exit_code: Option<i32>,
    pub passed: bool,
    pub containment: String,
    pub artifact_path: String,
    #[serde(default)]
    pub execution: Option<SecurityPocExecutionMetadata>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityPocExecutionMetadata {
    pub expected_observation: String,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub timeout_ms: u64,
    pub output_limit_bytes: u64,
    pub sandbox_provider: String,
    pub sandbox_profile_sha256: String,
    pub script_path: String,
    pub stdout_path: String,
    pub stderr_path: String,
}

#[derive(Clone, Debug, Default)]
pub struct SecurityPocLedger {
    receipts: BTreeMap<String, SecurityPocReceipt>,
}

impl SecurityPocLedger {
    pub fn record(&mut self, receipt: SecurityPocReceipt) -> Result<(), String> {
        require_id("receiptId", &receipt.receipt_id)?;
        match self.receipts.get(&receipt.receipt_id) {
            Some(existing) if existing == &receipt => Ok(()),
            Some(_) => Err(format!(
                "PoC receipt id `{}` was already issued for different evidence",
                receipt.receipt_id
            )),
            None => {
                self.receipts.insert(receipt.receipt_id.clone(), receipt);
                Ok(())
            }
        }
    }

    pub fn get(&self, receipt_id: &str) -> Option<&SecurityPocReceipt> {
        self.receipts.get(receipt_id)
    }

    pub(crate) fn validate_candidate(
        &self,
        scan_id: &str,
        inventory_id: &str,
        candidate: &SecurityCandidate,
    ) -> Result<(Option<&SecurityPocReceipt>, Option<&SecurityPocReceipt>), String> {
        require_text("poc.goal", &candidate.poc.goal)?;
        if candidate
            .poc
            .limitations
            .iter()
            .any(|limitation| limitation.trim().is_empty())
        {
            return Err(format!(
                "candidate `{}` has an empty PoC limitation",
                candidate.candidate_id
            ));
        }

        let positive = self.resolve(
            candidate.poc.positive_receipt_id.as_deref(),
            scan_id,
            inventory_id,
            candidate,
            SecurityPocControl::Positive,
        )?;
        let negative = self.resolve(
            candidate.poc.negative_receipt_id.as_deref(),
            scan_id,
            inventory_id,
            candidate,
            SecurityPocControl::Negative,
        )?;

        match candidate.poc.outcome {
            SecurityPocOutcome::Reproduced
            | SecurityPocOutcome::PartiallyReproduced
            | SecurityPocOutcome::NotReproduced => {
                let positive = positive.ok_or_else(|| {
                    format!(
                        "candidate `{}` PoC outcome requires a positive control receipt",
                        candidate.candidate_id
                    )
                })?;
                let negative = negative.ok_or_else(|| {
                    format!(
                        "candidate `{}` PoC outcome requires a negative control receipt",
                        candidate.candidate_id
                    )
                })?;
                if positive.receipt_id == negative.receipt_id
                    || positive.script_sha256 == negative.script_sha256
                {
                    return Err(format!(
                        "candidate `{}` PoC controls must be distinct",
                        candidate.candidate_id
                    ));
                }
                if !positive.passed || !negative.passed {
                    return Err(format!(
                        "candidate `{}` PoC requires passing positive and negative controls",
                        candidate.candidate_id
                    ));
                }
            }
            SecurityPocOutcome::Blocked | SecurityPocOutcome::UnsafeToExecute => {
                if candidate.poc.limitations.is_empty() {
                    return Err(format!(
                        "candidate `{}` blocked or unsafe PoC requires a limitation",
                        candidate.candidate_id
                    ));
                }
            }
        }

        let consistent = match candidate.validation.disposition {
            SecurityDisposition::Reportable => matches!(
                candidate.poc.outcome,
                SecurityPocOutcome::Reproduced | SecurityPocOutcome::PartiallyReproduced
            ),
            SecurityDisposition::Suppressed | SecurityDisposition::NotApplicable => {
                candidate.poc.outcome == SecurityPocOutcome::NotReproduced
            }
            SecurityDisposition::Deferred => matches!(
                candidate.poc.outcome,
                SecurityPocOutcome::NotReproduced
                    | SecurityPocOutcome::Blocked
                    | SecurityPocOutcome::UnsafeToExecute
            ),
        };
        if !consistent {
            return Err(format!(
                "candidate `{}` validation disposition and PoC outcome are inconsistent",
                candidate.candidate_id
            ));
        }

        Ok((positive, negative))
    }

    fn resolve<'a>(
        &'a self,
        receipt_id: Option<&str>,
        scan_id: &str,
        inventory_id: &str,
        candidate: &SecurityCandidate,
        control: SecurityPocControl,
    ) -> Result<Option<&'a SecurityPocReceipt>, String> {
        let Some(receipt_id) = receipt_id else {
            return Ok(None);
        };
        require_id("PoC receipt id", receipt_id)?;
        let receipt = self.receipts.get(receipt_id).ok_or_else(|| {
            format!(
                "candidate `{}` references unknown host-issued PoC receipt `{receipt_id}`",
                candidate.candidate_id
            )
        })?;
        if receipt.scan_id != scan_id
            || receipt.inventory_id != inventory_id
            || receipt.candidate_id != candidate.candidate_id
            || receipt.control != control
        {
            return Err(format!(
                "PoC receipt `{receipt_id}` does not match scan, inventory, candidate, and control"
            ));
        }
        if receipt.containment != "managed_disposable" {
            return Err(format!(
                "PoC receipt `{receipt_id}` was not produced in managed disposable containment"
            ));
        }
        Ok(Some(receipt))
    }
}

fn require_id(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Err(format!(
            "{name} must contain only letters, numbers, `.`, `_`, or `-`"
        ))
    } else {
        Ok(())
    }
}

fn require_text(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{name} must not be empty"))
    } else {
        Ok(())
    }
}
