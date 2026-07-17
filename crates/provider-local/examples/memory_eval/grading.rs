//! Check evaluation for the memory-lifecycle eval: deterministic checks over
//! tool calls / files / memory stores, plus an LLM judge for text claims.

use std::path::Path;

use serde::Serialize;

/// One grading assertion. Deterministic checks read the run record and the
/// final repo/memory state; `Judge`/`JudgeNotes` call the judge model.
#[derive(Clone, Debug, Serialize)]
pub enum Check {
    /// Some bash call's command contained any of these needles.
    BashRanAny(Vec<String>),
    /// Final file content contains the needle (case-insensitive).
    FileContains(String, String),
    /// Final file content does NOT contain the needle (case-insensitive).
    FileNotContains(String, String),
    /// The path does not exist at the end of the run.
    FileAbsent(String),
    /// Directory exists and contains at least one file whose name contains
    /// the (possibly empty) needle.
    DirHasFile(String, String),
    /// The final assistant reply contains any of these needles.
    ReplyContainsAny(Vec<String>),
    /// Saved memory (remember-call args ∪ final project store) contains it.
    RememberedContains(String),
    /// Saved memory does NOT contain it (contamination probe).
    RememberedNotContains(String),
    /// A needle that was present in the SEEDED store is gone from the final
    /// store (superseded note was updated or removed).
    StoreForgotten(String),
    /// At least one memory save happened this run.
    MemorySaveHappened,
    /// LLM judge over the final reply.
    Judge(String),
    /// LLM judge over the saved memory notes (remember args + store delta).
    JudgeNotes(String),
}

impl Check {
    pub fn name(&self) -> &'static str {
        match self {
            Check::BashRanAny(_) => "bash_ran_any",
            Check::FileContains(..) => "file_contains",
            Check::FileNotContains(..) => "file_not_contains",
            Check::FileAbsent(_) => "file_absent",
            Check::DirHasFile(..) => "dir_has_file",
            Check::ReplyContainsAny(_) => "reply_contains",
            Check::RememberedContains(_) => "remembered_contains",
            Check::RememberedNotContains(_) => "remembered_not_contains",
            Check::StoreForgotten(_) => "store_forgotten",
            Check::MemorySaveHappened => "memory_save_happened",
            Check::Judge(_) => "judge_reply",
            Check::JudgeNotes(_) => "judge_notes",
        }
    }
}

/// Everything observed while running one scenario.
#[derive(Default)]
pub struct RunRecord {
    /// (tool name, full raw args as JSON text) per call, in order.
    pub tool_calls: Vec<(String, String)>,
    /// Assistant reply text per turn.
    pub replies: Vec<String>,
    /// Concatenated text of the seeded project store (before the run).
    pub store_before: String,
    /// Concatenated text of the project store after the run.
    pub store_after: String,
    pub cost_usd: f64,
}

impl RunRecord {
    fn remembered_text(&self) -> String {
        // remember-call args catch saves routed to the (shared) global scope;
        // the final project store catches direct file writes.
        let mut s = String::new();
        for (name, args) in &self.tool_calls {
            if name == "memory" && args.contains("remember") {
                s.push_str(args);
                s.push('\n');
            }
        }
        s.push_str(&self.store_after);
        s
    }

    fn save_happened(&self) -> bool {
        self.tool_calls
            .iter()
            .any(|(n, a)| n == "memory" && a.contains("remember"))
            || self.store_after.len() > self.store_before.len()
    }
}

