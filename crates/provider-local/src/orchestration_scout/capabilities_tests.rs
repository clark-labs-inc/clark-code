use super::*;
use crate::background::BackgroundTasks;
use crate::exec::LocalExecutor;
use crate::loop_state::SessionState;
use crate::sandbox::Sandbox;
use crate::tools::ReadTracker;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

fn context(root: &Path) -> ToolCtx {
    ToolCtx {
        sandbox: Arc::new(Sandbox::new(root).unwrap()),
        executor: Arc::new(LocalExecutor),
        reads: Arc::new(Mutex::new(ReadTracker::default())),
        cancel: CancellationToken::new(),
        background: Arc::new(BackgroundTasks::default()),
        session: Arc::new(tokio::sync::Mutex::new(SessionState::default())),
        progress: None,
        agent_progress: None,
        call_progress: None,
    }
}

#[test]
fn dotenv_parser_returns_names_and_never_values() {
    let input = "PUBLIC_NAME=visible\nSECRET_TOKEN=do-not-leak\nexport AWS_PROFILE=prod\n";
    let keys = dotenv_keys(input);
    let encoded = serde_json::to_string(&keys).unwrap();
    assert!(encoded.contains("PUBLIC_NAME"));
    assert!(encoded.contains("SECRET_TOKEN"));
    assert!(encoded.contains("credential_candidate"));
    assert!(!encoded.contains("do-not-leak"));
    assert!(!encoded.contains("prod"));
}

#[test]
fn capability_fingerprint_excludes_random_census_id() {
    let mut report = CapabilityReport {
        id: "one".into(),
        schema_version: "v1".into(),
        platform: "linux".into(),
        architecture: "x86_64".into(),
        scope: ".".into(),
        executable_names: vec!["git".into()],
        environment: Vec::new(),
        dotenv_files: Vec::new(),
        credential_surfaces: Vec::new(),
        routing: BTreeMap::new(),
        fallbacks: Vec::new(),
        truncated: CensusTruncation {
            executables: false,
            environment_names: false,
            dotenv_files: false,
        },
        fingerprint: String::new(),
    };
    let first = safe_fingerprint(&report);
    report.id = "two".into();
    assert_eq!(safe_fingerprint(&report), first);
}

#[tokio::test]
async fn capability_tool_finds_gitignored_dotenv_but_never_returns_values() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(".gitignore"), ".env\nnested/*.env\n").unwrap();
    std::fs::write(
        temp.path().join(".env"),
        "PUBLIC_NAME=visible-value\nSECRET_TOKEN=do-not-leak\n",
    )
    .unwrap();
    std::fs::create_dir(temp.path().join("nested")).unwrap();
    std::fs::write(
        temp.path().join("nested/service.env"),
        "AWS_PROFILE=production-name\n",
    )
    .unwrap();
    let state = Arc::new(ScoutToolState {
        censuses: Mutex::new(HashMap::new()),
        ledgers: Mutex::new(HashMap::new()),
        max_parallel_agents: 3,
    });
    let outcome = ScoutCapabilitiesTool { state }
        .invoke(json!({"scope": "."}), &context(temp.path()))
        .await;
    assert!(!outcome.is_error, "{}", outcome.content);
    let serialized = serde_json::to_string(&outcome.details).unwrap();
    assert!(serialized.contains("PUBLIC_NAME"));
    assert!(serialized.contains("SECRET_TOKEN"));
    assert!(serialized.contains("AWS_PROFILE"));
    assert!(!serialized.contains("visible-value"));
    assert!(!serialized.contains("do-not-leak"));
    assert!(!serialized.contains("production-name"));
    assert_eq!(outcome.details["dotenv_files"].as_array().unwrap().len(), 2);
}
