use super::*;
use agent_core::{
    apply, AgentEvent, GoalState, GoalStatus, ProviderFailureClass, ProviderIncident,
    ProviderIncidentCategory, ProviderIncidentScope, ProviderIncidentStatus,
    ProviderRequestDiagnostics, ProviderRetryCounts, RunFailureKind, RunId, RunStatus, SessionId,
};
use std::time::Duration;

fn config() -> CloudTrajectoryConfig {
    CloudTrajectoryConfig {
        title: "Test".into(),
        provider: "local".into(),
        project: None,
        repository_fingerprint: None,
        remote_host: None,
        mode: None,
        metadata: Value::Null,
    }
}

fn specialist_config() -> CloudTrajectoryConfig {
    CloudTrajectoryConfig {
        metadata: json!({
            "specialistContext": {
                "kind": "spec",
                "workflow": "spec:spec"
            }
        }),
        ..config()
    }
}

fn request(run_id: &str) -> AppendRequest {
    let event = AgentEvent::RunStarted {
        run: RunId::new(run_id),
    };
    request_with_event(run_id, event)
}

fn request_with_event(run_id: &str, event: AgentEvent) -> AppendRequest {
    AppendRequest {
        conversation: super::super::Conversation {
            title: "Test".into(),
            provider: "local".into(),
            project: None,
            repository_fingerprint: None,
            remote_host: None,
            mode: None,
        },
        events: vec![super::super::EventRecord {
            event_id: uuid::Uuid::new_v4(),
            run_id: Some(run_id.into()),
            event_kind: "run_started".into(),
            recorded_at_unix_ms: 10,
            payload: json!({"event": serde_json::to_value(event).unwrap()}),
        }],
    }
}

