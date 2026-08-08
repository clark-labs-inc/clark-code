//! Post-turn durable-fact extraction — the structural fix for "the model
//! forgot to save".
//!
//! The in-loop `memory` tool depends on the model *choosing* to save while
//! deep in a coding task, which live evals showed is a coin flip. This module
//! runs AFTER the turn finishes, off the latency path: a cheap keyword
//! heuristic decides whether the user's message plausibly stated durable
//! facts; if so, one small side-completion extracts them as strict JSON and
//! deterministic code applies them to the store — provenance-tagged, quoting
//! the user, and superseding contradicted notes via [`memory::delete_memory`].
//!
//! Failures are silent by design (extraction is a best-effort background
//! enrichment, never a turn blocker); parse failures skip the turn.

use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::exec::{Executor, LocalExecutor};
use crate::llm::LlmClient;
use crate::memory::{self, MemoryType};

/// Everything the extractor needs, captured at prompt time.
pub(crate) struct ExtractionCtx {
    pub llm: LlmClient,
    pub executor: Arc<dyn Executor>,
    pub project_root: PathBuf,
    pub global_dir: Option<PathBuf>,
}

/// Cheap pre-filter: does the user's message plausibly contain a durable
/// fact (identity, product, decision, preference, correction)? Keeps the
/// extra model call off turns that are pure task traffic.
pub(crate) fn worth_extracting(user_text: &str) -> bool {
    let t = user_text.to_lowercase();
    const CUES: &[&str] = &[
        "i'm ",
        "i am ",
        "i work",
        "my product",
        "my app",
        "my company",
        "my startup",
        "we use",
        "we call",
        "call them",
        "called '",
        "called \"",
        "remember",
        "always ",
        "never ",
        "from now on",
        "going forward",
        "we decided",
        "we're going with",
        "we're switching",
        "switching to",
        "instead of",
        "rebrand",
        "renamed",
        "prefer",
        "team rule",
        "our rule",
        "no longer",
        "not anymore",
        "keep in mind",
        "for the record",
        "heads up",
        "about me",
    ];
    CUES.iter().any(|c| t.contains(c))
}

/// Max facts applied per turn — extraction enriches, it doesn't transcribe.
const MAX_FACTS: usize = 4;

#[derive(serde::Deserialize)]
struct ExtractedFact {
    title: String,
    content: String,
    #[serde(default)]
    scope: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    supersedes: String,
}

#[derive(serde::Deserialize)]
struct Extraction {
    #[serde(default)]
    facts: Vec<ExtractedFact>,
}

