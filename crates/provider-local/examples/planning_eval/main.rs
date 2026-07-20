//! Paid, env-gated A/B benchmark for Clark Code Plan Mode.
//!
//! It runs the same synthetic repository scenarios through the legacy and
//! decision-complete prompt profiles, forbids mutation via real Plan Mode, and
//! scores the resulting typed proposals with deterministic contract checks.
//!
//! CLARK_CODE_LIVE=1 CLARK_CODE_API_KEY=... CLARK_CODE_MODEL=... \
//! PLANNING_EVAL_SCENARIOS=typed-boundary,preference-migration,parser-fix \
//! PLANNING_EVAL_MAX_COST_USD=5 \
//! cargo run -p provider-local --example planning_eval

use agent_core::domain::{AgentEvent, ProposedPlan};
use agent_core::provider::{
    CollaborationMode, PromptInput, Provider, ProviderConfig, SessionOptions,
};
use futures::StreamExt;
use provider_local::LocalAgentProvider;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Instant;

const MAX_TURNS: usize = 3;
const CONTROL_PROFILE: &str = "decision_complete";
const CANDIDATE_PROFILE: &str = "concise";

struct Scenario {
    id: &'static str,
    prompt: &'static str,
    required_terms: &'static [&'static str],
    seed: fn(&Path),
}

#[derive(Serialize)]
struct ResultRow {
    profile: &'static str,
    scenario: &'static str,
    trial: usize,
    proposed: bool,
    read_only: bool,
    contract_hits: usize,
    contract_total: usize,
    score: f64,
    turns: usize,
    tool_calls: usize,
    input_tokens: u64,
    output_tokens: u64,
    context_tokens: u64,
    cost_usd: f64,
    elapsed_ms: u128,
    plan: Option<String>,
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn seed_typed_boundary(root: &Path) {
    write(
        root,
        "src/domain.rs",
        "pub struct Plan { pub phases: Vec<String> }\n",
    );
    write(
        root,
        "src/provider.rs",
        "pub struct Session { pub mode: Option<String> }\n",
    );
    write(root, "src/projection.rs", "// reducer stores Plan events\n");
    write(
        root,
        "tests/projection.rs",
        "// replay coverage belongs here\n",
    );
}

fn seed_preference_migration(root: &Path) {
    write(
        root,
        "app/permissions.ts",
        "export type Mode = 'ask' | 'auto' | 'full' | 'plan';\n",
    );
    write(root, "app/store.ts", "const key = 'permission-mode';\n");
    write(root, "app/Composer.tsx", "// one combined mode picker\n");
    write(root, "app/history.ts", "// durable transcript replay\n");
}

fn seed_parser_fix(root: &Path) {
    write(
        root,
        "src/checklist.rs",
        "pub fn parse(items: &[Step]) -> bool { !items.is_empty() }\n",
    );
    write(
        root,
        "tests/checklist.rs",
        "// only happy-path coverage today\n",
    );
    write(
        root,
        "README.md",
        "Exactly one step is active until all steps complete.\n",
    );
}

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            id: "typed-boundary",
            prompt: "Design the planning redesign. Separate execution checklists from read-only proposed plans, persist both through projection/replay, and identify exact files, interfaces, migration, and tests. Do not implement.",
            required_terms: &["src/domain.rs", "src/provider.rs", "src/projection.rs", "tests/projection.rs", "migration", "replay"],
            seed: seed_typed_boundary,
        },
        Scenario {
            id: "preference-migration",
            prompt: "Plan a clean UI migration that separates action approval policy from collaboration Plan Mode while preserving users who stored the old combined 'plan' preference. Include current/fresh implementation choices and tests. Do not implement.",
            required_terms: &["app/permissions.ts", "app/store.ts", "app/Composer.tsx", "app/history.ts", "migration", "fresh"],
            seed: seed_preference_migration,
        },
        Scenario {
            id: "parser-fix",
            prompt: "Plan the smallest robust change enforcing exactly one in-progress checklist step, structural-replan explanations, and regression tests. Ground the plan in this repository. Do not implement.",
            required_terms: &["src/checklist.rs", "tests/checklist.rs", "in_progress", "explanation"],
            seed: seed_parser_fix,
        },
    ]
}

fn tree_digest(root: &Path) -> String {
    let mut files = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    files.sort();
    let mut hash = Sha256::new();
    for path in files {
        hash.update(
            path.strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .as_bytes(),
        );
        hash.update(std::fs::read(path).unwrap());
    }
    format!("{:x}", hash.finalize())
}

