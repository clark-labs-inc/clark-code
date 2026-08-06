use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleFinding {
    pub id: &'static str,
    pub severity: &'static str,
    pub boundary: &'static str,
    pub observed_behavior: &'static str,
    pub why_plan_is_not_respected: &'static str,
    pub evidence_test: &'static str,
    pub evidence_mode: &'static str,
}

pub fn findings() -> Vec<LifecycleFinding> {
    vec![
        LifecycleFinding {
            id: "continue_feedback_dropped",
            severity: "high",
            boundary: "plan_review_response_to_next_planner_turn",
            observed_behavior: "Typed ContinuePlanning feedback is accepted by the provider but absent from the next model request.",
            why_plan_is_not_respected: "The review decision mutates mode but does not persist the user's requested revision as planner input.",
            evidence_test: "typed_continue_feedback_is_not_delivered_without_a_user_turn",
            evidence_mode: "deterministic_fake_provider_wire_capture",
        },
        LifecycleFinding {
            id: "stale_revision_approval_selects_newest",
            severity: "critical",
            boundary: "approval_card_to_proposed_plan_state",
            observed_behavior: "An approval sent from revision 1 approves revision 2 when both revisions share the same plan ID.",
            why_plan_is_not_respected: "Approval identity is plan ID only; the reviewed revision hash and revision number are not bound to the decision.",
            evidence_test: "stale_revision_approval_selects_the_newest_same_id_plan",
            evidence_mode: "deterministic_fake_provider_wire_capture",
        },
        LifecycleFinding {
            id: "mode_toggle_promotes_unapproved_plan",
            severity: "critical",
            boundary: "collaboration_mode_toggle_to_execution_authority",
            observed_behavior: "Switching directly from Plan to Default labels an awaiting-decision proposal as the approved plan.",
            why_plan_is_not_respected: "Execution authority is inferred from mode plus proposal presence instead of an explicit revision-bound approval.",
            evidence_test: "direct_mode_toggle_labels_an_unapproved_proposal_as_approved",
            evidence_mode: "deterministic_fake_provider_wire_capture",
        },
        LifecycleFinding {
            id: "fresh_handoff_omits_plan_middle",
            severity: "critical",
            boundary: "typed_plan_state_to_fresh_execution_prompt",
            observed_behavior: "A long plan's head and tail are delivered while a decision in the middle is silently replaced by an omission marker.",
            why_plan_is_not_respected: "The execution contract is a lossy bounded rendering rather than the exact approved bytes or a verified semantic capsule.",
            evidence_test: "typed_fresh_delivery_silently_omits_a_long_plans_middle",
            evidence_mode: "deterministic_fake_provider_wire_capture",
        },
        LifecycleFinding {
            id: "duplicate_fresh_approval_clears_authority",
            severity: "high",
            boundary: "duplicate_approval_to_fresh_session_reset",
            observed_behavior: "A second Fresh approval clears task and plan context without reinjecting approved-plan authority.",
            why_plan_is_not_respected: "Approval is not one-shot or idempotent and Fresh reset is not coupled to durable authority reinjection.",
            evidence_test: "duplicate_fresh_approval_clears_context_without_reinjecting_the_plan",
            evidence_mode: "deterministic_fake_provider_wire_capture",
        },
        LifecycleFinding {
            id: "delayed_continue_reopens_approved_plan",
            severity: "critical",
            boundary: "approved_plan_state_to_delayed_review_decision",
            observed_behavior: "After implementation approval, a delayed ContinuePlanning decision for the same plan ID is accepted and returns the session to Plan Mode.",
            why_plan_is_not_respected: "Plan decisions are not compare-and-swap transitions over revision, status, and decision identity.",
            evidence_test: "delayed_continue_decision_reopens_an_approved_session",
            evidence_mode: "deterministic_fake_provider_wire_capture",
        },
        LifecycleFinding {
            id: "proposal_not_frozen_at_review",
            severity: "high",
            boundary: "propose_plan_tool_to_reviewed_proposal",
            observed_behavior: "The model can emit multiple plan proposals in one turn; later bytes replace the first visible revision before the turn ends.",
            why_plan_is_not_respected: "Displaying a proposal does not terminate or freeze the reviewed revision, so review UI and final provider state can diverge.",
            evidence_test: "propose_plan_does_not_terminate_the_turn_or_freeze_the_reviewed_bytes",
            evidence_mode: "deterministic_fake_provider_wire_capture",
        },
        LifecycleFinding {
            id: "fresh_inherits_planner_read_authorization",
            severity: "medium",
            boundary: "planner_workspace_state_to_fresh_executor",
            observed_behavior: "Fresh execution inherits the planner's hidden read-before-edit authorization and can edit without rereading.",
            why_plan_is_not_respected: "Fresh clears visible transcript but reuses session-scoped hidden tool state, so the execution boundary is not actually fresh.",
            evidence_test: "fresh_execution_inherits_the_planners_hidden_read_authorization",
            evidence_mode: "deterministic_fake_provider_wire_capture",
        },
        LifecycleFinding {
            id: "approval_accepted_before_planner_terminal",
            severity: "critical",
            boundary: "visible_proposal_event_to_active_planner_run",
            observed_behavior: "The provider accepts implementation approval while the planner is blocked inside a later model request and has not emitted RunFinished.",
            why_plan_is_not_respected: "Proposal display is not an atomic terminal boundary, so approval can interleave with an active planner turn.",
            evidence_test: "approval_is_accepted_before_the_planner_run_finishes",
            evidence_mode: "deterministic_blocked_provider_wire_capture",
        },
        LifecycleFinding {
            id: "approval_event_deferred_until_execution_prompt",
            severity: "critical",
            boundary: "plan_decision_response_to_durable_event_history",
            observed_behavior: "Implementation approval returns successfully without an event stream; the first Approved plan event is emitted only when a later execution prompt starts.",
            why_plan_is_not_respected: "Approval acknowledgement and durable plan-state projection are split across two host operations with a crash and send-failure gap.",
            evidence_test: "approval_event_is_deferred_until_the_next_execution_prompt",
            evidence_mode: "deterministic_fake_provider_event_capture",
        },
        LifecycleFinding {
            id: "unplanned_write_is_allowed",
            severity: "critical",
            boundary: "approved_plan_to_execution_tool_policy",
            observed_behavior: "After approval, the executor successfully creates a file that the approved plan explicitly excludes.",
            why_plan_is_not_respected: "Write authorization is governed by ordinary permissions and sandbox containment, not the approved plan.",
            evidence_test: "approved_plan_does_not_constrain_execution_writes",
            evidence_mode: "deterministic_fake_provider_workspace_receipt",
        },
        LifecycleFinding {
            id: "workspace_drift_does_not_invalidate_approval",
            severity: "high",
            boundary: "planning_workspace_snapshot_to_execution_workspace",
            observed_behavior: "The repository changes after proposal emission, but the original proposal can still be approved and executed as authoritative.",
            why_plan_is_not_respected: "Approval carries no repository baseline or dirty-tree fingerprint for compare-and-swap validation.",
            evidence_test: "workspace_drift_does_not_invalidate_the_proposed_plan",
            evidence_mode: "deterministic_fake_provider_workspace_receipt",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::findings;
    use agent_core::domain::{
        AgentEvent, ChecklistStatus, PlanExecutionStep, ProposedPlan, ProposedPlanStatus, RunStatus,
    };
    use agent_core::provider::{
        ClientResponse, CollaborationMode, PlanDecision, PromptInput, Provider, ProviderConfig,
        ResumeItem, ResumeTranscript, SessionOptions,
    };
    use futures::StreamExt;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;

    #[test]
    fn lifecycle_finding_manifest_is_complete_and_unique() {
        let findings = findings();
        assert_eq!(findings.len(), 12);
        let ids = findings
            .iter()
            .map(|finding| finding.id)
            .collect::<std::collections::BTreeSet<_>>();
        let tests = findings
            .iter()
            .map(|finding| finding.evidence_test)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), findings.len());
        assert_eq!(tests.len(), findings.len());
    }

    fn final_body() -> String {
        text_body("done")
    }

    fn text_body(text: &str) -> String {
        [
            &format!("data: {}", json!({"choices":[{"delta":{"content": text}}]})),
            r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            "data: [DONE]",
            "",
        ]
        .join("\n\n")
    }

    fn proposal_args(plan: impl Into<String>) -> Value {
        json!({
            "global_reminders": ["Preserve the approved behavior"],
            "execution_contract": [{
                "title": "Implement the approved change",
                "files": ["src/lib.rs"],
                "done_when": ["The approved behavior is verified"],
                "reminders": ["Do not omit approved requirements"]
            }],
            "plan": plan.into()
        })
    }

    fn completed_plan_args() -> Value {
        json!({
            "explanation": "The approved execution contract is complete.",
            "plan": [{
                "plan_step_id": "step-1",
                "step": "Implement the approved change",
                "status": "completed"
            }]
        })
    }

    fn two_step_proposal_args(plan: impl Into<String>) -> Value {
        json!({
            "global_reminders": ["Preserve the approved behavior"],
            "execution_contract": [{
                "title": "Implement the approved change",
                "files": ["src/lib.rs"],
                "done_when": ["The approved behavior is implemented"],
                "reminders": ["Do not omit approved requirements"]
            }, {
                "title": "Verify the approved change",
                "files": ["tests/lib.rs"],
                "done_when": ["The focused verification passes"],
                "reminders": ["Preserve the implementation evidence"]
            }],
            "plan": plan.into()
        })
    }

    fn two_step_plan_args(first_status: &str, second_status: &str) -> Value {
        json!({
            "explanation": "Advance the approved execution contract.",
            "plan": [{
                "plan_step_id": "step-1",
                "step": "Implement the approved change",
                "status": first_status
            }, {
                "plan_step_id": "step-2",
                "step": "Verify the approved change",
                "status": second_status
            }]
        })
    }

    fn tool_call_sse(call_id: &str, name: &str, args: serde_json::Value) -> String {
        let chunk = json!({
            "choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "id": call_id,
                "function": {"name": name, "arguments": args.to_string()}
            }]}}]
        });
        format!(
            "data: {chunk}\n\ndata: {}\n\ndata: [DONE]\n\n",
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#
        )
    }

    fn http_response(body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        )
        .into_bytes()
    }

    async fn serve(listener: TcpListener, bodies: Vec<String>) -> Vec<Vec<u8>> {
        let responses = bodies
            .iter()
            .map(|body| http_response(body))
            .collect::<Vec<_>>();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut captured = Vec::with_capacity(responses.len());
        for _ in 0..responses.len() {
            let (mut socket, _) = listener.accept().await.unwrap();
            captured.push(read_request(&mut socket).await);
            let index = calls.fetch_add(1, Ordering::SeqCst);
            socket.write_all(&responses[index]).await.unwrap();
            socket.flush().await.unwrap();
        }
        captured
    }

    async fn serve_blocked_second_response(
        listener: TcpListener,
        first_body: String,
        second_body: String,
        second_request_seen: oneshot::Sender<()>,
        release_second_response: oneshot::Receiver<()>,
    ) -> Vec<Vec<u8>> {
        let mut captured = Vec::with_capacity(2);
        let (mut first, _) = listener.accept().await.unwrap();
        captured.push(read_request(&mut first).await);
        first.write_all(&http_response(&first_body)).await.unwrap();
        first.flush().await.unwrap();

        let (mut second, _) = listener.accept().await.unwrap();
        captured.push(read_request(&mut second).await);
        let _ = second_request_seen.send(());
        release_second_response.await.unwrap();
        second
            .write_all(&http_response(&second_body))
            .await
            .unwrap();
        second.flush().await.unwrap();
        captured
    }

    async fn read_request(socket: &mut TcpStream) -> Vec<u8> {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        let mut content_length = None;
        loop {
            let count = socket.read(&mut chunk).await.unwrap();
            if count == 0 {
                return buffer;
            }
            buffer.extend_from_slice(&chunk[..count]);
            if content_length.is_none() {
                if let Some(headers_end) = headers_end(&buffer) {
                    let headers = String::from_utf8_lossy(&buffer[..headers_end]);
                    content_length = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    });
                }
            }
            if let (Some(headers_end), Some(length)) = (headers_end(&buffer), content_length) {
                if buffer.len() >= headers_end + 4 + length {
                    return buffer;
                }
            }
        }
    }

    fn headers_end(bytes: &[u8]) -> Option<usize> {
        bytes.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn request_json(raw: &[u8]) -> Value {
        let end = headers_end(raw).unwrap();
        serde_json::from_slice(&raw[end + 4..]).unwrap()
    }

    async fn provider(
        directory: &tempfile::TempDir,
        address: std::net::SocketAddr,
        collaboration_mode: CollaborationMode,
        resume: Option<ResumeTranscript>,
    ) -> (
        provider_local::LocalAgentProvider,
        agent_core::provider::Session,
    ) {
        provider_with_reminders(directory, address, collaboration_mode, resume, false).await
    }

    async fn provider_with_reminders(
        directory: &tempfile::TempDir,
        address: std::net::SocketAddr,
        collaboration_mode: CollaborationMode,
        resume: Option<ResumeTranscript>,
        plan_execution_reminders: bool,
    ) -> (
        provider_local::LocalAgentProvider,
        agent_core::provider::Session,
    ) {
        let mut provider = provider_local::LocalAgentProvider::new();
        provider
            .connect(ProviderConfig {
                auth_token: Some("test-key".into()),
                extra: json!({
                    "base_url": format!("http://{address}/v1"),
                    "model": "fake-model",
                    "memories": false,
                    "sandbox_mode": "disabled",
                    "compact_recent_user_token_budget": 1,
                    "hidden_plan_protocol": false,
                    "planning_research_autoactivate": false,
                    "plan_execution_reminders": plan_execution_reminders,
                    "permissions": {
                        "edit_file": "allow",
                        "write_file": "allow"
                    }
                }),
                ..Default::default()
            })
            .await
            .unwrap();
        let session = provider
            .new_session(SessionOptions {
                cwd: Some(directory.path().to_string_lossy().to_string()),
                collaboration_mode: Some(collaboration_mode),
                resume,
                ..Default::default()
            })
            .await
            .unwrap();
        (provider, session)
    }

    async fn prompt_for_plan(
        provider: &mut provider_local::LocalAgentProvider,
        session: &agent_core::provider::Session,
        text: &str,
    ) -> ProposedPlan {
        let mut stream = provider
            .prompt(&session.id, PromptInput::text(text))
            .await
            .unwrap();
        let mut proposal = None;
        while let Some(event) = stream.next().await {
            if let AgentEvent::ProposedPlanUpdated { ref plan, .. } = event {
                proposal = Some(plan.clone());
            }
            if matches!(event, AgentEvent::RunFinished { .. }) {
                break;
            }
        }
        proposal.unwrap()
    }

    async fn drain_prompt(
        provider: &mut provider_local::LocalAgentProvider,
        session: &agent_core::provider::Session,
        text: &str,
    ) {
        let mut stream = provider
            .prompt(&session.id, PromptInput::text(text))
            .await
            .unwrap();
        while let Some(event) = stream.next().await {
            if matches!(event, AgentEvent::RunFinished { .. }) {
                break;
            }
        }
    }

    async fn collect_prompt(
        provider: &mut provider_local::LocalAgentProvider,
        session: &agent_core::provider::Session,
        text: &str,
    ) -> Vec<AgentEvent> {
        let mut stream = provider
            .prompt(&session.id, PromptInput::text(text))
            .await
            .unwrap();
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            let terminal = matches!(event, AgentEvent::RunFinished { .. });
            events.push(event);
            if terminal {
                break;
            }
        }
        events
    }

    fn run_finished_done(events: &[AgentEvent]) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                AgentEvent::RunFinished { outcome, .. } if outcome.status == RunStatus::Done
            )
        })
    }

    fn role_text(request: &Value, role: &str) -> String {
        request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == role)
            .filter_map(|message| message["content"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn typed_continue_feedback_is_not_delivered_without_a_user_turn() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse("plan-1", "propose_plan", proposal_args("Plan v1")),
                final_body(),
                tool_call_sse("plan-2", "propose_plan", proposal_args("Plan v2")),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let first = prompt_for_plan(&mut provider, &session, "research task").await;
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: first.id,
                    decision: PlanDecision::ContinuePlanning {
                        feedback: Some("ADD_SECURITY_TEST_SENTINEL".into()),
                    },
                },
            )
            .await
            .unwrap();
        let _ = prompt_for_plan(&mut provider, &session, "continue").await;
        let captured = server.await.unwrap();
        let second_turn = request_json(&captured[2]).to_string();
        assert!(
            !second_turn.contains("ADD_SECURITY_TEST_SENTINEL"),
            "typed feedback unexpectedly reached the planner; update the risk classification"
        );
    }

    #[tokio::test]
    async fn stale_revision_approval_selects_the_newest_same_id_plan() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args("Plan v1 reviewed by user"),
                ),
                final_body(),
                tool_call_sse(
                    "plan-2",
                    "propose_plan",
                    proposal_args("Plan v2 not reviewed"),
                ),
                final_body(),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let first = prompt_for_plan(&mut provider, &session, "research task").await;
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: first.id.clone(),
                    decision: PlanDecision::ContinuePlanning { feedback: None },
                },
            )
            .await
            .unwrap();
        let second = prompt_for_plan(&mut provider, &session, "revise").await;
        assert_eq!(first.id, second.id);
        assert_eq!(second.revision, first.revision + 1);
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    // This represents a delayed approval from the v1 card.
                    plan_id: first.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Fresh,
                    },
                },
            )
            .await
            .unwrap();
        drain_prompt(&mut provider, &session, "Implement the approved plan.").await;
        let captured = server.await.unwrap();
        let implementation = request_json(&captured[4]);
        let developer = role_text(&implementation, "developer");
        assert!(developer.contains("Plan v2 not reviewed"));
        assert!(!developer.contains("Plan v1 reviewed by user"));
    }

    #[tokio::test]
    async fn approved_plan_resume_reinjects_typed_execution_authority() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse("checklist-1", "update_plan", completed_plan_args()),
                final_body(),
            ],
        ));
        let resume = ResumeTranscript {
            items: vec![ResumeItem::ProposedPlan {
                plan: ProposedPlan {
                    id: "approved-plan".into(),
                    revision: 3,
                    markdown: "APPROVED_RESUME_SENTINEL".into(),
                    status: ProposedPlanStatus::Approved,
                    global_reminders: vec!["RESUME_GLOBAL_REMINDER".into()],
                    execution_contract: vec![PlanExecutionStep {
                        id: "step-1".into(),
                        title: "Resume approved work".into(),
                        files: vec!["src/lib.rs".into()],
                        done_when: vec!["The resumed behavior is verified".into()],
                        reminders: vec!["RESUME_STEP_REMINDER".into()],
                    }],
                },
            }],
            truncated: false,
        };
        let (mut provider, session) = provider_with_reminders(
            &directory,
            address,
            CollaborationMode::Default,
            Some(resume),
            true,
        )
        .await;
        drain_prompt(&mut provider, &session, "continue implementation").await;
        let captured = server.await.unwrap();
        let request = request_json(&captured[0]);
        let system = role_text(&request, "system");
        let developer = role_text(&request, "developer");
        assert!(
            system.contains("APPROVED_RESUME_SENTINEL"),
            "typed plan data should remain in resumed history"
        );
        assert!(
            developer.contains("<approved_plan_reminder>")
                && developer.contains("\"plan_id\":\"approved-plan\"")
                && developer.contains("RESUME_GLOBAL_REMINDER")
                && developer.contains("RESUME_STEP_REMINDER"),
            "approved plan did not regain typed developer authority: {developer}"
        );
    }

    #[tokio::test]
    async fn compaction_reinjects_typed_approved_plan_authority() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args("COMPACTION_PLAN_SENTINEL"),
                ),
                final_body(),
                tool_call_sse("checklist-1", "update_plan", completed_plan_args()),
                final_body(),
                text_body("COMPACTION_SUMMARY_WITHOUT_APPROVED_PLAN"),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider_with_reminders(&directory, address, CollaborationMode::Plan, None, true).await;
        let plan = prompt_for_plan(&mut provider, &session, "research task").await;
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: plan.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Current,
                    },
                },
            )
            .await
            .unwrap();
        drain_prompt(&mut provider, &session, "Implement the approved plan.").await;

        let mut compacted = provider.compact(&session.id).await.unwrap();
        while compacted.next().await.is_some() {}
        drain_prompt(&mut provider, &session, "continue implementation").await;

        let captured = server.await.unwrap();
        let compaction_request = request_json(&captured[4]).to_string();
        assert!(
            compaction_request.contains("COMPACTION_PLAN_SENTINEL"),
            "the compactor should have received the approved plan"
        );
        let continuation = request_json(&captured[5]);
        let all_messages = continuation["messages"].to_string();
        let developer = role_text(&continuation, "developer");
        assert!(all_messages.contains("COMPACTION_SUMMARY_WITHOUT_APPROVED_PLAN"));
        assert!(
            !all_messages.contains("COMPACTION_PLAN_SENTINEL"),
            "continuation messages: {all_messages}"
        );
        assert!(
            developer.contains("<approved_plan_reminder>")
                && developer.contains("\"plan_id\"")
                && developer.contains("\"step-1\"")
                && developer.contains("Preserve the approved behavior"),
            "compaction did not restore typed plan authority: {developer}"
        );
    }

    #[tokio::test]
    async fn completion_guard_reopens_until_typed_contract_resolves() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args("COMPLETION_GUARD_PLAN_SENTINEL"),
                ),
                final_body(),
                text_body("finished without reconciling the plan"),
                tool_call_sse("checklist-1", "update_plan", completed_plan_args()),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider_with_reminders(&directory, address, CollaborationMode::Plan, None, true).await;
        let proposal = prompt_for_plan(&mut provider, &session, "research task").await;
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Current,
                    },
                },
            )
            .await
            .unwrap();
        let events = collect_prompt(&mut provider, &session, "Implement the approved plan.").await;
        let captured = server.await.unwrap();
        let audit_request = request_json(&captured[3]);
        let developer = role_text(&audit_request, "developer");
        assert!(
            developer.contains("\"reason\":\"completion_audit\"")
                && developer.contains("\"id\":\"step-1\"")
                && developer.contains("The approved behavior is verified"),
            "the unresolved plan did not reopen with its typed evidence contract: {developer}"
        );
        assert!(events.iter().any(|event| {
            matches!(
                event,
                AgentEvent::ExecutionChecklistUpdated { checklist, .. }
                    if checklist.steps.iter().all(|step| {
                        step.plan_step_id.as_deref() == Some("step-1")
                            && step.status == ChecklistStatus::Completed
                    })
            )
        }));
        assert!(run_finished_done(&events));
    }

    #[tokio::test]
    async fn checklist_rejects_unbound_steps_and_emits_only_the_approved_contract() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args("CHECKLIST_BINDING_PLAN_SENTINEL"),
                ),
                final_body(),
                tool_call_sse(
                    "checklist-invalid",
                    "update_plan",
                    json!({
                        "explanation": "Replace approved work.",
                        "plan": [{
                            "step": "Unrelated cleanup only",
                            "status": "completed"
                        }]
                    }),
                ),
                tool_call_sse("checklist-valid", "update_plan", completed_plan_args()),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider_with_reminders(&directory, address, CollaborationMode::Plan, None, true).await;
        let proposal = prompt_for_plan(&mut provider, &session, "research task").await;
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Current,
                    },
                },
            )
            .await
            .unwrap();
        let events = collect_prompt(&mut provider, &session, "Implement the approved plan.").await;
        let captured = server.await.unwrap();
        let retry_request = request_json(&captured[3]).to_string();
        assert!(
            retry_request.contains("plan_step_id")
                && retry_request.contains("approved execution contract"),
            "the invalid checklist did not return a contract-binding error: {retry_request}"
        );
        let checklists = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ExecutionChecklistUpdated { checklist, .. } => Some(checklist),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(checklists.len(), 1);
        assert_eq!(
            checklists[0].steps[0].plan_step_id.as_deref(),
            Some("step-1")
        );
        assert_eq!(
            checklists[0].steps[0].title,
            "Implement the approved change"
        );
        assert!(run_finished_done(&events));
    }

    #[tokio::test]
    async fn completed_step_reinjects_the_next_typed_step_before_generation() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    two_step_proposal_args("STEP_TRANSITION_PLAN_SENTINEL"),
                ),
                final_body(),
                tool_call_sse(
                    "checklist-start",
                    "update_plan",
                    two_step_plan_args("in_progress", "pending"),
                ),
                tool_call_sse(
                    "checklist-transition",
                    "update_plan",
                    two_step_plan_args("completed", "in_progress"),
                ),
                tool_call_sse(
                    "checklist-complete",
                    "update_plan",
                    two_step_plan_args("completed", "completed"),
                ),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider_with_reminders(&directory, address, CollaborationMode::Plan, None, true).await;
        let proposal = prompt_for_plan(&mut provider, &session, "research task").await;
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Current,
                    },
                },
            )
            .await
            .unwrap();
        let events = collect_prompt(&mut provider, &session, "Implement the approved plan.").await;
        let captured = server.await.unwrap();
        let transition_request = request_json(&captured[4]);
        let developer = role_text(&transition_request, "developer");
        assert!(
            developer.contains("\"reason\":\"step_completed\"")
                && developer.contains("\"completed_step_ids\":[\"step-1\"]")
                && developer.contains("\"id\":\"step-2\"")
                && developer.contains("The focused verification passes")
                && developer.contains("Preserve the implementation evidence"),
            "the next generation did not receive the completed-step reminder: {developer}"
        );
        assert!(run_finished_done(&events));
    }

    #[tokio::test]
    async fn direct_mode_toggle_labels_an_unapproved_proposal_as_approved() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args("UNAPPROVED_MODE_TOGGLE_SENTINEL"),
                ),
                final_body(),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let proposal = prompt_for_plan(&mut provider, &session, "research task").await;
        assert_eq!(proposal.status, ProposedPlanStatus::AwaitingDecision);

        provider
            .set_collaboration_mode(&session.id, CollaborationMode::Default)
            .await
            .unwrap();
        drain_prompt(&mut provider, &session, "continue").await;

        let captured = server.await.unwrap();
        let developer = role_text(&request_json(&captured[2]), "developer");
        assert!(developer.contains("Implement the approved plan"));
        assert!(developer.contains("UNAPPROVED_MODE_TOGGLE_SENTINEL"));
    }

    #[tokio::test]
    async fn typed_fresh_delivery_preserves_a_long_plans_middle() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let long_plan = format!(
            "HEAD_SENTINEL\n{}\nMIDDLE_DECISION_SENTINEL\n{}\nTAIL_SENTINEL",
            "a".repeat(5_200),
            "z".repeat(1_800)
        );
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse("plan-1", "propose_plan", proposal_args(long_plan)),
                final_body(),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let proposal = prompt_for_plan(&mut provider, &session, "research task").await;
        assert!(proposal.markdown.contains("MIDDLE_DECISION_SENTINEL"));
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Fresh,
                    },
                },
            )
            .await
            .unwrap();
        drain_prompt(&mut provider, &session, "Implement the approved plan.").await;

        let captured = server.await.unwrap();
        let developer = role_text(&request_json(&captured[2]), "developer");
        assert!(developer.contains("HEAD_SENTINEL"));
        assert!(developer.contains("TAIL_SENTINEL"));
        assert!(!developer.contains("proposal middle omitted"));
        assert!(developer.contains("MIDDLE_DECISION_SENTINEL"));
    }

    #[tokio::test]
    async fn duplicate_fresh_approval_clears_context_without_reinjecting_the_plan() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args("DUPLICATE_APPROVAL_PLAN_SENTINEL"),
                ),
                final_body(),
                final_body(),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let proposal =
            prompt_for_plan(&mut provider, &session, "DUPLICATE_APPROVAL_TASK_SENTINEL").await;
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id.clone(),
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Current,
                    },
                },
            )
            .await
            .unwrap();
        drain_prompt(&mut provider, &session, "Implement the approved plan.").await;

        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Fresh,
                    },
                },
            )
            .await
            .unwrap();
        drain_prompt(&mut provider, &session, "continue implementation").await;

        let captured = server.await.unwrap();
        let continuation = request_json(&captured[3])["messages"].to_string();
        assert!(!continuation.contains("DUPLICATE_APPROVAL_TASK_SENTINEL"));
        assert!(!continuation.contains("DUPLICATE_APPROVAL_PLAN_SENTINEL"));
        assert!(!continuation.contains("Implement the approved plan"));
    }

    #[tokio::test]
    async fn delayed_continue_decision_reopens_an_approved_session() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args("DELAYED_DECISION_PLAN_SENTINEL"),
                ),
                final_body(),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let proposal = prompt_for_plan(&mut provider, &session, "research task").await;
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id.clone(),
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Current,
                    },
                },
            )
            .await
            .unwrap();
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id,
                    decision: PlanDecision::ContinuePlanning {
                        feedback: Some("DELAYED_CONTINUE_SENTINEL".into()),
                    },
                },
            )
            .await
            .expect("a conflicting delayed decision is currently accepted");
        let events =
            collect_prompt(&mut provider, &session, "continue after delayed decision").await;
        let captured = server.await.unwrap();
        assert!(run_finished_done(&events));
        let developer = role_text(&request_json(&captured[2]), "developer");
        assert!(developer.contains("Plan Mode is active"));
        assert!(developer.contains("DELAYED_DECISION_PLAN_SENTINEL"));
        assert!(!developer.contains("Implement the approved plan"));
    }

    #[tokio::test]
    async fn propose_plan_does_not_terminate_the_turn_or_freeze_the_reviewed_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args("FIRST_VISIBLE_PLAN_SENTINEL"),
                ),
                tool_call_sse(
                    "plan-2",
                    "propose_plan",
                    proposal_args("SECOND_SAME_TURN_PLAN_SENTINEL"),
                ),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let mut stream = provider
            .prompt(&session.id, PromptInput::text("research task"))
            .await
            .unwrap();
        let mut proposals = Vec::new();
        while let Some(event) = stream.next().await {
            if let AgentEvent::ProposedPlanUpdated { ref plan, .. } = event {
                proposals.push(plan.clone());
            }
            if matches!(event, AgentEvent::RunFinished { .. }) {
                break;
            }
        }
        let _ = server.await.unwrap();
        assert_eq!(proposals.len(), 2);
        assert_eq!(proposals[0].id, proposals[1].id);
        assert_eq!(proposals[0].revision + 1, proposals[1].revision);
        assert!(proposals[0]
            .markdown
            .contains("FIRST_VISIBLE_PLAN_SENTINEL"));
        assert!(proposals[1]
            .markdown
            .contains("SECOND_SAME_TURN_PLAN_SENTINEL"));
    }

    #[tokio::test]
    async fn fresh_execution_inherits_the_planners_hidden_read_authorization() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("service.mjs"),
            "export const value='old';\n",
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse("read-1", "read_file", json!({"path":"service.mjs"})),
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args("Edit service.mjs from old to new"),
                ),
                final_body(),
                tool_call_sse(
                    "edit-1",
                    "edit_file",
                    json!({
                        "path":"service.mjs",
                        "old_string":"export const value='old';",
                        "new_string":"export const value='new';"
                    }),
                ),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let proposal = prompt_for_plan(&mut provider, &session, "research service.mjs").await;
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Fresh,
                    },
                },
            )
            .await
            .unwrap();
        drain_prompt(&mut provider, &session, "Implement the approved plan.").await;
        let _ = server.await.unwrap();

        assert_eq!(
            std::fs::read_to_string(directory.path().join("service.mjs")).unwrap(),
            "export const value='new';\n",
            "Fresh unexpectedly reset the session-scoped read tracker; update the risk classification"
        );
    }

    #[tokio::test]
    async fn approval_is_accepted_before_the_planner_run_finishes() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (second_seen_tx, second_seen_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(serve_blocked_second_response(
            listener,
            tool_call_sse(
                "plan-1",
                "propose_plan",
                proposal_args("APPROVED_BEFORE_TERMINAL_SENTINEL"),
            ),
            final_body(),
            second_seen_tx,
            release_rx,
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let mut stream = provider
            .prompt(&session.id, PromptInput::text("research task"))
            .await
            .unwrap();
        let proposal = loop {
            let event = stream.next().await.unwrap();
            assert!(
                !matches!(event, AgentEvent::RunFinished { .. }),
                "planner finished before exposing the proposal"
            );
            if let AgentEvent::ProposedPlanUpdated { plan, .. } = event {
                break plan;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), second_seen_rx)
            .await
            .expect("planner reached its blocked post-proposal model request")
            .unwrap();
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Current,
                    },
                },
            )
            .await
            .expect("approval is currently accepted while the planner run is active");
        release_tx.send(()).unwrap();
        let mut terminal = false;
        while let Some(event) = stream.next().await {
            if matches!(
                event,
                AgentEvent::RunFinished { ref outcome, .. }
                    if outcome.status == RunStatus::Done
            ) {
                terminal = true;
                break;
            }
        }
        assert!(terminal);
        assert_eq!(server.await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn approval_event_is_deferred_until_the_next_execution_prompt() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args("DEFERRED_APPROVAL_EVENT_SENTINEL"),
                ),
                final_body(),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let planning_events = collect_prompt(&mut provider, &session, "research task").await;
        let proposal = planning_events
            .iter()
            .find_map(|event| match event {
                AgentEvent::ProposedPlanUpdated { plan, .. } => Some(plan.clone()),
                _ => None,
            })
            .expect("planner emitted a proposal");
        assert_eq!(proposal.status, ProposedPlanStatus::AwaitingDecision);
        assert!(planning_events.iter().all(|event| {
            !matches!(
                event,
                AgentEvent::ProposedPlanUpdated { plan, .. }
                    if plan.status == ProposedPlanStatus::Approved
            )
        }));

        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Current,
                    },
                },
            )
            .await
            .expect("approval response returns without an event stream");

        let execution_events =
            collect_prompt(&mut provider, &session, "Implement the approved plan.").await;
        let first_plan_event = execution_events.iter().find_map(|event| match event {
            AgentEvent::ProposedPlanUpdated { plan, .. } => Some(plan),
            _ => None,
        });
        assert!(matches!(
            first_plan_event,
            Some(plan) if plan.status == ProposedPlanStatus::Approved
        ));
        assert!(run_finished_done(&execution_events));
        assert_eq!(server.await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn approved_plan_does_not_constrain_execution_writes() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args(
                        "Create only planned-output.txt. Do not create unplanned-output.txt.",
                    ),
                ),
                final_body(),
                tool_call_sse(
                    "write-1",
                    "write_file",
                    json!({"path":"unplanned-output.txt","content":"outside the approved plan\n"}),
                ),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let proposal = prompt_for_plan(&mut provider, &session, "research task").await;
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Current,
                    },
                },
            )
            .await
            .unwrap();
        let events = collect_prompt(&mut provider, &session, "Implement the approved plan.").await;
        let _ = server.await.unwrap();
        assert!(run_finished_done(&events));
        assert_eq!(
            std::fs::read_to_string(directory.path().join("unplanned-output.txt")).unwrap(),
            "outside the approved plan\n"
        );
        assert!(!directory.path().join("planned-output.txt").exists());
    }

    #[tokio::test]
    async fn run_can_finish_done_with_the_approved_work_missing() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args("Create required-output.txt containing REQUIRED_PLAN_WORK."),
                ),
                final_body(),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let proposal = prompt_for_plan(&mut provider, &session, "research task").await;
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Current,
                    },
                },
            )
            .await
            .unwrap();
        let events = collect_prompt(&mut provider, &session, "Implement the approved plan.").await;
        let _ = server.await.unwrap();
        assert!(run_finished_done(&events));
        assert!(!directory.path().join("required-output.txt").exists());
    }

    #[tokio::test]
    async fn workspace_drift_does_not_invalidate_the_proposed_plan() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("contract.txt"), "version=1\n").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(
            listener,
            vec![
                tool_call_sse(
                    "plan-1",
                    "propose_plan",
                    proposal_args("Implement against contract.txt version=1."),
                ),
                final_body(),
                final_body(),
            ],
        ));
        let (mut provider, session) =
            provider(&directory, address, CollaborationMode::Plan, None).await;
        let proposal = prompt_for_plan(&mut provider, &session, "research task").await;
        std::fs::write(directory.path().join("contract.txt"), "version=2\n").unwrap();
        provider
            .respond(
                &session.id,
                ClientResponse::PlanDecision {
                    plan_id: proposal.id,
                    decision: PlanDecision::Implement {
                        context: agent_core::provider::PlanImplementationContext::Current,
                    },
                },
            )
            .await
            .expect("workspace drift currently does not invalidate approval");
        let events = collect_prompt(&mut provider, &session, "Implement the approved plan.").await;
        let captured = server.await.unwrap();
        assert!(run_finished_done(&events));
        assert!(role_text(&request_json(&captured[2]), "developer")
            .contains("Implement against contract.txt version=1."));
        assert_eq!(
            std::fs::read_to_string(directory.path().join("contract.txt")).unwrap(),
            "version=2\n"
        );
    }
}
