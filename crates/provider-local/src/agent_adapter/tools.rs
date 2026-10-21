use super::*;

pub(crate) struct DesktopToolRegistryOptions {
    pub session: Arc<Mutex<SessionState>>,
    pub control: Arc<Mutex<RunControl>>,
    pub session_id: SessionId,
    pub run: RunId,
    pub events: Sender<desktop::AgentEvent>,
    pub execution: Option<crate::root_execution::RootExecutionTrace>,
    pub image_policy: ToolImagePolicy,
    /// Plan Mode uses Codex's hidden proposed-plan artifact instead of making
    /// either plan tool part of autoregressive context.
    pub hide_plan_mode_tools: bool,
}

/// How tool-produced images reach the current coding model. The UI always
/// receives typed image blocks; models without native image input receive a
/// bounded isolated vision description instead of an incompatible synthetic
/// multimodal turn.
#[derive(Clone)]
pub(crate) struct ToolImagePolicy {
    pub native_image_support: bool,
    pub vision: Option<AuxiliaryModelConfig>,
}

pub(crate) fn desktop_tool_registry(
    source: Arc<ToolRegistry>,
    ctx: ToolCtx,
    options: DesktopToolRegistryOptions,
) -> ca::ToolRegistry {
    let mut registry = ca::ToolRegistry::new();
    let mut gate = PermissionGate::new(
        options.session,
        options.control,
        options.session_id,
        options.events.clone(),
    );
    if let Some(execution) = options.execution {
        gate = gate.with_execution(execution);
    }
    for exec in source.executors() {
        if options.hide_plan_mode_tools
            && matches!(
                exec.name(),
                "propose_plan" | "update_plan" | "deliver_files"
            )
        {
            continue;
        }
        registry.register(Arc::new(DesktopToolAdapter {
            exec,
            ctx: ctx.clone(),
            gate: gate.clone(),
            run: options.run.clone(),
            events: options.events.clone(),
            image_policy: options.image_policy.clone(),
        }));
    }
    registry
}

struct DesktopToolAdapter {
    exec: Arc<dyn ToolExecutor>,
    ctx: ToolCtx,
    gate: PermissionGate,
    /// Typed tool effects are normalized against the active run here because
    /// they are provider-local outcomes, not `agent-loop` stream events.
    run: RunId,
    events: Sender<desktop::AgentEvent>,
    image_policy: ToolImagePolicy,
}

#[async_trait]
impl ca::AgentTool for DesktopToolAdapter {
    fn name(&self) -> &str {
        self.exec.name()
    }

    fn description(&self) -> &str {
        self.exec.description()
    }

    fn parameters_schema(&self) -> Value {
        self.exec.parameters()
    }

    fn requires_exclusive_sandbox(&self) -> bool {
        self.exec.mutating()
    }

