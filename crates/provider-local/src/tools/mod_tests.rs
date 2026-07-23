use super::*;
use crate::loop_state::SessionState;

#[test]
fn read_tracker_distinguishes_fresh_notread_and_stale() {
    use std::time::{Duration, SystemTime};
    let mut t = ReadTracker::default();
    let path = Path::new("/proj/a.rs");
    let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    // Never recorded → NotRead.
    assert_eq!(t.check(path, t0), ReadCheck::NotRead);
    // Recorded at t0, unchanged → Fresh.
    t.record(path, t0);
    assert_eq!(t.check(path, t0), ReadCheck::Fresh);
    // File now newer than the recorded read → Stale.
    let t1 = t0 + Duration::from_secs(5);
    assert_eq!(t.check(path, t1), ReadCheck::Stale);
}

#[test]
fn permission_mode_parses_synonyms() {
    assert_eq!(PermissionMode::parse("allow"), Some(PermissionMode::Allow));
    assert_eq!(PermissionMode::parse("ASK"), Some(PermissionMode::Ask));
    assert_eq!(PermissionMode::parse(" deny "), Some(PermissionMode::Deny));
    assert_eq!(PermissionMode::parse("maybe"), None);
}

#[test]
fn schema_property_order_survives_serialization() {
    // Tool schemas are autoregressive prompts: the model emits arguments
    // in the property order it sees, so authored order must reach the
    // wire. Without serde_json's `preserve_order` feature the json!{}
    // maps alphabetize (new_string before path) — this test pins the
    // feature and the intended orders.
    fn wire_order(registry: &ToolRegistry, tool: &str, props: &[&str]) {
        let schema = registry
            .schemas()
            .into_iter()
            .find(|s| s.function.name == tool)
            .unwrap_or_else(|| panic!("{tool} not registered"));
        let wire = serde_json::to_string(&schema.function.parameters).unwrap();
        let positions: Vec<usize> = props
            .iter()
            .map(|p| {
                wire.find(&format!("\"{p}\""))
                    .unwrap_or_else(|| panic!("{tool}: {p} missing from schema"))
            })
            .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "{tool}: properties out of order on the wire: {wire}"
        );
    }
    let reg = ToolRegistry::new(None, Some(memory::MemoryConfig::default()));
    let model_visible_schemas = serde_json::to_string(&reg.schemas())
        .unwrap()
        .to_ascii_lowercase();
    assert!(!model_visible_schemas.contains("codex"));
    // Locate before payload: the model must commit to where/what it is
    // replacing before it generates the replacement.
    wire_order(&reg, "edit_file", &["path", "old_string", "new_string"]);
    wire_order(&reg, "write_file", &["path", "content"]);
    wire_order(&reg, "read_file", &["path", "offset", "limit"]);
    // Commit to the command and location before deciding whether it needs
    // a user-reviewed sandbox exception; execution tuning comes last.
    wire_order(
        &reg,
        "bash",
        &[
            "command",
            "workdir",
            "sandbox_permissions",
            "justification",
            "effect",
            "effect_target",
            "run_in_background",
            "timeout_ms",
        ],
    );
    wire_order(
        &reg,
        "bash_wait",
        &[
            "task_id",
            "output_contains",
            "timeout_ms",
            "poll_interval_ms",
        ],
    );
    wire_order(&reg, "bash_input", &["task_id", "text", "close"]);
    // Decide the action, scope, and provenance before the fact being saved.
    wire_order(
        &reg,
        "memory",
        &["action", "scope", "source", "title", "content"],
    );
    // Rationale first: explanation tokens condition the plan steps.
    wire_order(&reg, "update_plan", &["explanation", "plan"]);
    wire_order(&reg, "tool_search", &["query", "limit"]);
    wire_order(&reg, "grep", &["pattern", "path"]);
    wire_order(&reg, "view_image", &["path"]);
    wire_order(
        &reg,
        "verify_effect",
        &["effect_id", "status", "evidence", "expected", "observed"],
    );
    wire_order(&reg, "document_convert", &["path", "to", "output_path"]);

    let mut image_registry = ToolRegistry::new(None, None);
    image_registry.enable_image_generation(image::ImageGenerationConfig {
        base_url: "https://api.clarkslabs.com/v1".into(),
        api_key: "ck_live_test".into(),
    });
    wire_order(
        &image_registry,
        "generate_image",
        &["prompt", "input_images", "output_path"],
    );
}

