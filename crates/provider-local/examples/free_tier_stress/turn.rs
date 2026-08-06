use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use agent_core::domain::{
    AgentEvent, ContentBlock, GoalStatus, MessagePhase, Role, RunFailureKind, RunOutcome, RunStatus,
};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use provider_local::LocalAgentProvider;
use serde_json::{json, Value};

use super::model::{event_name, CaseReceipt};
use super::MODEL;

const TURN_TIMEOUT: Duration = Duration::from_secs(300);
const CANCEL_SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

struct Observation {
    outcome: Option<RunOutcome>,
    text: String,
    tools: Vec<String>,
    goal_completed: bool,
    event_counts: BTreeMap<String, usize>,
    model_responses: Vec<Value>,
    errors: Vec<String>,
    duration_ms: u128,
    timed_out: bool,
}

pub(super) async fn connect_provider(
    api_key: &str,
    base_url: &str,
    root: &Path,
) -> Result<(LocalAgentProvider, agent_core::provider::Session), String> {
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            cwd: Some(root.to_string_lossy().into_owned()),
            auth_token: Some(api_key.to_string()),
            extra: json!({
                "base_url": base_url,
                "model": MODEL,
                "temperature": 0.0,
                "reasoning_effort": "max",
                "memories": false,
                "research": false,
                "browser_enabled": false,
                "sandbox_mode": "disabled",
                "permissions": {
                    "write_file": "allow",
                    "edit_file": "allow",
                    "bash": "deny"
                },
                "mcp_servers": []
            }),
            ..ProviderConfig::default()
        })
        .await
        .map_err(|error| format!("connect Free provider: {error}"))?;
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(root.to_string_lossy().into_owned()),
            ..SessionOptions::default()
        })
        .await
        .map_err(|error| format!("create Free session: {error}"))?;
    Ok((provider, session))
}

async fn observe_turn(
    provider: &mut LocalAgentProvider,
    session: &agent_core::provider::Session,
    prompt: &str,
) -> Observation {
    let started = Instant::now();
    let mut observation = Observation {
        outcome: None,
        text: String::new(),
        tools: Vec::new(),
        goal_completed: false,
        event_counts: BTreeMap::new(),
        model_responses: Vec::new(),
        errors: Vec::new(),
        duration_ms: 0,
        timed_out: false,
    };
    let mut events = match provider
        .prompt(&session.id, PromptInput::text(prompt))
        .await
    {
        Ok(events) => events,
        Err(error) => {
            observation.errors.push(format!("prompt failed: {error}"));
            observation.duration_ms = started.elapsed().as_millis();
            return observation;
        }
    };
    let mut active_run = None;
    let collect = async {
        while let Some(event) = events.next().await {
            let name = event_name(&event).to_string();
            *observation.event_counts.entry(name).or_default() += 1;
            match event {
                AgentEvent::RunStarted { run } => active_run = Some(run),
                AgentEvent::MessageChunk {
                    role: Role::Agent,
                    delta: ContentBlock::Text { text },
                    ..
                } => observation.text.push_str(&text),
                AgentEvent::MessagePhase {
                    phase: MessagePhase::Commentary,
                    ..
                } => observation.text.clear(),
                AgentEvent::ToolCall { call, .. } => observation.tools.push(
                    call.tool_name
                        .unwrap_or_else(|| call.title.trim().to_string()),
                ),
                AgentEvent::GoalUpdated { goal, .. } => {
                    observation.goal_completed |= goal.status == GoalStatus::Complete;
                }
                AgentEvent::Trace {
                    source, payload, ..
                } if source == "model_response" => observation.model_responses.push(payload),
                AgentEvent::Error { code, message, .. } => {
                    observation.errors.push(format!("{code}: {message}"));
                }
                AgentEvent::PermissionRequest { request } => {
                    observation
                        .errors
                        .push(format!("unexpected permission request {}", request.id));
                    let _ = provider
                        .respond(
                            &session.id,
                            ClientResponse::Permission {
                                request: request.id,
                                option: "deny".to_string(),
                                feedback: Some("Free stress is unattended and fail-closed".into()),
                            },
                        )
                        .await;
                }
                AgentEvent::RunFinished { outcome, .. } => {
                    observation.outcome = Some(outcome);
                    break;
                }
                _ => {}
            }
        }
    };
    if tokio::time::timeout(TURN_TIMEOUT, collect).await.is_err() {
        observation.timed_out = true;
        observation.errors.push("turn timed out".to_string());
        if let Some(run) = active_run.as_ref() {
            if let Err(error) = provider.cancel(&session.id, run).await {
                observation
                    .errors
                    .push(format!("timeout cancellation failed: {error}"));
            }
        }
    } else if observation.outcome.is_none() {
        observation
            .errors
            .push("event stream closed without RunFinished".to_string());
    }
    observation.duration_ms = started.elapsed().as_millis();
    observation
}

