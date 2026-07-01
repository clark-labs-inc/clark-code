//! Thin launcher from clark-desktop's provider API into `clark_agent::run`.

use std::sync::Arc;

use agent_core::domain::{AgentEvent, RunOutcome, RunStatus};
use agent_core::ids::{RunId, SessionId};
use async_channel::Sender;
use tokio::sync::Mutex;

use crate::agent_adapter::{desktop_tool_registry, ClarkAgentStream, DesktopEventSink};
use crate::compaction::{CheckpointCompactor, CompactionConfig};
use crate::llm::LlmClient;
use crate::loop_state::{RunControl, SessionState};
use crate::tools::{ToolCtx, ToolRegistry};

/// Everything `run_turn` needs, bundled to keep the spawned task signature sane.
pub(crate) struct TurnContext {
    pub llm: LlmClient,
    pub registry: Arc<ToolRegistry>,
    pub ctx: ToolCtx,
    pub session: Arc<Mutex<SessionState>>,
    pub control: Arc<Mutex<RunControl>>,
    pub session_id: SessionId,
    pub max_iterations: u32,
    pub compaction: CompactionConfig,
    pub model: String,
    pub temperature: Option<f32>,
    pub user_text: String,
}

/// Drive one user turn to completion, emitting normalized Desktop events into
/// `tx` while clark-agent owns the actual LLM/tool loop.
pub(crate) async fn run_turn(tc: TurnContext, tx: Sender<AgentEvent>, run: RunId) {
    let cancel = tc.ctx.cancel.clone();
    let _ = tx.send(AgentEvent::RunStarted { run: run.clone() }).await;

    let root = tc.ctx.sandbox.root().to_path_buf();
    if let Ok(Some(id)) =
        tokio::task::spawn_blocking(move || crate::checkpoint::create_checkpoint(&root)).await
    {
        let _ = tx
            .send(AgentEvent::Checkpoint {
                run: run.clone(),
                id,
            })
            .await;
    }

    if cancel.is_cancelled() {
        finish(&tx, &run, RunStatus::Cancelled, None, None).await;
        return;
    }

    let (system_prompt, transcript) = {
        let session = tc.session.lock().await;
        (session.system_prompt.clone(), session.transcript.clone())
    };

    let tools = desktop_tool_registry(
        tc.registry.clone(),
        tc.ctx.clone(),
        tc.session.clone(),
        tc.control.clone(),
        tc.session_id.clone(),
        tx.clone(),
    );
    // Documents the agent writes into this workspace become inline artifacts.
    let docs_dir = tc.ctx.sandbox.docs_root().map(std::path::Path::to_path_buf);
    let sink = Arc::new(DesktopEventSink::new(
        tx.clone(),
        run.clone(),
        tc.registry.clone(),
        docs_dir,
    ));

    let mut builder = clark_agent::AgentBuilder::new()
        .stream(Arc::new(ClarkAgentStream::new(tc.llm.clone())))
        .tools(tools)
        .event_sink(sink)
        .default_execution_mode(clark_agent::ExecutionMode::Sequential)
        .max_iterations(tc.max_iterations as usize)
        .grace_iterations(0)
        .model_id(tc.model.clone())
        .context_transform(CheckpointCompactor::new(
            tc.llm.clone(),
            tc.compaction.clone(),
        ));
    if let Some(temperature) = tc.temperature {
        builder = builder.temperature(temperature);
    }
    let config = match builder.build() {
        Ok(config) => config,
        Err(error) => {
            let message = format!("failed to build local agent loop: {error}");
            let _ = tx
                .send(AgentEvent::Error {
                    code: "local_agent_config".into(),
                    message: message.clone(),
                    run: Some(run.clone()),
                })
                .await;
            finish(&tx, &run, RunStatus::Failed, None, Some(message)).await;
            return;
        }
    };

    let identity = clark_agent::RunIdentity::root()
        .with_run_id(run.as_str())
        .with_conversation_id(tc.session_id.as_str());
    let context = clark_agent::AgentContext::new(system_prompt)
        .with_messages(transcript)
        .with_identity(identity);
    let prompt = clark_agent::AgentMessage::User {
        content: clark_agent::UserContent::Text(tc.user_text),
        timestamp: None,
    };

    match clark_agent::run(vec![prompt], context, &config, cancel).await {
        Ok(result) => {
            let outcome = result.outcome;
            tc.session.lock().await.transcript.extend(result.messages);
            if outcome.is_complete() {
                finish(
                    &tx,
                    &run,
                    RunStatus::Done,
                    Some(outcome.label().to_string()),
                    None,
                )
                .await;
            } else {
                finish(
                    &tx,
                    &run,
                    RunStatus::Failed,
                    Some(outcome.label().to_string()),
                    Some(format!(
                        "stopped after {} model iterations",
                        tc.max_iterations
                    )),
                )
                .await;
            }
        }
        Err(error) => {
            let mapped = map_loop_error(error);
            if let Some((code, message)) = mapped.ui_error.clone() {
                let _ = tx
                    .send(AgentEvent::Error {
                        code,
                        message,
                        run: Some(run.clone()),
                    })
                    .await;
            }
            finish(&tx, &run, mapped.status, None, mapped.run_error).await;
        }
    }
}