#[test]
fn registry_lists_local_tools_and_optionally_clark() {
    let local = ToolRegistry::new(None, None);
    let names: Vec<_> = local
        .schemas()
        .iter()
        .map(|s| s.function.name.clone())
        .collect();
    assert!(names.contains(&"read_file".to_string()));
    assert!(names.contains(&"edit_file".to_string()));
    assert!(names.contains(&"bash".to_string()));
    assert!(names.contains(&"view_image".to_string()));
    assert!(!names.contains(&"generate_image".to_string()));
    assert!(!names.contains(&"clark_research".to_string()));
    assert!(!names.contains(&"memory".to_string()));
    assert!(local.get("read_file").is_some());
    assert!(local.get("nope").is_none());

    let with_clark = ToolRegistry::new(
        Some(AgenticClarkConfig {
            base_url: "https://api.clarkslabs.com/v1".into(),
            api_key: Some("ck_live_x".into()),
            model: "clark".into(),
        }),
        None,
    );
    let names: Vec<_> = with_clark
        .schemas()
        .iter()
        .map(|s| s.function.name.clone())
        .collect();
    assert!(names.contains(&"clark_research".to_string()));
}

#[tokio::test]
async fn runtime_catalog_keeps_core_eager_and_defers_specialized_tools() {
    let registry = ToolRegistry::new(None, None);
    let session = Arc::new(tokio::sync::Mutex::new(SessionState::default()));
    let gate = registry.deferred_tool_gate(session.clone());
    let available = registry
        .executors()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    let available_refs = available.iter().map(String::as_str).collect::<Vec<_>>();
    let initial = gate
        .next_turn_tool_allowlist(clark_agent::plugin::ToolGateContext {
            iteration: 0,
            messages: &[],
            conversation_id: Some("session"),
            available_tool_names: &available_refs,
        })
        .await
        .unwrap();

    for name in [
        "read_file",
        "grep",
        "edit_file",
        "bash",
        "propose_plan",
        "enter_plan_mode",
        "update_plan",
        "tool_search",
    ] {
        assert!(initial.contains(name), "{name} should be eager");
    }
    for name in [
        "create_goal",
        "document_convert",
        "web_fetch",
        "android_list_devices",
        "android_tap",
    ] {
        assert!(!initial.contains(name), "{name} should be deferred");
    }
    #[cfg(target_os = "macos")]
    assert!(!initial.contains("ios_list_simulators"));

    session
        .lock()
        .await
        .deferred_tools
        .insert("android_tap".into());
    let activated = gate
        .next_turn_tool_allowlist(clark_agent::plugin::ToolGateContext {
            iteration: 1,
            messages: &[],
            conversation_id: Some("session"),
            available_tool_names: &available_refs,
        })
        .await
        .unwrap();
    assert!(activated.contains("android_tap"));
    assert!(!activated.contains("android_swipe"));
}

#[test]
fn memory_tool_registered_only_when_enabled() {
    let off = ToolRegistry::new(None, None);
    assert!(off.get("memory").is_none());
    let on = ToolRegistry::new(None, Some(memory::MemoryConfig::default()));
    assert!(on.get("memory").is_some());
    // Memory writes are curated + path-constrained, so they don't gate.
    assert!(!on.get("memory").unwrap().mutating());
}

#[test]
fn organization_knowledge_is_an_explicit_read_only_registry_plugin() {
    let mut registry = ToolRegistry::new(None, None);
    assert!(registry.get("organization_knowledge").is_none());
    registry.enable_organization_knowledge(organization_knowledge::OrganizationKnowledgeConfig {
        base_url: "https://api.clarkslabs.com/v1".into(),
        api_key: "ck_live_test".into(),
    });
    let tool = registry.get("organization_knowledge").unwrap();
    assert!(!tool.mutating());
    assert_eq!(tool.kind(), ToolKind::Research);
}

