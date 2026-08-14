mod prompts;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agent_core::domain::{AgentEvent, PermissionOptionKind};
use agent_core::ids::RunId;
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::future::join_all;
use futures::StreamExt;
use serde::de::DeserializeOwned;
use serde_json::json;
use uuid::Uuid;

use crate::control::ControlPlane;
use crate::coordinator::support::{
    checkpoint, final_task_statuses, metrics, reader_task, review_task, trigger_metrics, unix_ms,
    verify_task, writer_task,
};
use crate::model::{
    AgentStatus, AttemptRecord, BenchmarkRecord, CheckResult, EvidenceLevel, HardFailure, LaneKind,
    LaneSpec, PermissionCeiling, ReviewVerdict, StructuredHandoff, TaskContract, TaskMode,
    TaskStatus,
};
use crate::scenarios::{self, Scenario, SeededRepository};
use crate::scripted_provider::{attempt_from_events, ScriptedProfile};

pub struct LiveRunOptions {
    pub artifact_root: PathBuf,
    pub repetition: u32,
    pub api_key: String,
    pub base_url: String,
    pub acp_command: Option<Vec<String>>,
    pub attempt_timeout: Duration,
}

struct LiveAttemptRequest {
    task: TaskContract,
    model: String,
    role: String,
    prompt: String,
    acp: bool,
}