/// Run one extraction pass over the turn's user message and apply the
/// results to the store. Never returns an error — best-effort by contract.
pub(crate) async fn extract_and_store(ctx: ExtractionCtx, user_text: &str) {
    if !worth_extracting(user_text) {
        return;
    }
    // Existing titles let the model mark supersessions instead of duplicating.
    let mut existing = Vec::new();
    for fact in memory::load_facts(
        ctx.executor.as_ref(),
        &memory::memory_dir(&ctx.project_root),
    )
    .await
    {
        if let Some(name) = fact.header.name {
            existing.push(format!("project: {name}"));
        }
    }
    if let Some(gdir) = &ctx.global_dir {
        for fact in memory::load_facts(&LocalExecutor, gdir).await {
            if let Some(name) = fact.header.name {
                existing.push(format!("global: {name}"));
            }
        }
    }
    let existing_block = if existing.is_empty() {
        "(none yet)".to_string()
    } else {
        existing.join("\n")
    };

    let system = "You extract durable memory from a user's message to their coding assistant. \
Reply with ONLY a JSON object, no prose: \
{\"facts\":[{\"title\":\"...\",\"content\":\"...\",\"scope\":\"project|global\",\"type\":\"user|project|feedback|reference\",\"source\":\"user-stated|inferred\",\"supersedes\":\"existing note title or empty string\"}]}\n\
Rules:\n\
- Extract ONLY lasting facts the user actually stated: who they are, what they're building \
and for whom, decisions, vocabulary, standing preferences about how to work.\n\
- NOTHING transient (this task's details), nothing invented, nothing merely implied, and \
nothing from any source other than this user message — no profile data, no prior \
knowledge, no guesses. When the user reports someone else's opinion, keep the attribution \
in the content. When they say something is undecided, record the indecision itself.\n\
- content should quote or closely track the user's own words.\n\
- scope: \"global\" only for facts about the user that hold across all their projects; \
otherwise \"project\".\n\
- If a fact reverses or replaces one of the existing notes listed, put that note's exact \
title in supersedes.\n\
- If the message contains no durable facts, return {\"facts\":[]}.";

    let prompt = format!("Existing note titles:\n{existing_block}\n\nUser message:\n{user_text}");
    let cancel = CancellationToken::new();
    let Ok(reply) = ctx.llm.complete(Some(system), &prompt, &cancel).await else {
        return;
    };
    let Some(parsed) = parse_extraction(&reply) else {
        return;
    };

    for fact in parsed.facts.into_iter().take(MAX_FACTS) {
        if fact.title.trim().is_empty() || fact.content.trim().is_empty() {
            continue;
        }
        let kind = MemoryType::parse(&fact.kind);
        let source = if fact.source == "inferred" {
            "inferred"
        } else {
            "user-stated"
        };
        let (result, forget_scope_global) = if fact.scope == "global" {
            let Some(gdir) = &ctx.global_dir else {
                continue;
            };
            (
                memory::save_memory(
                    &LocalExecutor,
                    gdir,
                    &fact.title,
                    &fact.content,
                    kind,
                    Some(source),
                )
                .await,
                true,
            )
        } else {
            (
                memory::save_memory(
                    ctx.executor.as_ref(),
                    &memory::memory_dir(&ctx.project_root),
                    &fact.title,
                    &fact.content,
                    kind,
                    Some(source),
                )
                .await,
                false,
            )
        };
        if let Err(e) = result {
            tracing::debug!("memory extraction save failed: {e}");
            continue;
        }
        let supersedes = fact.supersedes.trim();
        if !supersedes.is_empty() {
            // Try the scope the new fact landed in first, then the other one —
            // the superseded note may live in either.
            let mut removed = if forget_scope_global {
                memory::delete_memory(
                    &LocalExecutor,
                    ctx.global_dir.as_ref().expect("global scope implies dir"),
                    supersedes,
                )
                .await
                .ok()
                .flatten()
            } else {
                memory::delete_memory(
                    ctx.executor.as_ref(),
                    &memory::memory_dir(&ctx.project_root),
                    supersedes,
                )
                .await
                .ok()
                .flatten()
            };
            if removed.is_none() {
                removed = if forget_scope_global {
                    memory::delete_memory(
                        ctx.executor.as_ref(),
                        &memory::memory_dir(&ctx.project_root),
                        supersedes,
                    )
                    .await
                    .ok()
                    .flatten()
                } else if let Some(gdir) = &ctx.global_dir {
                    memory::delete_memory(&LocalExecutor, gdir, supersedes)
                        .await
                        .ok()
                        .flatten()
                } else {
                    None
                };
            }
            if let Some(file) = removed {
                tracing::debug!("memory extraction superseded {file}");
            }
        }
    }
}

/// Pull the first JSON object out of the model's reply (tolerating fences).
fn parse_extraction(reply: &str) -> Option<Extraction> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    serde_json::from_str(&reply[start..=end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_fires_on_durable_cues_only() {
        assert!(worth_extracting(
            "Hi! I'm a product manager building PawPal for dog owners."
        ));
        assert!(worth_extracting(
            "Heads up — we rebranded, customers are called 'members' now."
        ));
        assert!(worth_extracting("Team rule going forward: ISO dates."));
        assert!(!worth_extracting(
            "Fix the failing unit test in src/util.js"
        ));
        assert!(!worth_extracting("Add a GET /health endpoint"));
    }

    #[test]
    fn parses_extraction_with_fences_and_prose() {
        let reply = "Sure! ```json\n{\"facts\":[{\"title\":\"T\",\"content\":\"C\",\"scope\":\"project\",\"type\":\"user\",\"source\":\"user-stated\",\"supersedes\":\"\"}]}\n```";
        let parsed = parse_extraction(reply).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(parsed.facts[0].title, "T");
        // Empty / junk replies degrade to None, not a panic.
        assert!(parse_extraction("no json here").is_none());
        assert!(parse_extraction("{\"facts\": []}")
            .unwrap()
            .facts
            .is_empty());
    }
}