#[test]
fn mutating_tools_are_flagged() {
    let reg = ToolRegistry::new(None, None);
    assert!(reg.get("write_file").unwrap().mutating());
    assert!(reg.get("edit_file").unwrap().mutating());
    assert!(reg.get("bash").unwrap().mutating());
    assert!(!reg.get("read_file").unwrap().mutating());
    assert!(!reg.get("grep").unwrap().mutating());
    assert!(!reg.get("view_image").unwrap().mutating());

    let mut signed_in = ToolRegistry::new(None, None);
    signed_in.enable_image_generation(image::ImageGenerationConfig {
        base_url: "https://api.clarkslabs.com/v1".into(),
        api_key: "ck_live_test".into(),
    });
    assert!(signed_in.get("generate_image").unwrap().mutating());
}

#[test]
fn plan_tools_are_registered_with_correct_mutating_flags() {
    let reg = ToolRegistry::new(None, None);
    assert!(!reg.get("propose_plan").unwrap().mutating());
    assert!(reg.get("enter_plan_mode").unwrap().mutating());
    assert!(!reg.get("update_plan").unwrap().mutating());
}

#[test]
fn web_fetch_is_registered_non_mutating_but_requires_external_consent() {
    let reg = ToolRegistry::new(None, None);
    let t = reg.get("web_fetch").unwrap();
    assert!(!t.mutating());
    assert_eq!(t.permission_class(), ToolPermissionClass::External);
}

#[test]
fn android_tools_are_registered_with_correct_mutating_flags() {
    let reg = ToolRegistry::new(None, None);
    // Read-only: never gate the user.
    assert!(!reg.get("android_list_devices").unwrap().mutating());
    assert!(!reg.get("android_screenshot").unwrap().mutating());
    // Mutating: one "always allow" confirm each.
    for name in [
        "android_boot_emulator",
        "android_shutdown_emulator",
        "android_install_app",
        "android_uninstall_app",
        "android_launch_app",
        "android_tap",
        "android_swipe",
        "android_type_text",
        "android_press_button",
    ] {
        assert!(
            reg.get(name).unwrap().mutating(),
            "{name} should be mutating"
        );
    }
}

#[test]
fn remote_registry_does_not_expose_desktop_mobile_tools() {
    let mut reg = ToolRegistry::new(None, None);
    reg.disable_desktop_mobile_tools();
    let names: Vec<_> = reg
        .schemas()
        .into_iter()
        .map(|tool| tool.function.name)
        .collect();
    assert!(names.iter().all(|name| !name.starts_with("android_")));
    assert!(names.iter().all(|name| !name.starts_with("ios_")));
    assert!(names.iter().any(|name| name == "bash"));
}

#[test]
#[cfg(target_os = "macos")]
fn ios_tools_are_registered_with_correct_mutating_flags() {
    let reg = ToolRegistry::new(None, None);
    // Read-only: never gate the user.
    assert!(!reg.get("ios_list_simulators").unwrap().mutating());
    assert!(!reg.get("ios_screenshot").unwrap().mutating());
    // Mutating: one "always allow" confirm each.
    for name in [
        "ios_boot_simulator",
        "ios_shutdown_simulator",
        "ios_install_app",
        "ios_uninstall_app",
        "ios_launch_app",
        "ios_tap",
        "ios_swipe",
        "ios_type_text",
        "ios_press_button",
    ] {
        assert!(
            reg.get(name).unwrap().mutating(),
            "{name} should be mutating"
        );
    }
}

#[test]
#[cfg(not(target_os = "macos"))]
fn ios_tools_are_absent_on_non_macos() {
    let reg = ToolRegistry::new(None, None);
    assert!(reg.get("ios_list_simulators").is_none());
}