#[tokio::test]
async fn legacy_compaction_folds_acknowledged_events_before_reclaiming_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.sqlite3");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(schema::SCHEMA).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA auto_vacuum", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0,
        );
    }
    let outbox = TrajectoryOutbox::new(path.clone(), "owner", "session-legacy");
    let mut base = Snapshot::new();
    base.session = Some(SessionId::new("session-legacy"));
    outbox.initialize(&config(), &base, 0).await.unwrap();
    let batch = outbox.enqueue(&request("run-legacy")).await.unwrap();
    outbox.acknowledge(&batch.batch_id).await.unwrap();

    assert!(storage::migrate_legacy_database(&path).unwrap());
    let conn = open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM trajectory_outbox", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0,
    );
    assert_eq!(
        conn.query_row("PRAGMA auto_vacuum", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2,
    );
    let snapshot: Vec<u8> = conn
        .query_row(
            "SELECT base_snapshot_json FROM journal_conversation WHERE conversation_id = 'session-legacy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let snapshot: Snapshot = serde_json::from_slice(&snapshot).unwrap();
    assert_eq!(snapshot.history_checkpoint, Some(batch.local_seq));
    assert_eq!(
        snapshot
            .runs
            .get(&RunId::new("run-legacy"))
            .map(|run| run.status),
        Some(RunStatus::Running),
    );
}

#[tokio::test]
async fn crash_recovery_replays_uncheckpointed_batches_and_marks_run_interrupted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.sqlite3");
    let outbox = TrajectoryOutbox::new(path.clone(), "user@example.com", "session-1");
    let mut base = Snapshot::new();
    base.session = Some(SessionId::new("session-1"));
    base.goal = Some(GoalState {
        id: "goal-1".into(),
        objective: "finish the work".into(),
        status: GoalStatus::Active,
        run: Some(RunId::new("run-1")),
        tokens_used: 0,
        time_used_seconds: 0,
        continuations: 0,
        updated_at_ms: 1,
        blocker_reason: None,
    });
    outbox.initialize(&config(), &base, 0).await.unwrap();
    outbox.enqueue(&request("run-1")).await.unwrap();
    let incident = ProviderIncident {
        id: "incident-1".into(),
        status: ProviderIncidentStatus::Retrying,
        scope: ProviderIncidentScope::ModelRequest,
        failure_class: ProviderFailureClass::TransientTransport,
        category: ProviderIncidentCategory::Timeout,
        message: "Model connection timed out.".into(),
        detail: "gateway timeout".into(),
        model: "test-model".into(),
        provider_route: "gateway.test/v1".into(),
        provider_status: Some(504),
        provider_error_type: Some("upstream_timeout".into()),
        request: ProviderRequestDiagnostics {
            idempotency_key: "request-1".into(),
            provider_request_id: Some("provider-1".into()),
            attempts: 1,
            max_attempts: 4,
            retries: ProviderRetryCounts {
                transient: 1,
                ..Default::default()
            },
            output_started: false,
            started_at_ms: 5,
        },
        execution_recovery: None,
        observed_at_ms: 8,
        updated_at_ms: 8,
        completed_at_ms: None,
    };
    outbox
        .enqueue(&request_with_event(
            "run-1",
            AgentEvent::ProviderIncidentUpdated {
                run: RunId::new("run-1"),
                incident: incident.clone(),
            },
        ))
        .await
        .unwrap();

    let recovered = recover_snapshot(
        path.clone(),
        "user@example.com".into(),
        "session-1".into(),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    let run = &recovered.snapshot.runs[&RunId::new("run-1")];
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(
        run.outcome.as_ref().unwrap().failure_kind,
        Some(RunFailureKind::RuntimeInterrupted)
    );
    let recovered_incident = &recovered.snapshot.provider_incidents[&incident.id];
    assert_eq!(
        recovered_incident.status,
        ProviderIncidentStatus::Interrupted
    );
    assert_eq!(recovered_incident.completed_at_ms, None);
    assert!(recovered.pending);
    assert!(recovered.needs_snapshot_publication);
    let recovered_goal = recovered.snapshot.goal.as_ref().unwrap();
    assert_eq!(recovered_goal.status, GoalStatus::Blocked);
    assert_eq!(
        recovered_goal.blocker_reason.as_deref(),
        Some("Clark Code restarted before the goal finished. Continue from the saved history.")
    );
    let metadata = recovered.metadata.as_ref().unwrap();
    assert_eq!(metadata["id"], "session-1");
    assert_eq!(metadata["title"], "Test");
    assert_eq!(metadata["provider"], "local");
    assert_eq!(metadata["rev"], 0);
    assert!(metadata["createdAt"].as_i64().is_some());
    assert!(metadata["updatedAt"].as_i64().is_some());

    let pending = outbox.pending().await.unwrap();
    assert_eq!(pending.len(), 3);
    let terminal_events = pending.last().unwrap().request.events.iter().map(|record| {
        serde_json::from_value::<AgentEvent>(record.payload["event"].clone()).unwrap()
    });
    assert!(terminal_events.into_iter().any(|event| matches!(
        event,
        AgentEvent::ProviderIncidentUpdated { incident, .. }
            if incident.status == ProviderIncidentStatus::Interrupted
                && incident.completed_at_ms.is_none()
    )));
    let terminal_events = pending.last().unwrap().request.events.iter().map(|record| {
        serde_json::from_value::<AgentEvent>(record.payload["event"].clone()).unwrap()
    });
    assert!(terminal_events.into_iter().any(|event| matches!(
        event,
        AgentEvent::GoalUpdated { goal, .. }
            if goal.status == GoalStatus::Blocked
                && goal.blocker_reason.as_deref()
                    == Some("Clark Code restarted before the goal finished. Continue from the saved history.")
    )));

    for batch in &pending {
        outbox.acknowledge(&batch.batch_id).await.unwrap();
    }
    let acknowledged = recover_snapshot(path, "user@example.com".into(), "session-1".into(), None)
        .await
        .unwrap()
        .unwrap();
    assert!(!acknowledged.pending);
    assert!(
        acknowledged.needs_snapshot_publication,
        "event acknowledgement must not hide a terminal snapshot that still needs publishing"
    );
}

#[tokio::test]
async fn checkpoint_discards_only_acknowledged_covered_batches() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.sqlite3");
    let outbox = TrajectoryOutbox::new(path.clone(), "owner", "session");
    outbox
        .initialize(&config(), &Snapshot::new(), 0)
        .await
        .unwrap();
    let first = outbox.enqueue(&request("first")).await.unwrap();
    let second = outbox.enqueue(&request("second")).await.unwrap();
    outbox.acknowledge(&first.batch_id).await.unwrap();
    outbox.acknowledge(&second.batch_id).await.unwrap();
    checkpoint_snapshot(
        path,
        "owner".into(),
        "session".into(),
        json!({"id":"session"}),
        Snapshot::new(),
        99,
        first.local_seq,
        false,
    )
    .await
    .unwrap();
    let conn = open(&outbox.path).unwrap();
    let remaining: Vec<i64> = conn
        .prepare("SELECT local_seq FROM trajectory_outbox ORDER BY local_seq")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(remaining, vec![second.local_seq]);
}

#[tokio::test]
async fn equal_cloud_revision_prefers_the_newer_local_read_model() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.sqlite3");
    let mut local = Snapshot::new();
    local.session = Some(SessionId::new("local-checkpoint"));
    checkpoint_snapshot(
        path.clone(),
        "owner".into(),
        "conversation".into(),
        json!({"id":"conversation"}),
        local,
        7,
        0,
        false,
    )
    .await
    .unwrap();

    let mut cloud = Snapshot::new();
    cloud.session = Some(SessionId::new("older-cloud-snapshot"));
    let recovered = recover_snapshot(
        path,
        "owner".into(),
        "conversation".into(),
        Some((cloud, 7)),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(
        recovered.snapshot.session.as_ref().map(SessionId::as_str),
        Some("local-checkpoint")
    );
}