pub async fn run_live(
    scenario: &Scenario,
    lane: &LaneSpec,
    options: &LiveRunOptions,
) -> Result<BenchmarkRecord, String> {
    if lane.cloud_agents {
        return Err(
            "the brokered-cloud lane is scripted-only in this neutral foundation; a live run must be owned by a product composition that installs its research ToolPack"
                .into(),
        );
    }
    let run_id = format!(
        "{}-{}-live-r{}-{}",
        scenario.id,
        lane.id,
        options.repetition,
        &Uuid::new_v4().to_string()[..8]
    );
    let run_root = absolute_path(&options.artifact_root.join(&run_id))?;
    let repo_root = run_root.join("repo");
    let attempt_root = run_root.join("attempts");
    let scratch_home = run_root.join("scratch-home");
    std::fs::create_dir_all(&attempt_root).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&scratch_home).map_err(|error| error.to_string())?;
    std::env::set_var("HOME", &scratch_home);

    let seeded = scenarios::seed(&repo_root, scenario)?;
    let baseline_checkpoint = checkpoint(&seeded)
        .await
        .or_else(|| seeded.git_baseline.clone());
    let started_at_unix_ms = unix_ms();
    let started = Instant::now();
    let control = ControlPlane::new(lane.max_concurrency.max(1), lane.token_budget);
    let actual_delegate = should_delegate(lane.kind, scenario);
    let mut tasks = Vec::new();
    let mut attempts = Vec::new();
    let mut handoffs = Vec::new();
    let mut reviews = Vec::new();
    let mut findings = Vec::new();
    let mut hard_failures = BTreeSet::new();
    let mut recovered_failures = 0;
    let mut unrecovered_failures = 0;
    let mut review_catches = 0;
    let mut review_false_vetoes = 0;
    let mut error = None;

    if actual_delegate {
        let before_readers = scenarios::snapshot(&seeded.root)?;
        let requests: Vec<_> = scenario
            .reader_tasks
            .iter()
            .map(|reader| {
                let task = reader_task(reader);
                tasks.push(task.clone());
                let attempt_id = Uuid::new_v4().to_string();
                let model = if reader.cheap_model_eligible {
                    lane.subagent_model
                        .clone()
                        .unwrap_or_else(|| lane.root_model.clone())
                } else {
                    lane.root_model.clone()
                };
                LiveAttemptRequest {
                    prompt: prompts::reader(scenario, &task, &attempt_id),
                    task,
                    model,
                    role: "reader".into(),
                    acp: matches!(lane.kind, LaneKind::MixedHarness),
                }
            })
            .collect();
        let futures = requests.into_iter().map(|request| {
            run_attempt(
                &control,
                &seeded,
                baseline_checkpoint.clone(),
                request,
                &attempt_root,
                options,
            )
        });
        for result in join_all(futures).await {
            match result {
                Ok(attempt) => {
                    if let Some(handoff) = attempt.handoff.clone() {
                        findings.push(handoff.summary.clone());
                        report_handoff(&control, &handoff, &mut handoffs);
                    } else {
                        unrecovered_failures += 1;
                    }
                    attempts.push(attempt);
                }
                Err(reason) => {
                    unrecovered_failures += 1;
                    error.get_or_insert(reason);
                }
            }
        }
        if scenarios::snapshot(&seeded.root)? != before_readers {
            hard_failures.insert(HardFailure::UnauthorizedWrite);
        }
    }

    let writer_task = writer_task(scenario, &tasks);
    tasks.push(writer_task.clone());
    let mut writer_succeeded = false;
    let mut final_writer_attempt = None;
    let mut last_writer_error = None;
    for writer_attempt in 1..=lane.max_attempts.max(1) {
        let attempt_id = Uuid::new_v4().to_string();
        let request = LiveAttemptRequest {
            prompt: prompts::writer(
                scenario,
                &writer_task,
                &attempt_id,
                &findings,
                matches!(lane.kind, LaneKind::PlannedSingle),
                &[],
            ),
            task: writer_task.clone(),
            model: lane.root_model.clone(),
            role: if writer_attempt == 1 {
                "writer".into()
            } else {
                "writer-retry".into()
            },
            acp: false,
        };
        match run_attempt(
            &control,
            &seeded,
            baseline_checkpoint.clone(),
            request,
            &attempt_root,
            options,
        )
        .await
        {
            Ok(attempt) => {
                let valid = valid_writer_handoff(&attempt, &seeded);
                if let Some(handoff) = attempt.handoff.clone() {
                    report_handoff(&control, &handoff, &mut handoffs);
                    final_writer_attempt = Some(handoff.attempt_id.clone());
                }
                writer_succeeded = attempt.status == AgentStatus::Completed && valid;
                attempts.push(attempt);
                if writer_succeeded {
                    break;
                }
                recovered_failures += 1;
            }
            Err(reason) => {
                recovered_failures += 1;
                last_writer_error = Some(reason);
            }
        }
    }
    if !writer_succeeded {
        unrecovered_failures += 1;
        if let Some(reason) = last_writer_error {
            error.get_or_insert(reason);
        }
    }

    let pre_review_grade = scenarios::grade(scenario, &seeded)?;
    if writer_succeeded && lane.reviewer {
        let task = review_task(scenario, &writer_task);
        tasks.push(task.clone());
        let attempt_id = Uuid::new_v4().to_string();
        let request = LiveAttemptRequest {
            prompt: prompts::reviewer(scenario, &writer_task, &attempt_id),
            task,
            model: lane.root_model.clone(),
            role: "reviewer".into(),
            acp: false,
        };
        match run_attempt(
            &control,
            &seeded,
            baseline_checkpoint.clone(),
            request,
            &attempt_root,
            options,
        )
        .await
        {
            Ok(attempt) => {
                let verdict: Option<ReviewVerdict> = extract_json(&attempt.final_message);
                if let Some(verdict) = verdict {
                    if !verdict.accepted {
                        if pre_review_grade.correctness >= 1.0 {
                            review_false_vetoes += 1;
                        } else {
                            review_catches += 1;
                        }
                        control.set_task_status(&writer_task.id, TaskStatus::Rework);
                        let rework_id = Uuid::new_v4().to_string();
                        let rework = LiveAttemptRequest {
                            prompt: prompts::writer(
                                scenario,
                                &writer_task,
                                &rework_id,
                                &findings,
                                false,
                                &verdict.findings,
                            ),
                            task: writer_task.clone(),
                            model: lane.root_model.clone(),
                            role: "writer-rework".into(),
                            acp: false,
                        };
                        match run_attempt(
                            &control,
                            &seeded,
                            baseline_checkpoint.clone(),
                            rework,
                            &attempt_root,
                            options,
                        )
                        .await
                        {
                            Ok(rework_attempt) => {
                                writer_succeeded = valid_writer_handoff(&rework_attempt, &seeded);
                                if let Some(handoff) = rework_attempt.handoff.clone() {
                                    final_writer_attempt = Some(handoff.attempt_id.clone());
                                    report_handoff(&control, &handoff, &mut handoffs);
                                }
                                attempts.push(rework_attempt);
                            }
                            Err(reason) => {
                                writer_succeeded = false;
                                unrecovered_failures += 1;
                                error.get_or_insert(reason);
                            }
                        }
                    }
                    reviews.push(verdict);
                } else {
                    review_false_vetoes += 1;
                    error.get_or_insert("reviewer did not emit a valid structured verdict".into());
                }
                attempts.push(attempt);
            }
            Err(reason) => {
                review_false_vetoes += 1;
                error.get_or_insert(reason);
            }
        }
    }

    if writer_succeeded && lane.verifier {
        let task = verify_task(scenario, &writer_task);
        tasks.push(task.clone());
        let attempt_id = Uuid::new_v4().to_string();
        let request = LiveAttemptRequest {
            prompt: prompts::verifier(scenario, &task, &attempt_id),
            task,
            model: lane.root_model.clone(),
            role: "verifier".into(),
            acp: false,
        };
        match run_attempt(
            &control,
            &seeded,
            baseline_checkpoint.clone(),
            request,
            &attempt_root,
            options,
        )
        .await
        {
            Ok(attempt) => {
                if let Some(handoff) = attempt.handoff.clone() {
                    report_handoff(&control, &handoff, &mut handoffs);
                }
                attempts.push(attempt);
            }
            Err(reason) => {
                unrecovered_failures += 1;
                error.get_or_insert(reason);
            }
        }
    }

    if writer_succeeded {
        match final_writer_attempt {
            Some(attempt_id) => {
                if let Err(reason) = control.accept_result(&writer_task.id, &attempt_id) {
                    hard_failures.insert(HardFailure::AcceptedUnverifiableResult);
                    error.get_or_insert(reason);
                }
            }
            None => {
                hard_failures.insert(HardFailure::AcceptedUnverifiableResult);
            }
        }
    }

    let grade = scenarios::grade(scenario, &seeded)?;
    if !grade.unexpected_changed_paths.is_empty() {
        hard_failures.insert(HardFailure::OutOfScopeWrite);
    }
    if !grade.lost_user_changes.is_empty() {
        hard_failures.insert(HardFailure::LostUserChange);
    }
    if attempts.is_empty() {
        hard_failures.insert(HardFailure::CausalTraceMissing);
    }
    if !writer_succeeded {
        hard_failures.insert(HardFailure::AcceptedUnverifiableResult);
    }
    let duration_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let run_metrics = metrics(
        &attempts,
        grade.correctness,
        grade.changed_path_precision,
        recovered_failures,
        unrecovered_failures,
        review_catches,
        review_false_vetoes,
        duration_ms,
        lane,
        actual_delegate,
    );
    if run_metrics.lifecycle_trace_failures > 0 {
        hard_failures.insert(HardFailure::LifecycleTraceInvalid);
    }
    if run_metrics.duplicate_tool_receipts > 0 {
        hard_failures.insert(HardFailure::DuplicateToolReceipt);
    }
    let task_statuses = final_task_statuses(&control, &tasks, grade.correctness, writer_succeeded);
    let checks = grade
        .checks
        .into_iter()
        .map(|(id, passed, detail)| CheckResult { id, passed, detail })
        .collect();
    let result_checkpoint = checkpoint(&seeded).await;
    let orchestration_complete =
        hard_failures.is_empty() && grade.correctness >= 1.0 && writer_succeeded && error.is_none();
    let record = BenchmarkRecord {
        schema_version: 1,
        run_id,
        evidence_level: EvidenceLevel::Live,
        scenario_id: scenario.id.clone(),
        scenario_family: scenario.family.clone(),
        variant: scenario.variant,
        repetition: options.repetition,
        lane: lane.clone(),
        repository_path: repo_root.to_string_lossy().into_owned(),
        baseline_checkpoint,
        result_checkpoint,
        started_at_unix_ms,
        tasks,
        task_statuses,
        attempts,
        handoffs,
        reviews,
        actual_changed_paths: grade.changed_paths,
        checks,
        hard_failures,
        trigger: trigger_metrics(scenario, lane, actual_delegate),
        metrics: run_metrics,
        orchestration_complete,
        error,
    };
    std::fs::write(
        run_root.join("record.json"),
        serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(record)
}

