use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn full_stack_worker_simulation() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "-q", "--initial-branch=main"]);
    std::fs::write(project.join("README.md"), "sim project\n").unwrap();
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/lib.rs"), "pub fn sim() {}\n").unwrap();
    std::fs::write(project.join("docs.md"), "guide\n").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "initial"]);
    let project = project.canonicalize().unwrap();

    let model = spawn_model(vec![
        // Journey turn 1: write a file (permission-gated -> respond flow).
        tool_call_turn(
            "write_file",
            &json!({ "path": "notes/plan.md", "content": "# Plan\nstep one\n" }),
        ),
        // Journey turn 2: an auto-approved shell command.
        tool_call_turn("bash", &json!({ "command": "echo built > build.log" })),
        // Journey turn 3: settle.
        assistant_turn("Wrote the plan and produced build.log."),
        // Cancel phase: a completion that stalls until after the cancel lands.
        ScriptedTurn {
            stall: Duration::from_secs(20),
            body: sse(&[]),
        },
        // Post-cancel health turn: settle immediately.
        assistant_turn("Recovered after cancellation."),
    ]);

    let config = write_config(temp.path(), &project, &model.base_url, 8, 1024 * 1024);
    let mut worker = WorkerClient::spawn(&config).await;

    // ------------------------------------------------------------------ boot
    let pong = worker.ping("boot-ping").await;
    let pong = result_data(&pong, "boot ping");
    assert_eq!(
        pong["execution_residency"],
        json!("remote_worker"),
        "the ping receipt proves tool/model residency: {pong}"
    );

    // ------------------------------------------------- executor wire surface
    let (_, walk) = worker
        .call(WorkerClient::executor(
            "walk-bounded",
            "fs/walk",
            json!({ "path": project.to_string_lossy(), "max_entries": 1 }),
        ))
        .await;
    let walk = result_data(&walk, "bounded walk");
    assert_eq!(walk["truncated"], json!(true));
    assert_eq!(walk["entries"].as_array().unwrap().len(), 1);

    // A real secret outside the root: the read must fail as a TYPED caller
    // error, and no variant of the response may carry the bytes.
    std::fs::write(
        project.parent().unwrap().join("outside.txt"),
        "TOP-SECRET-OUTSIDE",
    )
    .unwrap();
    let (_, escape) = worker
        .call(WorkerClient::executor(
            "confinement-escape",
            "fs/read",
            json!({ "path": project.join("../outside.txt").to_string_lossy() }),
        ))
        .await;
    let escape_code = error_code(&escape, "parent-dir escape");
    assert_eq!(
        escape_code, "invalid_input",
        "escapes must be typed rejections"
    );
    assert!(
        !serde_json::to_string(&escape)
            .unwrap()
            .contains("TOP-SECRET"),
        "confinement failure leaked file content"
    );

    // --------------------------------------------------------- open session
    let (_, opened) = worker
        .call(WorkerClient::invoke(
            "open-1",
            "coding",
            "session.open",
            Some("sim"),
            json!({ "session_id": "sim-session" }),
        ))
        .await;
    let opened = result_data(&opened, "session.open");
    assert_eq!(opened["session_id"], json!("sim-session"));

    // ------------------------------------------------ the full working turn
    // The prompt streams; write_file raises a permission request that we must
    // answer over the wire from a SECOND in-flight request while the first is
    // still streaming. Drive both by hand rather than through `call`.
    let prompt = WorkerClient::invoke(
        "prompt-1",
        "coding",
        "session.prompt",
        Some("sim"),
        json!({
            "session_id": "sim-session",
            "input": { "blocks": [{ "type": "text", "text": "write the plan file, then build" }] }
        }),
    );
    let mut prompt_rx = worker.send(&prompt).await;

    let mut progress: Vec<Response> = Vec::new();
    let mut responded_permission = false;
    let terminal = loop {
        let frame = tokio::time::timeout(STEP_TIMEOUT, prompt_rx.recv())
            .await
            .expect("prompt frame within budget")
            .expect("prompt stream open");
        match frame {
            Response::Progress { .. } => {
                progress.push(frame);
                if responded_permission {
                    continue;
                }
                let (snapshot, _) = fold_events(&progress);
                if let Some(pending) = snapshot.pending_permission {
                    // Choose the allow-once option, exactly as the desktop does.
                    let option = pending
                        .options
                        .iter()
                        .find(|option| option.kind == PermissionOptionKind::AllowOnce)
                        .unwrap_or_else(|| pending.options.first().expect("options exist"));
                    let respond = WorkerClient::invoke(
                        "respond-1",
                        "coding",
                        "session.respond",
                        Some("sim"),
                        json!({
                            "session_id": "sim-session",
                            "response": {
                                "kind": "permission",
                                "request": pending.id,
                                "option": option.id,
                            }
                        }),
                    );
                    let (_, respond_terminal) = worker.call(respond).await;
                    result_data(&respond_terminal, "session.respond");
                    responded_permission = true;
                }
            }
            terminal => break terminal,
        }
    };

    assert!(
        responded_permission,
        "the gated write_file never raised a permission request"
    );
    let outcome = result_data(&terminal, "prompt terminal");
    assert_eq!(outcome["outcome"]["status"], json!("done"), "{outcome}");
    assert!(
        outcome.get("snapshot").is_none(),
        "terminal frames must not ship the whole conversation"
    );

    // Cross-layer truth: the tools really ran in the real repository.
    assert_eq!(
        std::fs::read_to_string(project.join("notes/plan.md")).unwrap(),
        "# Plan\nstep one\n"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("build.log"))
            .unwrap()
            .trim(),
        "built"
    );

    // Stream-level invariants over the whole run.
    let (snapshot, events) = fold_events(&progress);
    let started = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::RunStarted { .. }))
        .count();
    let finished = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::RunFinished { .. }))
        .count();
    assert_eq!(
        (started, finished),
        (1, 1),
        "exactly one run start and finish"
    );
    assert!(
        matches!(events.last(), Some(AgentEvent::RunFinished { .. })),
        "the terminal event closes the stream"
    );
    let checkpoint = events.iter().find_map(|event| match event {
        AgentEvent::Checkpoint { id, .. } => Some(id.clone()),
        _ => None,
    });
    let checkpoint = checkpoint.expect("a git repo run must take a checkpoint");
    let refs = git(&project, &["for-each-ref", "refs/agent/checkpoints/"]);
    assert!(
        refs.contains(&checkpoint),
        "checkpoint ref must exist in the real repository: {refs}"
    );
    assert!(snapshot.pending_permission.is_none(), "gate resolved");
    assert!(
        snapshot.timeline.iter().any(|item| {
            serde_json::to_string(item)
                .unwrap()
                .contains("Wrote the plan")
        }),
        "final answer reached the projected timeline"
    );

    // The scripted model saw the tool results round-trip.
    let requests = model.requests.lock().await;
    assert!(requests.len() >= 3, "three journey completions expected");
    let second = serde_json::to_string(&requests[1]).unwrap();
    assert!(
        second.contains("notes/plan.md") || second.contains("plan.md"),
        "turn 2 must carry the write_file result back to the model"
    );
    drop(requests);

    // ------------------------------------------------------------ cancel run
    let stall_prompt = WorkerClient::invoke(
        "prompt-cancel",
        "coding",
        "session.prompt",
        Some("sim"),
        json!({
            "session_id": "sim-session",
            "input": { "blocks": [{ "type": "text", "text": "this one gets cancelled" }] }
        }),
    );
    let mut cancel_rx = worker.send(&stall_prompt).await;
    // Give the request time to reach the stalled completion.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let cancel = Request {
        schema_version: PROTOCOL_VERSION,
        request_id: "cancel-1".into(),
        command: RequestCommand::Cancel {
            target_request_id: "prompt-cancel".into(),
        },
    };
    let (_, cancel_ack) = worker.call(cancel).await;
    result_data(&cancel_ack, "cancel acknowledged");
    let cancelled = loop {
        let frame = tokio::time::timeout(STEP_TIMEOUT, cancel_rx.recv())
            .await
            .expect("cancel terminal within budget")
            .expect("cancel stream open");
        match frame {
            Response::Progress { .. } => continue,
            terminal => break terminal,
        }
    };
    assert_eq!(
        error_code(&cancelled, "cancelled prompt"),
        "cancelled",
        "cancellation is a typed terminal"
    );

    // The session survives cancellation.
    let (_, after) = worker
        .call(WorkerClient::invoke(
            "prompt-after-cancel",
            "coding",
            "session.prompt",
            Some("sim"),
            json!({
                "session_id": "sim-session",
                "input": { "blocks": [{ "type": "text", "text": "are you alive" }] }
            }),
        ))
        .await;
    let after = result_data(&after, "post-cancel prompt");
    assert_eq!(after["outcome"]["status"], json!("done"));

    // ----------------------------------------------- crash + receipt replay
    let (_, first_read) = worker
        .call(WorkerClient::executor(
            "replay-1",
            "fs/read",
            json!({ "path": project.join("README.md").to_string_lossy() }),
        ))
        .await;
    let first_read = result_data(&first_read, "pre-crash read");

    let stderr_before = worker.stderr.lock().await.clone();
    assert!(
        !stderr_before.contains("panicked"),
        "worker panicked during the journey:\n{stderr_before}"
    );
    worker.child.start_kill().expect("kill worker");
    let _ = worker.child.wait().await;

    let mut worker = WorkerClient::spawn(&config).await;
    result_data(&worker.ping("post-restart-ping").await, "restart ping");

    // Same id, same params: the durable receipt replays the exact result.
    let (_, replayed) = worker
        .call(WorkerClient::executor(
            "replay-1",
            "fs/read",
            json!({ "path": project.join("README.md").to_string_lossy() }),
        ))
        .await;
    let replayed = result_data(&replayed, "replayed read");
    assert_eq!(
        replayed, first_read,
        "a restarted worker must replay the identical receipt"
    );

    // Same id, different params: typed conflict, not silent re-execution.
    let (_, conflict) = worker
        .call(WorkerClient::executor(
            "replay-1",
            "fs/read",
            json!({ "path": project.join("build.log").to_string_lossy() }),
        ))
        .await;
    assert_eq!(
        error_code(&conflict, "conflicting replay"),
        "request_id_conflict"
    );

    // Sessions are process state: after a crash the old session is gone and
    // says so in a typed way (the desktop reopens through session recovery).
    let (_, stale) = worker
        .call(WorkerClient::invoke(
            "prompt-stale",
            "coding",
            "session.prompt",
            Some("sim"),
            json!({
                "session_id": "sim-session",
                "input": { "blocks": [{ "type": "text", "text": "hello?" }] }
            }),
        ))
        .await;
    let stale_code = error_code(&stale, "stale session prompt");
    assert_eq!(stale_code, "invalid_input");

    // -------------------------------------------------------------- shutdown
    let shutdown = Request {
        schema_version: PROTOCOL_VERSION,
        request_id: "shutdown-1".into(),
        command: RequestCommand::Shutdown,
    };
    let (_, down) = worker.call(shutdown).await;
    result_data(&down, "shutdown acknowledged");
    let status = tokio::time::timeout(STEP_TIMEOUT, worker.child.wait())
        .await
        .expect("worker exits after shutdown")
        .expect("worker exit status");
    assert!(status.success(), "clean shutdown exit: {status:?}");

    let stderr = worker.stderr.lock().await.clone();
    assert!(
        !stderr.contains("panicked"),
        "restarted worker panicked:\n{stderr}"
    );
}