#[tokio::test]
async fn snapshot_publication_waits_for_its_exact_acknowledged_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.sqlite3");
    let outbox = TrajectoryOutbox::new(path.clone(), "owner", "session");
    outbox
        .initialize(&config(), &Snapshot::new(), 0)
        .await
        .unwrap();
    let first = outbox.enqueue(&request("first")).await.unwrap();
    let second = outbox.enqueue(&request("second")).await.unwrap();

    let pending = wait_for_acknowledged_prefix(
        path.clone(),
        "owner".into(),
        "session".into(),
        first.local_seq,
        Duration::ZERO,
    )
    .await
    .unwrap_err();
    assert!(pending.contains("still syncing"));

    outbox.acknowledge(&first.batch_id).await.unwrap();
    wait_for_acknowledged_prefix(
        path,
        "owner".into(),
        "session".into(),
        first.local_seq,
        Duration::ZERO,
    )
    .await
    .unwrap();
    assert!(
        outbox
            .pending()
            .await
            .unwrap()
            .iter()
            .any(|batch| batch.batch_id == second.batch_id),
        "a later batch must not block an older snapshot prefix"
    );
}

#[tokio::test]
async fn conflicted_branch_is_retained_for_sync_but_not_overlaid_on_cloud_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.sqlite3");
    let outbox = TrajectoryOutbox::new(path.clone(), "owner", "session");
    let mut base = Snapshot::new();
    base.session = Some(SessionId::new("session"));
    outbox.initialize(&config(), &base, 5).await.unwrap();
    outbox.enqueue(&request("divergent-run")).await.unwrap();

    let recovered = recover_snapshot(path, "owner".into(), "session".into(), Some((base, 6)))
        .await
        .unwrap()
        .unwrap();
    assert!(
        recovered.snapshot.runs.is_empty(),
        "cloud snapshot must win a CAS conflict"
    );
    assert!(
        recovered.pending,
        "unacknowledged audit events remain queued"
    );
}

#[tokio::test]
async fn successful_cloud_list_suppresses_deleted_cache_but_keeps_local_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.sqlite3");
    let outbox = TrajectoryOutbox::new(path.clone(), "owner", "session");
    outbox
        .initialize(&config(), &Snapshot::new(), 7)
        .await
        .unwrap();

    let online = merge_local_summaries(path.clone(), "owner".into(), vec![], true)
        .await
        .unwrap();
    assert!(
        online.is_empty(),
        "a cloud-deleted acknowledged cache row stays deleted"
    );

    outbox.enqueue(&request("local-run")).await.unwrap();
    let with_local_work = merge_local_summaries(path.clone(), "owner".into(), vec![], true)
        .await
        .unwrap();
    assert_eq!(
        with_local_work.len(),
        1,
        "unsynced work remains recoverable"
    );

    let offline = merge_local_summaries(path, "owner".into(), vec![], false)
        .await
        .unwrap();
    assert_eq!(
        offline.len(),
        1,
        "offline mode can use the acknowledged cache"
    );
}

#[tokio::test]
async fn cloud_list_recovers_a_missing_specialist_binding_from_owner_scoped_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.sqlite3");
    let outbox = TrajectoryOutbox::new(path.clone(), "owner", "spec-session");
    outbox
        .initialize(&specialist_config(), &Snapshot::new(), 7)
        .await
        .unwrap();

    let rows = merge_local_summaries(
        path,
        "owner".into(),
        vec![json!({
            "id": "spec-session",
            "title": "Sharing spec",
            "provider": "local",
            "rev": 7,
            "createdAt": 1,
            "updatedAt": 2
        })],
        true,
    )
    .await
    .unwrap();

    assert_eq!(
        rows[0]["specialistContext"],
        json!({"kind": "spec", "workflow": "spec:spec"})
    );
}

#[tokio::test]
async fn cloud_list_recovers_legacy_spec_binding_from_typed_skill_reference() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.sqlite3");
    let outbox = TrajectoryOutbox::new(path.clone(), "owner", "legacy-spec-session");
    outbox
        .initialize(&config(), &Snapshot::new(), 7)
        .await
        .unwrap();
    let snapshot = json!({
        "timeline": [{
            "item": "message",
            "role": "user",
            "blocks": [{
                "type": "skill_reference",
                "name": "spec:spec"
            }]
        }]
    });
    open(&path)
        .unwrap()
        .execute(
            "UPDATE journal_conversation SET base_snapshot_json = ?1",
            params![serde_json::to_vec(&snapshot).unwrap()],
        )
        .unwrap();

    let rows = merge_local_summaries(
        path,
        "owner".into(),
        vec![json!({
            "id": "legacy-spec-session",
            "title": "Sharing spec",
            "provider": "local",
            "rev": 7,
            "createdAt": 1,
            "updatedAt": 2
        })],
        true,
    )
    .await
    .unwrap();

    assert_eq!(
        rows[0]["specialistContext"],
        json!({"kind": "spec", "workflow": "spec:spec"})
    );
}

#[tokio::test]
async fn cloud_only_live_run_is_not_misclassified_as_a_local_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.sqlite3");
    let mut cloud = Snapshot::new();
    apply(
        &mut cloud,
        &AgentEvent::RunStarted {
            run: RunId::new("remote-run"),
        },
    );

    let recovered = recover_snapshot(path, "owner".into(), "session".into(), Some((cloud, 42)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        recovered.snapshot.runs[&RunId::new("remote-run")].status,
        RunStatus::Running
    );
    assert!(!recovered.pending);
    assert!(!recovered.needs_snapshot_publication);
}
