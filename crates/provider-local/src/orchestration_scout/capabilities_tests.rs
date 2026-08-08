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
        model_override: None,
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
        adapter_executable_names: vec!["git".into()],
        path_executable_count: 2,
        path_executable_names_sha256: "b".repeat(64),
        environment: Vec::new(),
        environment_name_count: 0,
        environment_names_sha256: "c".repeat(64),
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
    report.path_executable_count = 99;
    report.path_executable_names_sha256 = "d".repeat(64);
    report.environment_name_count = 42;
    report.environment_names_sha256 = "e".repeat(64);
    assert_eq!(safe_fingerprint(&report), first);
}

#[test]
fn adapter_bootstrap_hides_unrelated_path_and_environment_names() {
    let executables = vec![
        "aws".to_string(),
        "custom-business-binary".to_string(),
        "git".to_string(),
    ];
    assert_eq!(
        adapter_executables(&executables),
        vec!["git".to_string(), "aws".to_string()]
    );
    assert!(adapter_environment_name("AWS_PROFILE"));
    assert!(adapter_environment_name("OTEL_EXPORTER_OTLP_ENDPOINT"));
    assert!(!adapter_environment_name("DESKTOP_SESSION"));
    assert!(!adapter_environment_name("RANDOM_APP_SETTING"));
}

#[test]
fn cloud_routes_report_candidates_without_claiming_authorization() {
    let environment = vec![NamedCapability {
        name: "CLOUDSDK_AUTH_CREDENTIAL_FILE_OVERRIDE".to_owned(),
        credential_candidate: true,
    }];
    let routes = routing_capabilities(
        &["gcloud".to_owned()],
        &environment,
        &["gcloud_config".to_owned()],
        &[],
    );
    assert_eq!(routes["gcloud"].state, "present");
    assert_eq!(routes["gcp_api"].state, "auth_candidate_unverified");
    assert!(rust_fallbacks()
        .iter()
        .any(|fallback| fallback.capability == "gcp_control_plane"));
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
        target: Mutex::new(None),
        adapter_gate: tokio::sync::Mutex::new(()),
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
    assert!(outcome.details["path_executable_count"].is_number());
    assert!(outcome.details["path_executable_names_sha256"].is_string());
    assert!(outcome.details["adapter_executable_names"].is_array());
}

#[tokio::test]
async fn capability_tool_preserves_all_dotenv_keys_beyond_the_old_cap() {
    let temp = tempfile::tempdir().unwrap();
    let body = (0..520)
        .map(|index| format!("KEY_{index}=secret-value-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(temp.path().join(".env"), body).unwrap();
    let state = Arc::new(ScoutToolState {
        censuses: Mutex::new(HashMap::new()),
        ledgers: Mutex::new(HashMap::new()),
        target: Mutex::new(None),
        adapter_gate: tokio::sync::Mutex::new(()),
        max_parallel_agents: 3,
    });

    let outcome = ScoutCapabilitiesTool { state }
        .invoke(json!({"scope": "."}), &context(temp.path()))
        .await;

    assert!(!outcome.is_error, "{}", outcome.content);
    let file = &outcome.details["dotenv_files"][0];
    assert_eq!(file["keys"].as_array().unwrap().len(), 520);
    assert_eq!(file["keys_truncated"], false);
    let serialized = serde_json::to_string(file).unwrap();
    assert!(serialized.contains("KEY_519"));
    assert!(!serialized.contains("secret-value"));
}