pub(super) fn route_failures(responses: &[Value]) -> Vec<String> {
    if responses.is_empty() {
        return vec!["no model_response trace was emitted".to_string()];
    }
    let mut failures = Vec::new();
    for (index, response) in responses.iter().enumerate() {
        if response.get("requested_model").and_then(Value::as_str) != Some(MODEL) {
            failures.push(format!("response {index} requested_model was not {MODEL}"));
        }
        if response
            .get("resolved_model")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            failures.push(format!("response {index} omitted resolved_model"));
        }
        if response
            .get("provider")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            failures.push(format!("response {index} omitted provider"));
        }
        if response
            .get("fallback_model")
            .is_some_and(|value| !value.is_null())
        {
            failures.push(format!("response {index} used a fallback model"));
        }
    }
    failures
}

fn is_infrastructure(outcome: Option<&RunOutcome>) -> bool {
    outcome
        .and_then(|outcome| outcome.failure_kind)
        .is_some_and(|kind| {
            matches!(
                kind,
                RunFailureKind::PlatformKeyRejected
                    | RunFailureKind::ProviderError
                    | RunFailureKind::RateLimited
                    | RunFailureKind::TransportError
                    | RunFailureKind::ContextOverflow
                    | RunFailureKind::InsufficientCredits
            )
        })
}

fn case_receipt(
    id: &'static str,
    repetition: usize,
    observation: Observation,
    mut oracle_failures: Vec<String>,
) -> CaseReceipt {
    let route = route_failures(&observation.model_responses);
    oracle_failures.extend(route.iter().cloned());
    if observation.timed_out {
        oracle_failures.push("turn exceeded its timeout".to_string());
    }
    if observation.outcome.as_ref().map(|outcome| outcome.status) != Some(RunStatus::Done) {
        oracle_failures.push("run did not finish Done".to_string());
    }
    if !observation.errors.is_empty() {
        oracle_failures.push("typed error events were emitted".to_string());
    }
    let infrastructure_failure = is_infrastructure(observation.outcome.as_ref());
    let route_valid = route.is_empty();
    let passed = oracle_failures.is_empty();
    let verdict = if passed {
        "passed"
    } else if infrastructure_failure {
        "infrastructure_failure"
    } else if observation
        .outcome
        .as_ref()
        .is_some_and(|outcome| outcome.status == RunStatus::Done)
    {
        "quality_failure"
    } else {
        "runtime_failure"
    };
    let usage = observation
        .outcome
        .as_ref()
        .and_then(|outcome| outcome.usage);
    CaseReceipt {
        id,
        repetition,
        verdict,
        passed,
        infrastructure_failure,
        route_valid,
        duration_ms: observation.duration_ms,
        outcome: observation.outcome,
        usage,
        text: bounded_text(&observation.text, 16_000),
        tools: observation.tools,
        goal_completed: observation.goal_completed,
        event_counts: observation.event_counts,
        model_responses: observation.model_responses,
        errors: observation.errors,
        oracle_failures,
    }
}

fn bounded_text(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

pub(super) struct StandardCase<'a> {
    pub(super) id: &'static str,
    pub(super) prompt: String,
    pub(super) expected_text: &'a str,
    pub(super) expected_tools: &'a [(&'a str, usize)],
    pub(super) expected_file: Option<(&'a Path, &'a str)>,
    pub(super) require_goal: bool,
}

pub(super) async fn run_standard_case(
    provider: &mut LocalAgentProvider,
    session: &agent_core::provider::Session,
    repetition: usize,
    case: StandardCase<'_>,
) -> CaseReceipt {
    let observation = observe_turn(provider, session, &case.prompt).await;
    let mut failures = Vec::new();
    if observation.text.trim() != case.expected_text {
        failures.push(format!(
            "final text did not exactly equal {:?}",
            case.expected_text
        ));
    }
    for (tool, minimum) in case.expected_tools {
        let observed = observation.tools.iter().filter(|name| name == tool).count();
        if observed < *minimum {
            failures.push(format!(
                "expected at least {minimum} {tool} calls, observed {observed}"
            ));
        }
    }
    if let Some((path, expected)) = case.expected_file {
        match std::fs::read_to_string(path) {
            Ok(value) if value == expected => {}
            Ok(value) => failures.push(format!("{} contained {value:?}", path.display())),
            Err(error) => failures.push(format!("read {}: {error}", path.display())),
        }
    }
    if case.require_goal && !observation.goal_completed {
        failures.push("typed goal never reached Complete".to_string());
    }
    case_receipt(case.id, repetition, observation, failures)
}