#[derive(Serialize)]
pub struct CheckResult {
    pub name: &'static str,
    pub pass: bool,
    pub detail: String,
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

fn read(repo: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(repo.join(rel)).ok()
}

pub async fn grade(
    checks: &[Check],
    record: &RunRecord,
    repo: &Path,
    judge: &JudgeClient,
) -> Vec<CheckResult> {
    let mut out = Vec::new();
    let last_reply = record.replies.last().cloned().unwrap_or_default();
    for check in checks {
        let (pass, detail) = match check {
            Check::BashRanAny(needles) => {
                let ran = record.tool_calls.iter().any(|(n, a)| {
                    n == "bash" && needles.iter().any(|needle| contains_ci(a, needle))
                });
                (ran, format!("wanted any of {needles:?}"))
            }
            Check::FileContains(path, needle) => match read(repo, path) {
                Some(text) => {
                    let pass = contains_ci(&text, needle);
                    let detail = if pass {
                        format!("{path} ∋ {needle:?}")
                    } else {
                        format!(
                            "{path} ∋ {needle:?}; actual: {}",
                            text.chars().take(200).collect::<String>()
                        )
                    };
                    (pass, detail)
                }
                None => (false, format!("{path} missing")),
            },
            Check::FileNotContains(path, needle) => match read(repo, path) {
                Some(text) => (!contains_ci(&text, needle), format!("{path} ∌ {needle:?}")),
                // A missing file trivially doesn't contain the needle.
                None => (true, format!("{path} missing (vacuous pass)")),
            },
            Check::FileAbsent(path) => (
                !repo.join(path).exists(),
                format!("{path} should not exist"),
            ),
            Check::DirHasFile(dir, suffix) => {
                let found = std::fs::read_dir(repo.join(dir))
                    .map(|entries| {
                        entries.flatten().any(|e| {
                            e.path().is_file()
                                && e.file_name().to_string_lossy().contains(suffix.as_str())
                        })
                    })
                    .unwrap_or(false);
                (found, format!("{dir}/ contains a file matching {suffix:?}"))
            }
            Check::ReplyContainsAny(needles) => (
                needles.iter().any(|n| contains_ci(&last_reply, n)),
                format!("reply ∋ any of {needles:?}"),
            ),
            Check::RememberedContains(needle) => (
                contains_ci(&record.remembered_text(), needle),
                format!("memory ∋ {needle:?}"),
            ),
            Check::RememberedNotContains(needle) => (
                !contains_ci(&record.remembered_text(), needle),
                format!("memory ∌ {needle:?}"),
            ),
            Check::StoreForgotten(needle) => (
                contains_ci(&record.store_before, needle)
                    && !contains_ci(&record.store_after, needle),
                format!("store no longer contains {needle:?}"),
            ),
            Check::MemorySaveHappened => (record.save_happened(), "a save happened".into()),
            Check::Judge(rubric) => {
                let verdict = judge.judge(rubric, &last_reply).await;
                (verdict.0, verdict.1)
            }
            Check::JudgeNotes(rubric) => {
                let notes = record.remembered_text();
                let subject = if notes.trim().is_empty() {
                    "(no memory was saved at all)".to_string()
                } else {
                    notes
                };
                let verdict = judge.judge(rubric, &subject).await;
                (verdict.0, verdict.1)
            }
        };
        out.push(CheckResult {
            name: check.name(),
            pass,
            detail,
        });
    }
    out
}

/// Minimal non-streaming chat client for judging, kept independent of the
/// provider's own LLM plumbing so judging stays constant across passes.
pub struct JudgeClient {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub http: reqwest::Client,
}

impl JudgeClient {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: "clark-code".to_string(),
            http: reqwest::Client::new(),
        }
    }

    /// Returns (pass, why). Fails closed (pass=false) on transport errors so
    /// a broken judge shows up as suspicious scores, not silent passes.
    pub async fn judge(&self, rubric: &str, subject: &str) -> (bool, String) {
        let body = serde_json::json!({
            "model": self.model,
            "stream": false,
            "temperature": 0,
            "messages": [
                {"role": "system", "content": "You are a strict evaluator. Apply the rubric to the material. Reply with ONLY a JSON object: {\"pass\": true|false, \"why\": \"<one sentence>\"}"},
                {"role": "user", "content": format!("RUBRIC:\n{rubric}\n\nMATERIAL:\n{subject}")}
            ]
        });
        for attempt in 0..2 {
            let resp = self
                .http
                .post(format!("{}/chat/completions", self.base_url))
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await;
            let Ok(resp) = resp else {
                continue;
            };
            let Ok(json) = resp.json::<serde_json::Value>().await else {
                continue;
            };
            let content = json["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("");
            // Tolerate code fences or prose around the JSON object.
            if let Some(start) = content.find('{') {
                if let Some(end) = content.rfind('}') {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content[start..=end])
                    {
                        let pass = v["pass"].as_bool().unwrap_or(false);
                        let why = v["why"].as_str().unwrap_or("").to_string();
                        return (pass, why);
                    }
                }
            }
            if attempt == 1 {
                return (false, format!("judge unparseable: {content}"));
            }
        }
        (false, "judge unavailable".into())
    }
}
