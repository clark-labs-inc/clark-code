use super::*;

pub(super) fn apply(
    snapshot: &mut ExecutionSnapshot,
    event: &ExecutionEvent,
) -> Result<(), String> {
    match &event.kind {
        ExecutionEventKind::Created { .. } => {
            return Err("created may only appear as the first execution event".to_string())
        }
        ExecutionEventKind::AttemptStarted { attempt } => {
            if !matches!(
                snapshot.state,
                ExecutionState::Queued | ExecutionState::Recovering
            ) {
                return Err("attempts start only from queued or recovering".to_string());
            }
            if attempt.execution != snapshot.id
                || attempt.path != snapshot.root
                || attempt.sequence != snapshot.attempts.len() as u32 + 1
            {
                return Err("attempt identity is not the next root attempt".to_string());
            }
            snapshot.state = ExecutionState::Running;
            snapshot.active_attempt = Some(attempt.clone());
            snapshot.attempts.push(ExecutionAttempt {
                id: attempt.clone(),
                outcome: AttemptOutcome::Running,
                usage: ExecutionUsage::default(),
                failure_class: None,
                failure: None,
            });
        }
        ExecutionEventKind::StateChanged { from, to, .. } => {
            if snapshot.state != *from || !valid_transition(*from, *to) {
                return Err(format!("invalid execution transition: {from:?} -> {to:?}"));
            }
            snapshot.state = *to;
            if to.is_final() {
                let attempt = snapshot
                    .attempts
                    .last_mut()
                    .ok_or_else(|| "terminal execution has no attempt".to_string())?;
                attempt.outcome = match to {
                    ExecutionState::Completed => AttemptOutcome::Completed,
                    ExecutionState::Cancelled => AttemptOutcome::Cancelled,
                    ExecutionState::Blocked => AttemptOutcome::Blocked,
                    ExecutionState::Failed => AttemptOutcome::Failed,
                    _ => unreachable!("final state matched above"),
                };
                snapshot.active_attempt = None;
            }
        }
        ExecutionEventKind::Checkpointed { id } => {
            if id.trim().is_empty() {
                return Err("checkpoint id must not be empty".to_string());
            }
            snapshot.evidence.baseline_checkpoint = Some(id.clone());
        }
        ExecutionEventKind::SteeringRecorded => {
            if snapshot.state.is_final() {
                return Err("cannot steer a finished execution".to_string());
            }
            snapshot.steering_messages = snapshot.steering_messages.saturating_add(1);
        }
        ExecutionEventKind::ToolStarted { id, name, mutating } => {
            if snapshot.state != ExecutionState::Running {
                return Err("tools start only while an execution is running".to_string());
            }
            if id.trim().is_empty() || name.trim().is_empty() {
                return Err("tool evidence requires an id and name".to_string());
            }
            if snapshot.active_tools.contains_key(id) || snapshot.evidence.tools.contains_key(id) {
                return Err(format!("duplicate tool execution id: {id}"));
            }
            snapshot.active_tools.insert(
                id.clone(),
                ToolEvidence {
                    id: id.clone(),
                    name: name.clone(),
                    mutating: *mutating,
                    status: ToolExecutionStatus::Cancelled,
                    locations: BTreeSet::new(),
                },
            );
        }
        ExecutionEventKind::ToolFinished {
            id,
            status,
            locations,
        } => {
            let mut tool = snapshot
                .active_tools
                .remove(id)
                .ok_or_else(|| format!("tool finished without a start event: {id}"))?;
            tool.status = *status;
            tool.locations = locations.clone();
            if tool.name == "check_diagnostics" && *status == ToolExecutionStatus::Completed {
                snapshot
                    .evidence
                    .verification_tools
                    .insert(tool.name.clone());
            }
            snapshot.evidence.tools.insert(id.clone(), tool);
        }
        ExecutionEventKind::UsageRecorded { usage } => {
            snapshot.usage.add(usage, &snapshot.policy);
            let attempt = snapshot
                .attempts
                .last_mut()
                .ok_or_else(|| "usage requires an active or completed attempt".to_string())?;
            attempt.usage.add(usage, &snapshot.policy);
        }
        ExecutionEventKind::RecoveryScheduled {
            failure_class,
            message,
        } => {
            if snapshot.state != ExecutionState::Running
                || !failure_class.recoverable()
                || !snapshot.active_tools.is_empty()
            {
                return Err("recovery was scheduled outside a proven safe boundary".to_string());
            }
            let attempt = snapshot
                .attempts
                .last_mut()
                .ok_or_else(|| "recovery requires an active attempt".to_string())?;
            attempt.outcome = AttemptOutcome::Failed;
            attempt.failure_class = Some(*failure_class);
            attempt.failure = Some(message.clone());
            snapshot.recoveries = snapshot.recoveries.saturating_add(1);
            snapshot.state = ExecutionState::Recovering;
            snapshot.active_attempt = None;
        }
        ExecutionEventKind::ChildAttached { path, role } => {
            if path.depth() == 0 || !path.as_str().starts_with("/root/") {
                return Err("child execution path must be beneath /root".to_string());
            }
            if snapshot.children.contains_key(path) {
                return Err(format!("child already attached: {path}"));
            }
            snapshot.children.insert(
                path.clone(),
                ChildExecution {
                    path: path.clone(),
                    role: *role,
                    status: AgentStatus::PendingInit,
                },
            );
        }
        ExecutionEventKind::ChildUpdated { path, status } => {
            snapshot
                .children
                .get_mut(path)
                .ok_or_else(|| format!("unknown child execution: {path}"))?
                .status = *status;
        }
        ExecutionEventKind::EvidenceFinalized { receipt } => {
            if snapshot.state != ExecutionState::Verifying {
                return Err("evidence finalizes only while verifying".to_string());
            }
            let mut receipt = receipt.clone();
            if receipt.baseline_checkpoint.is_none() {
                receipt.baseline_checkpoint = snapshot.evidence.baseline_checkpoint.clone();
            }
            for (id, tool) in &snapshot.evidence.tools {
                receipt
                    .tools
                    .entry(id.clone())
                    .or_insert_with(|| tool.clone());
            }
            receipt
                .verification_tools
                .extend(snapshot.evidence.verification_tools.iter().cloned());
            snapshot.evidence = receipt;
        }
    }
    Ok(())
}

fn valid_transition(from: ExecutionState, to: ExecutionState) -> bool {
    matches!(
        (from, to),
        (ExecutionState::Running, ExecutionState::AwaitingInput)
            | (ExecutionState::AwaitingInput, ExecutionState::Running)
            | (ExecutionState::Running, ExecutionState::Verifying)
            | (ExecutionState::Verifying, ExecutionState::Completed)
            | (ExecutionState::Verifying, ExecutionState::Failed)
            | (ExecutionState::Verifying, ExecutionState::Cancelled)
            | (ExecutionState::Running, ExecutionState::Failed)
            | (ExecutionState::Running, ExecutionState::Cancelled)
            | (ExecutionState::Running, ExecutionState::Blocked)
            | (ExecutionState::AwaitingInput, ExecutionState::Failed)
            | (ExecutionState::AwaitingInput, ExecutionState::Cancelled)
    )
}