pub(super) async fn run_missing_file_stop_case(
    provider: &mut LocalAgentProvider,
    session: &agent_core::provider::Session,
    repetition: usize,
    sentinel: &str,
) -> CaseReceipt {
    let prompt = format!(
        "Use read_file once on intentionally-absent.txt. The file is expected to be missing: do \
         not retry, search, create it, or call another tool. After the missing-file result, reply \
         exactly `{sentinel}` and stop."
    );
    let observation = observe_turn(provider, session, &prompt).await;
    let mut failures = Vec::new();
    if observation.text.trim() != sentinel {
        failures.push(format!("final text did not exactly equal {sentinel:?}"));
    }
    if observation.tools != ["read_file"] {
        failures.push(format!(
            "expected exactly one read_file call and no optional work, observed {:?}",
            observation.tools
        ));
    }
    if observation.model_responses.len() != 2 {
        failures.push(format!(
            "expected one tool response plus one terminal response, observed {} model responses",
            observation.model_responses.len()
        ));
    }
    case_receipt("missing_file_stop", repetition, observation, failures)
}

pub(super) async fn run_cancel_case(
    provider: &mut LocalAgentProvider,
    session: &agent_core::provider::Session,
    repetition: usize,
) -> CaseReceipt {
    let started = Instant::now();
    let mut observation = Observation {
        outcome: None,
        text: String::new(),
        tools: Vec::new(),
        goal_completed: false,
        event_counts: BTreeMap::new(),
        model_responses: Vec::new(),
        errors: Vec::new(),
        duration_ms: 0,
        timed_out: false,
    };
    let mut events = match provider
        .prompt(
            &session.id,
            PromptInput::text(
                "Inspect every fixture in depth and keep working until all evidence is reconciled.",
            ),
        )
        .await
    {
        Ok(events) => events,
        Err(error) => {
            observation.errors.push(format!("prompt failed: {error}"));
            return case_receipt("cancel", repetition, observation, Vec::new());
        }
    };
    let wait_for_start = async {
        while let Some(event) = events.next().await {
            *observation
                .event_counts
                .entry(event_name(&event).to_string())
                .or_default() += 1;
            if let AgentEvent::RunStarted { run } = event {
                return Some(run);
            }
        }
        None
    };
    let run = match tokio::time::timeout(CANCEL_SETTLE_TIMEOUT, wait_for_start).await {
        Ok(Some(run)) => run,
        Ok(None) => {
            observation
                .errors
                .push("event stream closed before RunStarted".into());
            return case_receipt("cancel", repetition, observation, Vec::new());
        }
        Err(_) => {
            observation
                .errors
                .push("RunStarted was not observed within 10 seconds".into());
            return case_receipt("cancel", repetition, observation, Vec::new());
        }
    };
    if let Err(error) = provider.cancel(&session.id, &run).await {
        observation.errors.push(format!("cancel failed: {error}"));
    }
    let collect = async {
        while let Some(event) = events.next().await {
            *observation
                .event_counts
                .entry(event_name(&event).to_string())
                .or_default() += 1;
            match event {
                AgentEvent::Trace {
                    source, payload, ..
                } if source == "model_response" => {
                    observation.model_responses.push(payload);
                }
                AgentEvent::RunFinished { outcome, .. } => {
                    observation.outcome = Some(outcome);
                    break;
                }
                AgentEvent::Error { code, message, .. } => {
                    observation.errors.push(format!("{code}: {message}"));
                }
                _ => {}
            }
        }
    };
    if tokio::time::timeout(CANCEL_SETTLE_TIMEOUT, collect)
        .await
        .is_err()
    {
        observation.timed_out = true;
        observation
            .errors
            .push("cancel did not settle within 10 seconds".into());
    }
    observation.duration_ms = started.elapsed().as_millis();
    let mut failures = Vec::new();
    if observation.outcome.as_ref().map(|outcome| outcome.status) != Some(RunStatus::Cancelled) {
        failures.push("cancelled run did not finish Cancelled".to_string());
    }
    if observation.timed_out {
        failures.push("cancellation timed out".to_string());
    }
    let route = if observation.model_responses.is_empty() {
        Vec::new()
    } else {
        route_failures(&observation.model_responses)
    };
    failures.extend(route.iter().cloned());
    let route_valid = route.is_empty();
    let passed = failures.is_empty() && observation.errors.is_empty();
    let usage = observation
        .outcome
        .as_ref()
        .and_then(|outcome| outcome.usage);
    CaseReceipt {
        id: "cancel",
        repetition,
        verdict: if passed { "passed" } else { "runtime_failure" },
        passed,
        infrastructure_failure: false,
        route_valid,
        duration_ms: observation.duration_ms,
        outcome: observation.outcome,
        usage,
        text: bounded_text(&observation.text, 16_000),
        tools: observation.tools,
        goal_completed: false,
        event_counts: observation.event_counts,
        model_responses: observation.model_responses,
        errors: observation.errors,
        oracle_failures: failures,
    }
}