async fn run_attempt(
    control: &ControlPlane,
    seeded: &SeededRepository,
    baseline_checkpoint: Option<String>,
    request: LiveAttemptRequest,
    attempt_root: &Path,
    options: &LiveRunOptions,
) -> Result<AttemptRecord, String> {
    let attempt_id =
        extract_prompt_attempt_id(&request.prompt).unwrap_or_else(|| Uuid::new_v4().to_string());
    control.check_permission(
        request.task.permission_ceiling,
        request.task.permission_ceiling,
    )?;
    let reservation = control.reserve_spawn(request.task.logical_path.clone())?;
    reservation.commit(attempt_id.clone())?;
    control.set_task_status(&request.task.id, TaskStatus::Running);
    let result = async {
        let _writer_lease = if request.task.mode == TaskMode::Write {
            Some(control.acquire_writer(&request.task.id)?)
        } else {
            None
        };
        let mut provider = provider(&request);
        provider
            .connect(provider_config(&request, seeded, options)?)
            .await
            .map_err(|error| error.to_string())?;
        let session = provider
            .new_session(SessionOptions {
                cwd: Some(seeded.root.to_string_lossy().into_owned()),
                mode: Some("auto".into()),
                ..Default::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        let started = Instant::now();
        let mut stream = provider
            .prompt(&session.id, PromptInput::text(request.prompt.clone()))
            .await
            .map_err(|error| error.to_string())?;
        let collect = async {
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                if let AgentEvent::PermissionRequest { request: pending } = &event {
                    let option = pending
                        .options
                        .iter()
                        .find(|option| {
                            matches!(
                                option.kind,
                                PermissionOptionKind::RejectOnce
                                    | PermissionOptionKind::RejectAlways
                            )
                        })
                        .ok_or("permission request had no rejection option")?;
                    provider
                        .respond(
                            &session.id,
                            ClientResponse::Permission {
                                request: pending.id.clone(),
                                option: option.id.clone(),
                                feedback: Some(
                                    "benchmark permissions are fail-closed; stay within the role ceiling"
                                        .into(),
                                ),
                            },
                        )
                        .await
                        .map_err(|error| error.to_string())?;
                }
                events.push(event);
            }
            Ok::<_, String>(events)
        };
        let events = match tokio::time::timeout(options.attempt_timeout, collect).await {
            Ok(result) => result?,
            Err(_) => {
                let _ = provider.cancel(&session.id, &RunId::new("timeout")).await;
                return Err(format!("live attempt timed out: {attempt_id}"));
            }
        };
        let duration_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let artifact = attempt_root.join(format!("{attempt_id}.events.json"));
        std::fs::write(
            &artifact,
            serde_json::to_vec_pretty(&events).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let profile = ScriptedProfile {
            provider: if request.acp {
                "acp".into()
            } else {
                "local".into()
            },
            model: request.model.clone(),
            role: request.role.clone(),
            permission_ceiling: request.task.permission_ceiling,
        };
        let mut attempt = attempt_from_events(
            &profile,
            &request.task,
            &attempt_id,
            &events,
            duration_ms,
        );
        attempt.handoff = extract_json::<StructuredHandoff>(&attempt.final_message).filter(|handoff| {
            handoff.task_id == request.task.id && handoff.attempt_id == attempt_id
        });
        if let Some(handoff) = attempt.handoff.as_mut() {
            handoff.baseline_checkpoint = baseline_checkpoint;
            handoff.result_checkpoint = checkpoint(seeded).await;
            handoff
                .artifact_refs
                .push(artifact.to_string_lossy().into_owned());
        }
        let tokens = attempt
            .usage
            .input_tokens
            .saturating_add(attempt.usage.output_tokens);
        if let Err(reason) = control.record_usage(tokens) {
            attempt.status = AgentStatus::Errored;
            attempt.error.get_or_insert(reason);
        }
        let _ = provider.close_session(&session.id).await;
        Ok(attempt)
    }
    .await;
    control.release_agent(&attempt_id);
    result
}

fn provider(request: &LiveAttemptRequest) -> Box<dyn Provider> {
    if request.acp {
        Box::new(provider_acp::AcpProvider::new())
    } else {
        Box::new(provider_local::LocalAgentProvider::new())
    }
}

fn provider_config(
    request: &LiveAttemptRequest,
    seeded: &SeededRepository,
    options: &LiveRunOptions,
) -> Result<ProviderConfig, String> {
    if request.acp {
        if request.task.permission_ceiling != PermissionCeiling::ReadOnly {
            return Err(
                "the mixed ACP harness is read-only and cannot receive a writer task".into(),
            );
        }
        let command = options
            .acp_command
            .as_deref()
            .ok_or("mixed-harness live lane requires an ACP command")?;
        return Ok(ProviderConfig {
            command: Some(os_read_only_acp_command(command)?),
            cwd: Some(seeded.root.to_string_lossy().into_owned()),
            ..Default::default()
        });
    }
    let permission = if request.task.permission_ceiling == PermissionCeiling::WorkspaceWrite {
        "allow"
    } else {
        "deny"
    };
    let mut permissions = serde_json::Map::new();
    for tool in ["write_file", "edit_file", "bash"] {
        permissions.insert(tool.into(), json!(permission));
    }
    Ok(ProviderConfig {
        cwd: Some(seeded.root.to_string_lossy().into_owned()),
        auth_token: Some(options.api_key.clone()),
        headers: HashMap::new(),
        extra: json!({
            "base_url": options.base_url,
            "model": request.model,
            "temperature": 0.0,
            "max_iterations": 24,
            "permissions": permissions,
            "command_denylist": ["rm", "git clean", "git reset", "git checkout", "git restore", "git commit", "git push", "curl", "wget", "ssh"],
            "memories": false,
            "project_knowledge": false,
            "auto_compact": false,
            "browser_enabled": false,
        }),
        ..Default::default()
    })
}

fn os_read_only_acp_command(command: &[String]) -> Result<Vec<String>, String> {
    #[cfg(target_os = "macos")]
    {
        let mut wrapped = vec![
            "/usr/bin/sandbox-exec".to_string(),
            "-p".to_string(),
            "(version 1) (allow default) (deny file-write*)".to_string(),
            "--".to_string(),
        ];
        wrapped.extend(command.iter().cloned());
        Ok(wrapped)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = command;
        Err("live ACP benchmark currently requires the macOS write-denial sandbox".into())
    }
}

fn should_delegate(kind: LaneKind, scenario: &Scenario) -> bool {
    !matches!(kind, LaneKind::Single | LaneKind::PlannedSingle)
        && scenarios::trigger_policy(scenario)
        && !scenario.reader_tasks.is_empty()
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|error| error.to_string())
    }
}

