use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::*;

#[test]
fn public_plan_and_node_events_build_a_nested_outline() {
    let mut progress = starting_progress();
    assert!(apply_public_event(
        &mut progress,
        &json!({
            "type": "execution_plan_committed",
            "data": {"phases": [
                {"id":"plan","title":"Plan research","status":"completed","planned_steps":[]},
                {"id":"search","title":"Search and verify sources","status":"in_progress","public_narration":"Searching official sources","planned_steps":[
                    {"id":"read","title":"Read clarkchat.com","status":"pending"}
                ]}
            ]}
        })
    ));
    assert!(apply_public_event(
        &mut progress,
        &json!({
            "type": "execution_node_updated",
            "data": {"node_id":"node-1","phase_id":"search","planned_step_id":"read","status":"running","label":"Read clarkchat.com","summary":"Reading API and architecture pages"}
        })
    ));
    assert_eq!(progress.phases.len(), 2);
    assert_eq!(progress.phases[1].steps[0].status, ToolStatus::InProgress);
    assert_eq!(
        progress.latest_activity.as_deref(),
        Some("Reading API and architecture pages")
    );
}

#[test]
fn numeric_phase_ids_route_node_updates_to_the_declared_phase() {
    let mut progress = starting_progress();
    apply_public_event(
        &mut progress,
        &json!({
            "type":"execution_plan_committed",
            "data":{"phases":[
                {"id":1,"title":"First phase","status":"in_progress","planned_steps":[]},
                {"id":2,"title":"Second phase","status":"pending","planned_steps":[
                    {"id":"verify","title":"Verify claims","status":"pending"}
                ]}
            ]}
        }),
    );
    apply_public_event(
        &mut progress,
        &json!({
            "type":"execution_node_updated",
            "data":{"node_id":"verify","phase_id":2,"planned_step_id":"verify","status":"running","label":"Verify claims"}
        }),
    );

    assert_eq!(progress.phases[0].steps.len(), 0);
    assert_eq!(progress.phases[1].steps[0].status, ToolStatus::InProgress);
}

#[test]
fn subagent_updates_are_typed_and_stable() {
    let mut progress = starting_progress();
    for status in ["running", "completed"] {
        assert!(apply_public_event(
            &mut progress,
            &json!({
                "type":"subagent_event",
                "data":{"group_id":"research","row_index":0,"label":"Vorflux documentation","status":status,"activity":"Reading official docs","summary":"Verified primary claims"}
            })
        ));
    }
    assert_eq!(progress.agents.len(), 1);
    assert_eq!(progress.agents[0].status, ToolStatus::Completed);
    assert_eq!(
        progress.agents[0].activity.as_deref(),
        Some("Reading official docs")
    );
}

#[test]
fn terminal_public_events_distinguish_failure_from_cancellation() {
    let mut failed = starting_progress();
    apply_public_event(
        &mut failed,
        &json!({"type":"run_failed","data":{"status":"failed","error":"Research failed safely"}}),
    );
    assert_eq!(failed.status, ToolStatus::Failed);

    let mut cancelled = starting_progress();
    apply_public_event(
        &mut cancelled,
        &json!({"type":"run_failed","data":{"status":"cancelled"}}),
    );
    assert_eq!(cancelled.status, ToolStatus::Cancelled);
    assert_eq!(
        cancelled.latest_activity.as_deref(),
        Some("Research cancelled")
    );
}

#[test]
fn duplicate_or_unknown_events_do_not_advance_revision() {
    let mut progress = starting_progress();
    let unknown = json!({"type":"debug_trace","data":{"secret":"hidden"}});
    let revision = progress.revision;
    assert!(!apply_public_event(&mut progress, &unknown));
    assert_eq!(progress.revision, revision);
    assert!(progress.phases.is_empty());
    assert!(progress.agents.is_empty());
}

#[test]
fn completed_response_extracts_only_public_output_text() {
    let response = json!({
        "status":"completed",
        "output":[{"content":[{"type":"output_text","text":"Findings with citations"}]}]
    });
    assert_eq!(
        terminal_response(&response).unwrap().as_deref(),
        Some("Findings with citations")
    );
}

