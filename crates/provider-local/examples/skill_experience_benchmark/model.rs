use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

pub type DynError = Box<dyn std::error::Error + Send + Sync>;
pub type Evidence = BTreeMap<String, Value>;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum StepStatus {
    Passed,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepReceipt {
    id: String,
    title: String,
    status: StepStatus,
    duration_ms: u128,
    evidence: Evidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkReceipt {
    schema_version: u32,
    benchmark: &'static str,
    status: &'static str,
    started_at_epoch_ms: u128,
    duration_ms: u128,
    source: String,
    source_digest: String,
    workspace: String,
    live_model_calls: u32,
    steps: Vec<StepReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<String>,
}

pub struct Recorder {
    started: Instant,
    started_at_epoch_ms: u128,
    output: String,
    source: String,
    source_digest: String,
    steps: Vec<StepReceipt>,
}

impl Recorder {
    pub fn new(output: &Path, source: &Path, source_digest: &str) -> Self {
        Self {
            started: Instant::now(),
            started_at_epoch_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            output: output.to_string_lossy().into_owned(),
            source: source.to_string_lossy().into_owned(),
            source_digest: source_digest.to_string(),
            steps: Vec::new(),
        }
    }

    pub fn steps(&self) -> &[StepReceipt] {
        &self.steps
    }

    pub async fn step<T, F, Fut>(&mut self, id: &str, title: &str, work: F) -> Result<T, DynError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(T, Evidence), DynError>>,
    {
        let started = Instant::now();
        match work().await {
            Ok((value, evidence)) => {
                self.steps.push(StepReceipt {
                    id: id.to_string(),
                    title: title.to_string(),
                    status: StepStatus::Passed,
                    duration_ms: started.elapsed().as_millis(),
                    evidence,
                    error: None,
                });
                println!("PASS {id:34} {title}");
                Ok(value)
            }
            Err(cause) => {
                let message = cause.to_string();
                self.steps.push(StepReceipt {
                    id: id.to_string(),
                    title: title.to_string(),
                    status: StepStatus::Failed,
                    duration_ms: started.elapsed().as_millis(),
                    evidence: Evidence::new(),
                    error: Some(message.clone()),
                });
                println!("FAIL {id:34} {message}");
                Err(cause)
            }
        }
    }

    pub fn write_artifacts(&self, failure: Option<String>) -> Result<(), DynError> {
        let receipt = BenchmarkReceipt {
            schema_version: 1,
            benchmark: "clark_skill_experience_v1",
            status: if failure.is_none() {
                "passed"
            } else {
                "failed"
            },
            started_at_epoch_ms: self.started_at_epoch_ms,
            duration_ms: self.started.elapsed().as_millis(),
            source: self.source.clone(),
            source_digest: self.source_digest.clone(),
            workspace: self.output.clone(),
            live_model_calls: 0,
            steps: self.steps.clone(),
            failure,
        };
        let output = Path::new(&self.output);
        std::fs::write(
            output.join("receipt.json"),
            serde_json::to_vec_pretty(&receipt)?,
        )?;
        std::fs::write(output.join("report.md"), markdown(&receipt))?;
        Ok(())
    }
}

fn markdown(receipt: &BenchmarkReceipt) -> String {
    let mut out = format!(
        "# Clark skill experience benchmark\n\n\
         **Result:** {}  \n\
         **Source:** `{}`  \n\
         **Source digest:** `{}`  \n\
         **Synthetic user workspace:** `{}`  \n\
         **Live model calls:** 0\n\n\
         This benchmark starts from isolated empty local and remote user homes. \
         A scripted loopback endpoint exercises Clark's real provider request boundary \
         without measuring or claiming model quality.\n\n## Journey\n\n",
        receipt.status, receipt.source, receipt.source_digest, receipt.workspace
    );
    for step in &receipt.steps {
        let marker = match step.status {
            StepStatus::Passed => "PASS",
            StepStatus::Failed => "FAIL",
        };
        out.push_str(&format!(
            "- **{marker}** `{}` — {} ({} ms)\n",
            step.id, step.title, step.duration_ms
        ));
        for (key, value) in &step.evidence {
            out.push_str(&format!("  - {key}: `{}`\n", compact(value)));
        }
        if let Some(error) = &step.error {
            out.push_str(&format!("  - error: `{error}`\n"));
        }
    }
    out
}

fn compact(value: &Value) -> String {
    let rendered = value.to_string();
    if rendered.chars().count() <= 240 {
        rendered
    } else {
        format!("{}…", rendered.chars().take(240).collect::<String>())
    }
}

pub fn evidence<const N: usize>(items: [(&str, Value); N]) -> Evidence {
    items
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

pub fn require(condition: bool, message: impl Into<String>) -> Result<(), DynError> {
    condition.then_some(()).ok_or_else(|| error(message))
}

pub fn error(message: impl Into<String>) -> DynError {
    std::io::Error::other(message.into()).into()
}