#[derive(Clone)]
struct MappedLoopError {
    status: RunStatus,
    run_error: Option<String>,
    ui_error: Option<(String, String)>,
}

fn map_loop_error(error: clark_agent::LoopError) -> MappedLoopError {
    match error {
        clark_agent::LoopError::Aborted => MappedLoopError {
            status: RunStatus::Cancelled,
            run_error: None,
            ui_error: None,
        },
        clark_agent::LoopError::Stream(stream) => map_stream_error(stream),
        clark_agent::LoopError::ToolFatal { tool, reason } => {
            let message = format!("fatal tool `{tool}` error: {reason}");
            MappedLoopError::failed("tool_fatal", message)
        }
        clark_agent::LoopError::InvalidContinuation(message) => {
            MappedLoopError::failed("local_agent_state", message)
        }
        clark_agent::LoopError::EmptyOutcomeBudgetExhausted { budget, observed } => {
            MappedLoopError::failed(
                "empty_agent_response",
                format!(
                    "empty assistant outcome retry budget exhausted: observed {observed}, budget {budget}"
                ),
            )
        }
    }
}

fn map_stream_error(error: clark_agent::StreamError) -> MappedLoopError {
    match error {
        clark_agent::StreamError::Fatal(message)
            if message.starts_with("insufficient_credits:") =>
        {
            MappedLoopError::failed("insufficient_credits", message)
        }
        clark_agent::StreamError::Transient(message)
        | clark_agent::StreamError::ProviderRateLimited(message)
        | clark_agent::StreamError::ZeroOutputTransport(message)
        | clark_agent::StreamError::Fatal(message)
        | clark_agent::StreamError::ContextOverflow(message) => {
            MappedLoopError::failed("model_error", message)
        }
        clark_agent::StreamError::Empty => MappedLoopError::failed(
            "model_error",
            "model returned an empty response".to_string(),
        ),
    }
}

impl MappedLoopError {
    fn failed(code: &str, message: String) -> Self {
        Self {
            status: RunStatus::Failed,
            run_error: Some(message.clone()),
            ui_error: Some((code.to_string(), message)),
        }
    }
}

async fn finish(
    tx: &Sender<AgentEvent>,
    run: &RunId,
    status: RunStatus,
    stop_reason: Option<String>,
    error: Option<String>,
) {
    let _ = tx
        .send(AgentEvent::RunFinished {
            run: run.clone(),
            outcome: RunOutcome {
                status,
                stop_reason,
                error,
            },
        })
        .await;
    tx.close();
}