fn report_handoff(
    control: &ControlPlane,
    handoff: &StructuredHandoff,
    handoffs: &mut Vec<StructuredHandoff>,
) {
    if control.report_result(handoff.clone()).is_ok() {
        handoffs.push(handoff.clone());
    }
}

fn valid_writer_handoff(attempt: &AttemptRecord, seeded: &SeededRepository) -> bool {
    let Some(handoff) = &attempt.handoff else {
        return false;
    };
    let Ok(after) = scenarios::snapshot(&seeded.root) else {
        return false;
    };
    attempt.status == AgentStatus::Completed
        && handoff.reported_status == TaskStatus::Reported
        && !handoff.changed_paths.is_empty()
        && handoff.changed_paths.iter().all(|path| {
            seeded.before_agent.get(path) != after.get(path) && after.contains_key(path)
        })
}

fn extract_prompt_attempt_id(prompt: &str) -> Option<String> {
    let marker = "\"attempt_id\": \"";
    let tail = prompt.split(marker).nth(1)?;
    Some(tail.split('"').next()?.to_string())
}

fn extract_json<T: DeserializeOwned>(text: &str) -> Option<T> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Some(value);
    }
    for fenced in trimmed.split("```").skip(1).step_by(2) {
        let candidate = fenced.strip_prefix("json").unwrap_or(fenced).trim();
        if let Ok(value) = serde_json::from_str(candidate) {
            return Some(value);
        }
    }
    let end = trimmed.rfind('}')?;
    trimmed[..=end]
        .char_indices()
        .rev()
        .filter(|(_, character)| *character == '{')
        .find_map(|(start, _)| serde_json::from_str(&trimmed[start..=end]).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_plain_and_fenced_json() {
        let plain: ReviewVerdict =
            extract_json(r#"{"task_id":"t","accepted":true,"findings":[]}"#).unwrap();
        let fenced: ReviewVerdict = extract_json(
            "result\n```json\n{\"task_id\":\"t\",\"accepted\":false,\"findings\":[]}\n```",
        )
        .unwrap();
        assert!(plain.accepted);
        assert!(!fenced.accepted);
    }

    #[test]
    fn extracts_trailing_json_after_prose_with_braces() {
        let verdict: ReviewVerdict = extract_json(
            "Inspected {'ok': True}; no defect.\n{\"task_id\":\"t\",\"accepted\":true,\"findings\":[]}",
        )
        .unwrap();
        assert!(verdict.accepted);
    }
}
