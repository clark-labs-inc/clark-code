use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn finish_root(
    tx: &Sender<AgentEvent>,
    run: &RunId,
    execution: &RootExecutionTrace,
    context: &RootFinishContext,
    status: RunStatus,
    stop_reason: Option<String>,
    error: Option<String>,
    failure_kind: Option<RunFailureKind>,
    usage: Option<RunUsage>,
) {
    let terminal = match status {
        RunStatus::Done => agent_orchestration::ExecutionState::Completed,
        RunStatus::Cancelled => agent_orchestration::ExecutionState::Cancelled,
        RunStatus::Failed => agent_orchestration::ExecutionState::Failed,
        _ => agent_orchestration::ExecutionState::Blocked,
    };
    let reason = error.clone().or_else(|| stop_reason.clone());
    let execution_summary = execution
        .finalize(context.executor.as_ref(), &context.root, terminal, reason)
        .await;
    context.session.lock().await.active_execution = None;
    if status == RunStatus::Done {
        let _ = tx
            .send(AgentEvent::MessagePhase {
                run: run.clone(),
                phase: agent_core::domain::MessagePhase::FinalAnswer,
            })
            .await;
    }
    finish(
        tx,
        run,
        RunOutcome {
            status,
            stop_reason,
            error,
            failure_kind,
            usage,
            execution: Some(execution_summary),
        },
    )
    .await;
}
