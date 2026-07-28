use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize)]
pub struct CaseReceipt {
    pub id: String,
    pub status: &'static str,
    pub outcome: String,
    pub duration_ms: u128,
    pub evidence: Value,
}

#[derive(Debug, Serialize)]
pub struct CapabilityReceipt {
    pub platform: String,
    pub architecture: String,
    pub executable_count: usize,
    pub environment_name_count: usize,
    pub credential_surfaces: Vec<String>,
    pub known_tools: Value,
    pub values_observed: bool,
    pub executables_truncated: bool,
    pub environment_names_truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct BenchmarkReceipt {
    pub schema_version: u32,
    pub benchmark: &'static str,
    pub status: &'static str,
    pub host_label: String,
    pub started_at_epoch_ms: u128,
    pub duration_ms: u128,
    pub live_model_calls: u32,
    pub capability_census: CapabilityReceipt,
    pub containment: String,
    pub canonical_sha256: String,
    pub cases: Vec<CaseReceipt>,
}

pub struct Recorder {
    started: Instant,
    started_at_epoch_ms: u128,
    cases: Vec<CaseReceipt>,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            started_at_epoch_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            cases: Vec::new(),
        }
    }

    pub fn case(&mut self, id: &str, run: impl FnOnce() -> Result<(String, Value), String>) {
        let started = Instant::now();
        let (status, outcome, evidence) = match run() {
            Ok((outcome, evidence)) => ("passed", outcome, evidence),
            Err(error) => ("failed", error, Value::Null),
        };
        println!("{status:6} {id}");
        self.cases.push(CaseReceipt {
            id: id.to_string(),
            status,
            outcome,
            duration_ms: started.elapsed().as_millis(),
            evidence,
        });
    }

    pub fn passed(&self) -> bool {
        self.cases.iter().all(|case| case.status == "passed")
    }

    pub fn finish(
        self,
        host_label: String,
        capabilities: CapabilityReceipt,
        containment: String,
    ) -> BenchmarkReceipt {
        let canonical_sha256 = canonical_hash(&self.cases);
        BenchmarkReceipt {
            schema_version: 1,
            benchmark: "clark_scout_offline_v1",
            status: if self.passed() { "passed" } else { "failed" },
            host_label,
            started_at_epoch_ms: self.started_at_epoch_ms,
            duration_ms: self.started.elapsed().as_millis(),
            live_model_calls: 0,
            capability_census: capabilities,
            containment,
            canonical_sha256,
            cases: self.cases,
        }
    }
}

fn canonical_hash(cases: &[CaseReceipt]) -> String {
    let canonical = cases
        .iter()
        .map(|case| {
            (
                &case.id,
                case.status,
                case.evidence.get("semantic_sha256").and_then(Value::as_str),
            )
        })
        .collect::<Vec<_>>();
    let encoded = serde_json::to_vec(&canonical).unwrap_or_default();
    format!("{:x}", Sha256::digest(encoded))
}

pub fn write_artifacts(output: &Path, receipt: &BenchmarkReceipt) -> Result<(), String> {
    let receipt_json = serde_json::to_vec_pretty(receipt).map_err(|error| error.to_string())?;
    std::fs::write(output.join("receipt.json"), receipt_json).map_err(|error| error.to_string())?;

    let mut report = format!(
        "# Scout offline benchmark\n\n- Status: `{}`\n- Host label: `{}`\n- Platform: `{}/{}`\n- Containment: `{}`\n- Live model calls: `0`\n- Canonical SHA-256: `{}`\n\n## Cases\n\n",
        receipt.status,
        receipt.host_label,
        receipt.capability_census.platform,
        receipt.capability_census.architecture,
        receipt.containment,
        receipt.canonical_sha256,
    );
    for case in &receipt.cases {
        report.push_str(&format!(
            "- `{}` **{}** — {}\n",
            case.id,
            case.status.to_ascii_uppercase(),
            case.outcome
        ));
    }
    report.push_str(
        "\nThe canonical hash covers ordered case ids, pass/fail states, and \
         deterministic per-case semantic hashes when present. Host identity, paths, \
         timestamps, timing, and capability counts are excluded.\n",
    );
    std::fs::write(output.join("report.md"), report).map_err(|error| error.to_string())
}
