use super::*;
use crate::loop_state::SessionState;

struct ExtensionTool;

#[async_trait::async_trait]
impl ToolExecutor for ExtensionTool {
    fn name(&self) -> &str {
        "example_extension"
    }

    fn description(&self) -> &str {
        "Example product extension."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    async fn invoke(&self, _args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        ToolOutcome::ok("ok")
    }
}

struct ExtensionPack;

impl ToolPack for ExtensionPack {
    fn id(&self) -> &str {
        "example"
    }

    fn install(&self, registry: &mut ToolRegistry) -> Result<(), String> {
        registry.register_extension_tool(ToolExposure::Deferred, Arc::new(ExtensionTool))
    }
}

#[test]
fn product_tool_packs_extend_without_shadowing_core_tools() {
    let mut registry = ToolRegistry::new(None, None);
    registry.install_tool_pack(&ExtensionPack).unwrap();
    assert!(registry.get("example_extension").is_some());

    let duplicate = registry.install_tool_pack(&ExtensionPack).unwrap_err();
    assert!(duplicate.contains("already registered"));
    assert!(registry.get("read_file").is_some());
}

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
fn empty_registry_exposes_and_executes_no_tools() {
    let registry = ToolRegistry::empty();
    assert!(registry.schemas().is_empty());
    assert!(registry.executors().next().is_none());
    assert!(registry.tool_names().is_empty());
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
    // Commit to disclosure depth before selecting the memory boundary.
    wire_order(&reg, "memory_recall", &["action", "scope"]);
    // Rationale first: explanation tokens condition the plan steps.
    wire_order(&reg, "update_plan", &["explanation", "plan"]);
    // Commit cross-step invariants before obligations, then render prose last.
    // This prevents a polished Markdown answer from anchoring generation
    // before the model has decided the execution contract.
    wire_order(
        &reg,
        "propose_plan",
        &["global_reminders", "execution_contract", "plan"],
    );
    let propose = reg
        .schemas()
        .into_iter()
        .find(|schema| schema.function.name == "propose_plan")
        .unwrap();
    let execution_step_keys = propose.function.parameters["properties"]["execution_contract"]
        ["items"]["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        execution_step_keys,
        ["title", "files", "done_when", "reminders"],
        "propose_plan execution step keys must preserve autoregressive order"
    );
    let update_plan = reg
        .schemas()
        .into_iter()
        .find(|schema| schema.function.name == "update_plan")
        .unwrap();
    let checklist_step_keys = update_plan.function.parameters["properties"]["plan"]["items"]
        ["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        checklist_step_keys,
        ["plan_step_id", "step", "status"],
        "update_plan must locate the approved obligation before emitting mutable state"
    );
    wire_order(&reg, "tool_search", &["query"]);
    wire_order(&reg, "grep", &["pattern", "path"]);
    wire_order(&reg, "view_image", &["path"]);
    wire_order(&reg, "final_answer", &["content"]);
    wire_order(
        &reg,
        "verify_effect",
        &["effect_id", "status", "evidence", "expected", "observed"],
    );
    wire_order(&reg, "document_convert", &["path", "to", "output_path"]);
    wire_order(
        &reg,
        "security_poc_execute",
        &[
            "scan_id",
            "candidate_id",
            "inventory_id",
            "scope",
            "control",
            "language",
            "expected_observation",
            "script",
            "expected_exit_code",
            "timeout_seconds",
        ],
    );
    wire_order(
        &reg,
        "security_scan_contract",
        &[
            "action",
            "scope",
            "diff_kind",
            "base",
            "head",
            "scan_id",
            "deep_run_id",
            "orchestration_id",
            "candidate_ids",
            "cursor",
            "page_size",
            "path",
        ],
    );

    let mut image_registry = ToolRegistry::new(None, None);
    image_registry.enable_image_generation(image::ImageGenerationConfig {
        base_url: "https://product.example/v1".into(),
        api_key: "ck_live_test".into(),
    });
    wire_order(
        &image_registry,
        "generate_image",
        &["prompt", "input_images", "output_path"],
    );
}

#[test]
fn registry_lists_neutral_local_tools_without_product_extensions() {
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
    assert!(!names.contains(&"product_research".to_string()));
    assert!(!names.contains(&"memory".to_string()));
    assert!(local.get("read_file").is_some());
    assert!(local.get("nope").is_none());

    let product_config_does_not_implicitly_install_tools = ToolRegistry::new(
        Some(AuxiliaryModelConfig {
            base_url: "https://product.example/v1".into(),
            api_key: Some("ck_live_x".into()),
            model: "local-model".into(),
        }),
        None,
    );
    let names: Vec<_> = product_config_does_not_implicitly_install_tools
        .schemas()
        .iter()
        .map(|s| s.function.name.clone())
        .collect();
    assert!(!names.contains(&"product_research".to_string()));
}

#[tokio::test]
async fn copy_on_write_registry_isolates_prior_run_and_rebinds_tool_search() {
    let mut next = Arc::new(ToolRegistry::new(None, None));
    let prior_run = next.clone();

    Arc::make_mut(&mut next).enable_browser(crate::browser_binary::test_browser_config());

    assert!(!Arc::ptr_eq(&next, &prior_run));
    assert!(next.get("browser").is_some());
    assert!(prior_run.get("browser").is_none());

    let directory = tempfile::tempdir().unwrap();
    let session = Arc::new(tokio::sync::Mutex::new(SessionState::default()));
    let search = next.get("tool_search").unwrap();
    let outcome = search
        .invoke(
            serde_json::json!({"query": "browser"}),
            &ToolCtx {
                sandbox: Arc::new(Sandbox::new(directory.path()).unwrap()),
                executor: Arc::new(crate::exec::LocalExecutor),
                reads: Arc::new(Mutex::new(ReadTracker::default())),
                cancel: CancellationToken::new(),
                background: Arc::new(crate::background::BackgroundTasks::default()),
                session: session.clone(),
                progress: None,
                agent_progress: None,
                call_progress: None,
                model_override: None,
            },
        )
        .await;

    assert!(!outcome.is_error, "{}", outcome.content);
    assert!(session.lock().await.deferred_tools.contains("browser"));
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
        .next_turn_tool_allowlist(agent_loop::plugin::ToolGateContext {
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
        "security_poc_execute",
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
        .next_turn_tool_allowlist(agent_loop::plugin::ToolGateContext {
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
    assert!(off.get("memory_recall").is_none());
    let on = ToolRegistry::new(None, Some(memory::MemoryConfig::default()));
    assert!(on.get("memory").is_some());
    let recall = on.get("memory_recall").unwrap();
    assert!(!recall.mutating());
    assert_eq!(recall.kind(), ToolKind::Search);
    // Memory writes are curated + path-constrained, so they don't gate.
    assert!(!on.get("memory").unwrap().mutating());
}

#[test]
fn organization_knowledge_is_an_explicit_read_only_registry_plugin() {
    struct EmptyContext;
    #[async_trait::async_trait]
    impl crate::platform::PlatformContextProvider for EmptyContext {
        async fn personal_memories(&self) -> Result<Vec<crate::platform::PersonalMemory>, String> {
            Ok(Vec::new())
        }
        async fn repository_context(
            &self,
            _fingerprint: &str,
            _query: &str,
        ) -> Result<crate::platform::RepositoryContext, String> {
            Err("not configured".into())
        }
        async fn organization_knowledge(
            &self,
            query: &str,
            _organization_id: Option<&str>,
            _limit: i64,
        ) -> Result<crate::platform::OrganizationKnowledgeResponse, String> {
            Ok(crate::platform::OrganizationKnowledgeResponse {
                query: query.into(),
                organizations: Vec::new(),
            })
        }

        async fn feature_context(
            &self,
            request: &crate::platform::FeatureContextRequest,
        ) -> Result<crate::platform::FeatureContextResponse, String> {
            Ok(crate::platform::FeatureContextResponse {
                query: request.query.clone(),
                packets: Vec::new(),
                unavailable_reason: Some("not configured".into()),
            })
        }

        async fn submit_feature_context_feedback(
            &self,
            _request: &crate::platform::FeatureContextFeedbackRequest,
        ) -> Result<crate::platform::FeatureContextFeedbackReceipt, String> {
            Err("not configured".into())
        }
    }
    let mut registry = ToolRegistry::new(None, None);
    assert!(registry.get("organization_knowledge").is_none());
    registry.enable_organization_knowledge(Arc::new(EmptyContext));
    let tool = registry.get("organization_knowledge").unwrap();
    assert!(!tool.mutating());
    assert_eq!(tool.kind(), ToolKind::Research);

    registry.enable_feature_context(
        Arc::new(EmptyContext),
        crate::tools::feature_context::FeatureContextBinding {
            repository_fingerprint: Some("repo-fingerprint".into()),
            organization_id: Some("host-org".into()),
            workspace_id: Some("host-workspace".into()),
        },
    );
    let tool = registry.get("enterprise_context").unwrap();
    assert!(!tool.mutating());
    assert_eq!(tool.kind(), ToolKind::Research);
    let schema = serde_json::to_string(&tool.parameters()).unwrap();
    assert!(schema.find("\"action\"").unwrap() < schema.find("\"query\"").unwrap());
    assert!(!schema.contains("organization_id"));
    assert!(!schema.contains("workspace_id"));
    assert!(!schema.contains("repository_fingerprint"));

    let feedback = registry.get("enterprise_context_feedback").unwrap();
    assert!(feedback.mutating());
    assert_eq!(feedback.permission_class(), ToolPermissionClass::External);
    let scope = feedback.permission_scope(&serde_json::json!({})).unwrap();
    assert_eq!(scope.risk.as_deref(), Some("confirm"));
    assert!(!scope.remember);
    assert!(!scope.preapproved);
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
        base_url: "https://product.example/v1".into(),
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
