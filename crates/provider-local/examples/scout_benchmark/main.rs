mod model;
mod scenarios;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use exec_core::collect_system_capabilities;
use model::{write_artifacts, CapabilityReceipt, Recorder};
use provider_local::{discover_skill_catalog_snapshot, LocalExecutor};
use serde_json::json;
use uuid::Uuid;

struct Args {
    output: PathBuf,
    host_label: String,
    containment: String,
    denied_write: Option<PathBuf>,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = Self {
            output: PathBuf::from("target/scout-benchmark").join(&Uuid::new_v4().to_string()[..8]),
            host_label: "local".into(),
            containment: "external".into(),
            denied_write: None,
        };
        let mut input = std::env::args().skip(1);
        while let Some(argument) = input.next() {
            match argument.as_str() {
                "--out" => {
                    args.output = PathBuf::from(input.next().ok_or("--out requires a path")?)
                }
                "--host-label" => {
                    args.host_label = input.next().ok_or("--host-label requires a value")?
                }
                "--containment" => {
                    args.containment = input.next().ok_or("--containment requires a value")?
                }
                "--denied-write" => {
                    args.denied_write = Some(PathBuf::from(
                        input.next().ok_or("--denied-write requires a path")?,
                    ))
                }
                "--help" | "-h" => {
                    println!(
                        "scout_benchmark [--out PATH] [--host-label LABEL] \
                         [--containment external|bwrap] [--denied-write PATH]"
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        if args.host_label.is_empty()
            || !args
                .host_label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("host label may contain only letters, digits, dash, and underscore".into());
        }
        if !matches!(args.containment.as_str(), "external" | "bwrap") {
            return Err("containment must be external or bwrap".into());
        }
        Ok(args)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Scout benchmark failed: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args = Args::parse()?;
    create_output(&args.output)?;
    let system = collect_system_capabilities(None);
    let executable_set = system
        .executable_names
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let known_tools = [
        "git", "gh", "aws", "rg", "cargo", "rustc", "bwrap", "wasmtime",
    ]
    .into_iter()
    .map(|tool| (tool.to_string(), json!(executable_set.contains(tool))))
    .collect::<serde_json::Map<_, _>>();
    let capabilities = CapabilityReceipt {
        platform: system.platform,
        architecture: system.architecture,
        executable_count: system.executable_names.len(),
        environment_name_count: system.environment_variable_names.len(),
        credential_surfaces: system.credential_surfaces,
        known_tools: serde_json::Value::Object(known_tools),
        values_observed: false,
        executables_truncated: system.executables_truncated,
        environment_names_truncated: system.environment_names_truncated,
    };

    let mut recorder = Recorder::new();
    let skill_result = skill_contract().await;
    recorder.case("bundled_skill_contract", || skill_result);
    recorder.case("complete_ledger_replay", scenarios::complete_replay);
    recorder.case(
        "unissued_assignment_rejected",
        scenarios::unissued_assignment_rejected,
    );
    recorder.case(
        "worker_self_certification_rejected",
        scenarios::worker_self_certification_rejected,
    );
    recorder.case(
        "missing_replay_recipe_rejected",
        scenarios::missing_replay_recipe_rejected,
    );
    recorder.case(
        "unverified_failed_test_rejected",
        scenarios::unverified_failed_test_rejected,
    );
    recorder.case("t3_controls_required", scenarios::t3_controls_required);
    recorder.case(
        "underpowered_null_rejected",
        scenarios::underpowered_null_rejected,
    );
    recorder.case(
        "partial_seal_requires_gap",
        scenarios::partial_requires_limit,
    );
    recorder.case("forged_actor_rejected", scenarios::forged_actor_rejected);
    recorder.case("wilson_reference", scenarios::wilson_reference);
    recorder.case(
        "seeded_bootstrap_determinism",
        scenarios::seeded_bootstrap_determinism,
    );
    recorder.case("containment_controls", || containment_contract(&args));

    let receipt = recorder.finish(args.host_label, capabilities, args.containment);
    let passed = receipt.status == "passed";
    write_artifacts(&args.output, &receipt)?;
    println!("receipt={}", args.output.join("receipt.json").display());
    println!("canonical_sha256={}", receipt.canonical_sha256);
    if passed {
        Ok(())
    } else {
        Err("one or more benchmark cases failed".into())
    }
}

async fn skill_contract() -> Result<(String, serde_json::Value), String> {
    let skill_body = include_str!("../../skills/scout/SKILL.md");
    for required_rule in [
        "Exhaust every discovered capability family and safe authentication context",
        "Every pinned manifest row has a terminal status",
    ] {
        if !skill_body.contains(required_rule) {
            return Err(format!(
                "bundled Scout skill is missing exhaustive-sweep rule: {required_rule}"
            ));
        }
    }
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let tools = HashSet::from([
        "scout_capabilities".to_string(),
        "scout_ledger".to_string(),
        "scout_probe".to_string(),
        "scout_measure".to_string(),
        "delegate_read_only".to_string(),
        "resolve_delegation".to_string(),
    ]);
    let catalog = discover_skill_catalog_snapshot(
        &LocalExecutor,
        project.path(),
        "scout-benchmark",
        &tools,
        &[],
    )
    .await;
    let scout = catalog
        .skills
        .iter()
        .find(|skill| skill.invocation_name == "scout:scout")
        .ok_or_else(|| "bundled Scout skill missing".to_string())?;
    if !scout.enabled || !scout.missing_tools.is_empty() {
        return Err(format!(
            "Scout skill disabled: {:?}",
            scout.disabled_reason.as_deref()
        ));
    }
    Ok((
        "bundled Scout resolves with its exact typed dependencies and exhaustive manifest contract"
            .into(),
        json!({
            "invocation_name": scout.invocation_name,
            "required_tools": scout.required_tools,
            "catalog_revision": catalog.revision,
            "manifest_contract": "all_rows_terminal",
        }),
    ))
}

fn containment_contract(args: &Args) -> Result<(String, serde_json::Value), String> {
    match args.containment.as_str() {
        "external" => Ok((
            "external containment recorded as capability-limited, not isolated".into(),
            json!({
                "isolation_verdict": "unfalsifiable",
                "missing_instrument": "attested OS sandbox wrapper",
                "positive_write_control": "benchmark receipt directory"
            }),
        )),
        "bwrap" => {
            let path = args
                .denied_write
                .as_deref()
                .ok_or("bwrap containment requires --denied-write")?;
            match std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)
            {
                Err(_) => Ok((
                    "bubblewrap denied the negative write control".into(),
                    json!({
                        "isolation_verdict": "supported",
                        "negative_write_control": "denied",
                        "positive_write_control": "benchmark receipt directory"
                    }),
                )),
                Ok(_) => {
                    let _ = std::fs::remove_file(path);
                    Err("declared bwrap containment allowed the denied write control".into())
                }
            }
        }
        _ => Err("unsupported containment".into()),
    }
}

fn create_output(output: &Path) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::create_dir(output)
        .map_err(|error| format!("refusing to overwrite {}: {error}", output.display()))
}
