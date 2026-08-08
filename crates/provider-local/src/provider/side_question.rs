use super::*;

/// Map a no-tools side-question LLM failure to the engine's error vocabulary.
/// Cancelled is silent (the user dismissed the overlay); credit/auth failures
/// keep their typed shape so the UI can prompt appropriately; everything else
/// becomes a transport error.
fn map_llm_error(error: crate::llm::LlmError) -> Error {
    match error {
        crate::llm::LlmError::Cancelled => Error::Other("side question cancelled".into()),
        crate::llm::LlmError::InsufficientCredits => Error::Other("insufficient_credits".into()),
        crate::llm::LlmError::PlatformKeyRejected(message) => {
            Error::Other(format!("platform key rejected: {message}"))
        }
        crate::llm::LlmError::Provider(message) => Error::Other(message),
        crate::llm::LlmError::OutputQuarantined { .. } => {
            Error::Other("model response failed data-isolation validation".into())
        }
        crate::llm::LlmError::ContextOverflow(message) => Error::Other(message),
        crate::llm::LlmError::Recoverable(context) => Error::Transport(context.message),
    }
}

impl LocalAgentProvider {
    /// `/btw` — answer a one-off side question against the session's current
    /// context WITHOUT interrupting the active run or mutating session state
    /// (a forked, single-turn, tool-less model call; ported from Claude
    /// Code's `runSideQuestion`).
    ///
    /// Snapshot the session's system prompt + transcript by clone under the
    /// session lock, release, then build the wire messages lock-free and run a
    /// single no-tools `stream_chat`. Nothing is written back into `transcript`
    /// (or `reads`/`control`/`run_counter`), so the active run — if any — is
    /// untouched and keeps streaming into its own event channel.
    pub(super) async fn side_question_impl(&self, question: &str) -> Result<String> {
        self.side_question_future(question).await
    }

    pub(super) fn side_question_future(&self, question: &str) -> agent_core::SideQuestionFuture {
        let llm = self.llm.clone();
        let session = self.session.clone();
        let question = question.to_string();
        Box::pin(async move {
            let llm = llm.ok_or(Error::NotConnected)?;
            let (system_prompt, transcript) = {
                let state = session.lock().await;
                (state.system_prompt.clone(), state.transcript.clone())
            };

            let wrapped = format!(
                "<system-reminder>This is a side question from the user. Answer it directly in a \
             single response.\n\nIMPORTANT CONTEXT:\n- You are a separate, lightweight agent \
             spawned to answer this one question.\n- The main agent is NOT interrupted — it \
             continues working independently in the background.\n- You share the conversation \
             context but are a completely separate instance.\n- Do NOT reference being \
             interrupted or what you were \"previously doing\" — that framing is incorrect.\n\n\
             CRITICAL CONSTRAINTS:\n- You have NO tools available — you cannot read files, run \
             commands, search, or take any actions.\n- This is a one-off response — there will \
             be no follow-up turns.\n- You can ONLY provide information based on what you already \
             know from the conversation context.\n- NEVER say things like \"Let me…\", \
             \"I'll now…\", or promise to take any action.\n- If you don't know the answer, say \
             so — do not offer to look it up or investigate.\n\nSimply answer the question with \
             the information you have.</system-reminder>\n\n{question}"
            );

            let mut messages = crate::agent_adapter::to_wire_messages(&system_prompt, &transcript);
            messages.push(crate::llm::ChatMessage::user(wrapped));

            let cancel = CancellationToken::new();
            let turn = llm
                .stream_chat(&messages, &[], &cancel, |_| {}, |_| {})
                .await
                .map_err(map_llm_error)?;
            Ok(turn.text)
        })
    }
}
