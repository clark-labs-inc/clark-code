use super::*;

/// Saturation behavior needs a single-permit worker: a full worker must keep
/// answering health pings (or the desktop replaces it mid-work), refuse extra
/// work with a typed `busy`, and survive an oversized request line.
#[tokio::test(flavor = "multi_thread")]
async fn saturated_worker_stays_healthy_and_survives_bad_input() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("README.md"), "saturation fixture\n").unwrap();
    let project = project.canonicalize().unwrap();

    let model = spawn_model(vec![ScriptedTurn {
        stall: Duration::from_secs(4),
        ..assistant_turn("slow but done")
    }]);
    // One permit and a small request bound, so both edges are reachable.
    let config = write_config(temp.path(), &project, &model.base_url, 1, 8 * 1024);
    let mut worker = WorkerClient::spawn(&config).await;

    let (_, opened) = worker
        .call(WorkerClient::invoke(
            "open-sat",
            "coding",
            "session.open",
            Some("sim"),
            json!({ "session_id": "sat-session" }),
        ))
        .await;
    result_data(&opened, "session.open");

    // Occupy the only permit with the stalled prompt.
    let prompt = WorkerClient::invoke(
        "prompt-sat",
        "coding",
        "session.prompt",
        Some("sim"),
        json!({
            "session_id": "sat-session",
            "input": { "blocks": [{ "type": "text", "text": "take your time" }] }
        }),
    );
    let mut prompt_rx = worker.send(&prompt).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 1. Health pings must answer while saturated — a busy worker is alive.
    let pong = worker.ping("sat-ping").await;
    result_data(&pong, "ping under saturation");

    // 2. Real work beyond the permit is refused with a typed busy.
    let (_, busy) = worker
        .call(WorkerClient::executor(
            "sat-read",
            "fs/read",
            json!({ "path": project.join("README.md").to_string_lossy() }),
        ))
        .await;
    assert_eq!(error_code(&busy, "extra work while saturated"), "busy");

    // 3. An oversized request line is that caller's error, not a process exit.
    let oversized = format!(
        "{{\"schema_version\":{PROTOCOL_VERSION},\"request_id\":\"huge\",\"junk\":\"{}\"}}",
        "x".repeat(64 * 1024)
    );
    let mut huge_rx = worker.open_route("").await;
    worker.send_line(&oversized).await;
    let refusal = tokio::time::timeout(STEP_TIMEOUT, huge_rx.recv())
        .await
        .expect("oversized refusal within budget")
        .expect("stream open");
    assert_eq!(
        error_code(&refusal, "oversized request"),
        "request_too_large"
    );

    // The stalled prompt still completes afterwards: nothing was killed.
    let settled = loop {
        let frame = tokio::time::timeout(STEP_TIMEOUT, prompt_rx.recv())
            .await
            .expect("stalled prompt settles")
            .expect("prompt stream open");
        match frame {
            Response::Progress { .. } => continue,
            terminal => break terminal,
        }
    };
    let settled = result_data(&settled, "stalled prompt terminal");
    assert_eq!(settled["outcome"]["status"], json!("done"));

    let stderr = worker.stderr.lock().await.clone();
    assert!(
        !stderr.contains("panicked"),
        "worker panicked under saturation:\n{stderr}"
    );
}
