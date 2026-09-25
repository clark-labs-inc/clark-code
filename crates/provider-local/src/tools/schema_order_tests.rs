use super::*;

#[test]
fn schema_property_order_survives_serialization() {
    // Tool schemas are autoregressive prompts: the advertised property order guides argument
    // generation (it does not enforce model output order), so authored order must reach the
    // wire. Without serde_json's `preserve_order` feature the json!{}
    // maps alphabetize (new_string before path) — this test pins the
    // feature and the intended orders.
    fn wire_order(registry: &ToolRegistry, tool: &str, props: &[&str]) {
        let schema = registry
            .schemas()
            .into_iter()
            .find(|s| s.function.name == tool)
            .unwrap_or_else(|| panic!("{tool} not registered"));
        let wire = serde_json::to_string(&schema).unwrap();
        let decoded: Value = serde_json::from_str(&wire).unwrap();
        let keys = decoded["function"]["parameters"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let positions = props
            .iter()
            .map(|property| {
                keys.iter()
                    .position(|key| key == property)
                    .unwrap_or_else(|| panic!("{tool}: {property} missing from properties"))
            })
            .collect::<Vec<_>>();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{tool}: properties out of order on the wire: {keys:?}"
        );
    }
    let reg = ToolRegistry::new(Some(memory::MemoryConfig::default()));
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
    wire_order(
        &reg,
        "research_tree",
        &["action", "tree_id", "expected_revision", "event"],
    );
    wire_order(&reg, "research_tree_status", &["tree_id", "cursor"]);
    wire_order(&reg, "tool_search", &["query"]);
    wire_order(&reg, "grep", &["pattern", "path"]);
    wire_order(&reg, "view_image", &["path"]);
    wire_order(&reg, "final_answer", &["files", "content"]);
    wire_order(
        &reg,
        "verify_effect",
        &["effect_id", "evidence", "expected", "observed", "status"],
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
            "expected_exit_code",
            "script",
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

    let mut image_registry = ToolRegistry::new(None);
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