    async fn execute(
        &self,
        call_id: &str,
        args: Value,
        signal: CancellationToken,
        update: ca::ToolUpdateSink,
    ) -> Result<ca::ToolResult, ca::ToolError> {
        let tool_id = ToolCallId::new(call_id.to_string());

        let mut args = match args {
            Value::Null => json!({}),
            other => other,
        };

        let hooks = { self.ctx.session.lock().await.hooks.clone() };
        if !hooks.pre_tool_use.is_empty() {
            let hook_args = redaction::persisted_tool_args(self.exec.name(), &args);
            let runtime_args = args.clone();
            match crate::hooks::run_pre_tool_use(
                self.ctx.executor.as_ref(),
                self.ctx.sandbox.root(),
                &hooks.pre_tool_use,
                self.exec.name(),
                hook_args,
                &signal,
            )
            .await
            {
                crate::hooks::PreToolUseResult::Deny { reason } => {
                    return Ok(ca::ToolResult::error(format!(
                        "Blocked by a PreToolUse hook: {reason}"
                    )));
                }
                crate::hooks::PreToolUseResult::Allow { args: updated } => {
                    args =
                        redaction::restore_runtime_payload(self.exec.name(), &runtime_args, updated)
                }
            }
        }

        match self
            .gate
            .check(
                &tool_id,
                self.exec.name(),
                self.exec.as_ref(),
                &args,
                &self.ctx,
                &signal,
            )
            .await
        {
            PermissionOutcome::Allowed => {}
            PermissionOutcome::Denied(message) => return Ok(ca::ToolResult::error(message)),
            PermissionOutcome::Cancelled => return Err(ca::ToolError::Aborted),
            PermissionOutcome::Failed(message) => return Err(ca::ToolError::Fatal(message)),
        }

        if signal.is_cancelled() {
            return Err(ca::ToolError::Aborted);
        }

        // Hand the tool a live-progress sink: each reported delta rides the
        // engine's update channel out as `ToolExecutionUpdate`, which the UI
        // shows on the in-flight tool row (streamed shell output, grep progress).
        let mut call_ctx = self.ctx.clone();
        call_ctx.progress = Some(Arc::new(move |delta: String| {
            let _ = update.send(ca::ToolResult::text(delta));
        }));
        let events = self.events.clone();
        let progress_run = self.run.clone();
        let progress_parent = tool_id.clone();
        call_ctx.agent_progress = Some(Arc::new(move |agent| {
            let _ = events.try_send(desktop::AgentEvent::FanOut {
                run: progress_run.clone(),
                parent: progress_parent.clone(),
                agent,
            });
        }));
        let events = self.events.clone();
        let progress_run = self.run.clone();
        let progress_tool = tool_id.clone();
        call_ctx.call_progress = Some(Arc::new(move |progress| {
            let _ = events.try_send(desktop::AgentEvent::ToolCallUpdate {
                run: progress_run.clone(),
                id: progress_tool.clone(),
                patch: desktop::ToolCallPatch {
                    status: Some(desktop::ToolStatus::InProgress),
                    progress: Some(progress),
                    ..Default::default()
                },
            });
        }));
        let effect_intent = self.exec.effect_intent(&args);
        let mut outcome = self.exec.invoke(args.clone(), &call_ctx).await;
        let static_terminator = self.exec.terminates_run();
        if static_terminator && !outcome.is_error {
            let unresolved = self
                .ctx
                .session
                .lock()
                .await
                .effects
                .completion_prompt(&self.run);
            if let Some(reminder) = unresolved {
                outcome = ToolOutcome::error(reminder);
            }
        }
        let terminates_run = static_terminator
            || outcome.signals.iter().any(|signal| {
                matches!(
                    signal,
                    ToolSignal::Goal(goal)
                        if goal.status == crate::loop_state::GoalStatus::Complete
                )
            });
        if !outcome.is_error {
            if let Some(intent) = effect_intent {
                let receipt = self.ctx.session.lock().await.effects.register(
                    self.run.clone(),
                    call_id.to_string(),
                    self.exec.name(),
                    intent,
                );
                crate::effects::attach_pending_receipt(&mut outcome, &receipt);
            }
        }
        if !outcome.is_error {
            for signal in std::mem::take(&mut outcome.signals) {
                let event = match signal {
                    ToolSignal::ExecutionChecklist {
                        checklist,
                        explanation,
                    } => desktop::AgentEvent::ExecutionChecklistUpdated {
                        run: self.run.clone(),
                        checklist,
                        explanation,
                    },
                    ToolSignal::ProposedPlan(plan) => desktop::AgentEvent::ProposedPlanUpdated {
                        run: self.run.clone(),
                        plan,
                    },
                    ToolSignal::Goal(mut goal) => {
                        goal.run = Some(self.run.clone());
                        desktop::AgentEvent::GoalUpdated {
                            run: self.run.clone(),
                            goal,
                        }
                    }
                };
                let _ = self.events.send(event).await;
            }
        }

        if !hooks.post_tool_use.is_empty() {
            let hook_args = redaction::persisted_tool_args(self.exec.name(), &args);
            let post = crate::hooks::run_post_tool_use(
                self.ctx.executor.as_ref(),
                self.ctx.sandbox.root(),
                &hooks.post_tool_use,
                self.exec.name(),
                &hook_args,
                &outcome,
                &signal,
            )
            .await;
            if let Some(reason) = post.block_reason {
                if !outcome.details.is_object() {
                    outcome.details = json!({});
                }
                outcome.details["_agent_post_tool_original_content"] =
                    Value::String(outcome.content.clone());
                outcome.content = format!(
                    "PostToolUse hook rejected the tool result after execution: {reason}. Inspect canonical state and correct the effect before continuing."
                );
                outcome.is_error = true;
            } else if let Some(feedback) = post.feedback_message {
                if !outcome.details.is_object() {
                    outcome.details = json!({});
                }
                outcome.details["_agent_post_tool_original_content"] =
                    Value::String(outcome.content.clone());
                outcome.content = feedback;
            } else if !post.additional_contexts.is_empty() {
                outcome.content = format!(
                    "{}\n\n[hook context]\n{}",
                    outcome.content,
                    post.additional_contexts.join("\n")
                );
            }
        }

        if !outcome.is_error {
            for artifact in &outcome.artifacts {
                let _ = self
                    .events
                    .send(desktop::AgentEvent::Artifact {
                        run: self.run.clone(),
                        artifact: produced_artifact_to_desktop(artifact, &tool_id),
                    })
                    .await;
            }
        }

        if !self.image_policy.native_image_support && !outcome.images.is_empty() {
            // Keep the actual bytes in structured result metadata for the UI,
            // but do not inject them into a non-vision coding model's wire
            // transcript. A separate vision-only request gives the model the
            // visual facts it needs to continue safely.
            outcome.content.push_str(
                &crate::attachments::describe_tool_images(
                    &outcome.images,
                    self.image_policy.vision.as_ref(),
                    &signal,
                )
                .await,
            );
            store_tool_images(&mut outcome.details, &outcome.images);
        }

        Ok(tool_result_from_outcome(
            outcome,
            self.image_policy.native_image_support,
            terminates_run,
        ))
    }
}

pub(super) fn tool_result_from_outcome(
    outcome: ToolOutcome,
    native_image_support: bool,
    terminates_run: bool,
) -> ca::ToolResult {
    let mut result = if outcome.is_error {
        ca::ToolResult::error(outcome.content)
    } else {
        ca::ToolResult::text(outcome.content)
    };
    if native_image_support {
        for image in &outcome.images {
            result
                .content
                .push(ca::ToolResultBlock::Image(ca::ImageContent {
                    source: format!("data:{};base64,{}", image.mime_type, image.data_base64),
                    media_type: Some(image.mime_type.clone()),
                    alt: image.alt.clone(),
                }));
        }
    }
    result.details = outcome.details;
    if !outcome.locations.is_empty() {
        if !result.details.is_object() {
            result.details = json!({});
        }
        result.details["locations"] = json!(outcome.locations);
    }
    result.terminate = terminates_run && !result.is_error;
    result
}

pub(super) fn produced_artifact_to_desktop(
    artifact: &ProducedArtifact,
    tool_call: &ToolCallId,
) -> desktop::Artifact {
    desktop::Artifact {
        id: artifact.id.clone(),
        title: artifact.title.clone(),
        kind: artifact.kind,
        mime_type: artifact.mime_type.clone(),
        uri: artifact.uri.clone(),
        tool_call: Some(tool_call.clone()),
    }
}