async fn run_case(
    scenario: &'static Scenario,
    profile: &'static str,
    trial: usize,
    api_key: &str,
    model: &str,
    base_url: &str,
) -> ResultRow {
    let temp = tempfile::tempdir().unwrap();
    (scenario.seed)(temp.path());
    let before = tree_digest(temp.path());
    let started = Instant::now();
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some(api_key.to_string()),
            extra: json!({
                "base_url": base_url,
                "model": model,
                "memories": false,
                "research": false,
                "planning_prompt_profile": profile,
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(temp.path().to_string_lossy().to_string()),
            collaboration_mode: Some(CollaborationMode::Plan),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut proposal: Option<ProposedPlan> = None;
    let mut tool_calls = 0;
    let mut turns = 0;
    let mut input_tokens = 0_u64;
    let mut output_tokens = 0_u64;
    let mut context_tokens = 0_u64;
    let mut cost_usd = 0.0;
    let mut prompt = scenario.prompt.to_string();
    while proposal.is_none() && turns < MAX_TURNS {
        turns += 1;
        let mut stream = provider
            .prompt(&session.id, PromptInput::text(prompt.clone()))
            .await
            .unwrap();
        while let Some(event) = stream.next().await {
            match event {
                AgentEvent::ToolCall { .. } => tool_calls += 1,
                AgentEvent::ProposedPlanUpdated { plan, .. } => proposal = Some(plan),
                AgentEvent::RunFinished { outcome, .. } => {
                    if let Some(usage) = outcome.usage {
                        input_tokens = input_tokens.saturating_add(usage.input_tokens);
                        output_tokens = output_tokens.saturating_add(usage.output_tokens);
                        context_tokens = usage.context_tokens;
                        cost_usd += usage.cost_usd.unwrap_or(0.0);
                    }
                }
                AgentEvent::PermissionRequest { request } => {
                    panic!(
                        "Plan Mode requested permission in {}: {:?}",
                        scenario.id, request
                    )
                }
                _ => {}
            }
        }
        prompt = "Use the repository's existing conventions and your recommended defaults. Resolve any remaining implementation details and propose the complete plan now.".into();
    }

    let after = tree_digest(temp.path());
    let markdown = proposal
        .as_ref()
        .map(|plan| plan.markdown.as_str())
        .unwrap_or("");
    let contract_hits = scenario
        .required_terms
        .iter()
        .filter(|term| {
            markdown
                .to_ascii_lowercase()
                .contains(&term.to_ascii_lowercase())
        })
        .count();
    let read_only = before == after;
    let proposed = proposal.is_some();
    let score = (if proposed { 0.25 } else { 0.0 })
        + (if read_only { 0.25 } else { 0.0 })
        + 0.5 * contract_hits as f64 / scenario.required_terms.len() as f64;
    ResultRow {
        profile,
        scenario: scenario.id,
        trial,
        proposed,
        read_only,
        contract_hits,
        contract_total: scenario.required_terms.len(),
        score,
        turns,
        tool_calls,
        input_tokens,
        output_tokens,
        context_tokens,
        cost_usd,
        elapsed_ms: started.elapsed().as_millis(),
        plan: proposal.map(|plan| plan.markdown),
    }
}

#[tokio::main]
async fn main() {
    assert_eq!(
        std::env::var("CLARK_CODE_LIVE").as_deref(),
        Ok("1"),
        "set CLARK_CODE_LIVE=1 to authorize paid model calls"
    );
    let api_key = std::env::var("CLARK_CODE_API_KEY").expect("CLARK_CODE_API_KEY is required");
    let model = std::env::var("CLARK_CODE_MODEL").expect("CLARK_CODE_MODEL is required");
    let selected =
        std::env::var("PLANNING_EVAL_SCENARIOS").expect("PLANNING_EVAL_SCENARIOS is required");
    let max_cost: f64 = std::env::var("PLANNING_EVAL_MAX_COST_USD")
        .expect("PLANNING_EVAL_MAX_COST_USD is required")
        .parse()
        .expect("cost cap must be a number");
    let base_url = std::env::var("CLARK_CODE_BASE_URL")
        .unwrap_or_else(|_| "https://api.clarkslabs.com/v1".into());
    let selected = selected.split(',').map(str::trim).collect::<Vec<_>>();
    let repetitions = std::env::var("PLANNING_EVAL_REPETITIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let all = Box::leak(scenarios().into_boxed_slice());
    let mut rows = Vec::new();
    let mut spent = 0.0;
    for trial in 1..=repetitions {
        for scenario in all
            .iter()
            .filter(|scenario| selected.contains(&scenario.id))
        {
            for profile in [CONTROL_PROFILE, CANDIDATE_PROFILE] {
                assert!(
                    spent < max_cost,
                    "planning eval cost cap reached before trial {trial} {}/{}",
                    scenario.id,
                    profile
                );
                let row = run_case(scenario, profile, trial, &api_key, &model, &base_url).await;
                spent += row.cost_usd;
                println!("{}", serde_json::to_string(&row).unwrap());
                rows.push(row);
            }
        }
    }
    assert!(!rows.is_empty(), "no selected planning scenarios matched");
    let mean = |profile: &str| {
        let matching = rows
            .iter()
            .filter(|row| row.profile == profile)
            .collect::<Vec<_>>();
        matching.iter().map(|row| row.score).sum::<f64>() / matching.len() as f64
    };
    let mean_input_tokens = |profile: &str| {
        let matching = rows
            .iter()
            .filter(|row| row.profile == profile)
            .collect::<Vec<_>>();
        matching
            .iter()
            .map(|row| row.input_tokens as f64)
            .sum::<f64>()
            / matching.len() as f64
    };
    let control_score = mean(CONTROL_PROFILE);
    let candidate_score = mean(CANDIDATE_PROFILE);
    let control_input = mean_input_tokens(CONTROL_PROFILE);
    let candidate_input = mean_input_tokens(CANDIDATE_PROFILE);
    println!(
        "{}",
        json!({
            "eval": "planning_ab",
            "model": model,
            "scenarios": selected,
            "repetitions": repetitions,
            "control_profile": CONTROL_PROFILE,
            "candidate_profile": CANDIDATE_PROFILE,
            "control_mean": control_score,
            "candidate_mean": candidate_score,
            "score_delta": candidate_score - control_score,
            "control_mean_input_tokens": control_input,
            "candidate_mean_input_tokens": candidate_input,
            "input_token_delta": candidate_input - control_input,
            "input_token_reduction_fraction": if control_input == 0.0 { 0.0 } else { (control_input - candidate_input) / control_input },
            "total_cost_usd": spent,
        })
    );
}