#[tokio::test]
async fn background_response_streams_public_outline_and_returns_findings() {
    let (base_url, requests) = endpoint(vec![
        ok(json!({"id":"resp_1","status":"in_progress"})),
        ok(json!({
            "data":[
                {"sequence":1,"type":"execution_plan_committed","data":{"phases":[
                    {"id":"plan","title":"Plan research","status":"completed","planned_steps":[]},
                    {"id":"search","title":"Search and verify sources","status":"in_progress","planned_steps":[
                        {"id":"read","title":"Read clarkchat.com","status":"pending"}
                    ]}
                ]}},
                {"sequence":2,"type":"execution_node_updated","data":{"node_id":"read","phase_id":"search","planned_step_id":"read","status":"running","label":"Read clarkchat.com","summary":"Reading API and architecture pages"}},
                {"sequence":3,"type":"subagent_event","data":{"group_id":"research","row_index":0,"label":"Vorflux documentation","status":"running","activity":"Reading official docs"}}
            ],
            "next_after_seq":3
        })),
        ok(json!({
            "data":[{"sequence":4,"type":"run_completed","data":{"status":"completed","summary":"Research complete"}}],
            "next_after_seq":4
        })),
        ok(completed("Final cited findings")),
    ])
    .await;
    let client = test_client(base_url);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let answer = client
        .research(
            "Investigate Vorflux",
            &CancellationToken::new(),
            move |progress| {
                sink.lock().unwrap().push(progress);
            },
        )
        .await
        .unwrap();

    assert_eq!(answer, "Final cited findings");
    let snapshots = seen.lock().unwrap();
    let final_outline = snapshots
        .iter()
        .rev()
        .find(|progress| !progress.phases.is_empty())
        .unwrap();
    assert_eq!(
        final_outline.phases[1].steps[0].status,
        ToolStatus::InProgress
    );
    assert_eq!(final_outline.agents[0].label, "Vorflux documentation");
    drop(snapshots);
    let requests = requests.lock().unwrap();
    assert!(requests[0].starts_with("POST /v1/responses HTTP/1.1"));
    assert!(requests[0].contains("\"background\":true"));
    assert!(requests[0].contains("\"model\":\"clark\""));
    assert!(requests[1].contains("GET /v1/responses/resp_1/events?after_seq=0&limit=200"));
    assert!(requests[2].contains("after_seq=3"));
}

#[tokio::test]
async fn event_poll_failure_falls_back_to_response_status() {
    let (base_url, _) = endpoint(vec![
        ok(json!({"id":"resp_2","status":"in_progress"})),
        response(
            404,
            json!({"error":{"message":"progress endpoint unavailable"}}),
        ),
        ok(completed("Recovered findings")),
    ])
    .await;
    let answer = test_client(base_url)
        .research("Investigate", &CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(answer, "Recovered findings");
}

#[tokio::test]
async fn replayed_event_sequences_are_deduplicated() {
    let (base_url, _) = endpoint(vec![
        ok(json!({"id":"resp_replay","status":"in_progress"})),
        ok(json!({
            "data":[
                {"sequence":1,"type":"run_note","data":{"summary":"Reading primary sources"}},
                {"sequence":1,"type":"run_note","data":{"summary":"Duplicate should be ignored"}},
                {"sequence":2,"type":"run_completed","data":{"summary":"Research complete"}}
            ],
            "next_after_seq":2
        })),
        ok(completed("Deduplicated findings")),
    ])
    .await;
    let snapshots = Arc::new(Mutex::new(Vec::new()));
    let sink = snapshots.clone();
    let answer = test_client(base_url)
        .research("Investigate", &CancellationToken::new(), move |progress| {
            sink.lock().unwrap().push(progress);
        })
        .await
        .unwrap();

    assert_eq!(answer, "Deduplicated findings");
    assert!(
        snapshots
            .lock()
            .unwrap()
            .iter()
            .all(|progress| progress.latest_activity.as_deref()
                != Some("Duplicate should be ignored"))
    );
}

#[tokio::test]
async fn cancellation_interrupts_poll_wait() {
    let (base_url, _) = endpoint(vec![
        ok(json!({"id":"resp_3","status":"in_progress"})),
        ok(json!({"data":[],"next_after_seq":0})),
    ])
    .await;
    let client = ClarkResearchClient::new(AgenticClarkConfig {
        base_url,
        api_key: Some("ck_test".to_string()),
        model: "clark".to_string(),
    })
    .unwrap()
    .with_test_timing(
        Duration::from_secs(10),
        Duration::from_secs(10),
        Duration::from_secs(20),
    );
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let task =
        tokio::spawn(async move { client.research("Investigate", &task_cancel, |_| {}).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    cancel.cancel();
    assert_eq!(task.await.unwrap().unwrap_err(), "Clark research cancelled");
}

fn test_client(base_url: String) -> ClarkResearchClient {
    ClarkResearchClient::new(AgenticClarkConfig {
        base_url,
        api_key: Some("ck_test".to_string()),
        model: "clark".to_string(),
    })
    .unwrap()
    .with_test_timing(
        Duration::from_millis(2),
        Duration::from_millis(50),
        Duration::from_secs(2),
    )
}

fn completed(text: &str) -> Value {
    json!({
        "id":"resp_done",
        "status":"completed",
        "output":[{"content":[{"type":"output_text","text":text}]}]
    })
}

fn ok(body: Value) -> Vec<u8> {
    response(200, body)
}

fn response(status: u16, body: Value) -> Vec<u8> {
    let body = body.to_string();
    let reason = if status == 200 {
        "OK"
    } else {
        "Service Unavailable"
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

async fn endpoint(responses: Vec<Vec<u8>>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let server_requests = requests.clone();
    tokio::spawn(async move {
        let mut responses = VecDeque::from(responses);
        while let Some(response) = responses.pop_front() {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            server_requests.lock().unwrap().push(request);
            stream.write_all(&response).await.unwrap();
            stream.flush().await.unwrap();
        }
    });
    (format!("http://{address}/v1"), requests)
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer).await.unwrap();
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        let Some(headers_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..headers_end]);
        let content_length = headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        });
        if bytes.len() >= headers_end + 4 + content_length.unwrap_or(0) {
            break;
        }
    }
    String::from_utf8(bytes).unwrap()
}
