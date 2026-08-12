use serde_json::Value;

fn argument<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn snippet(value: &str) -> String {
    value
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(80)
        .collect()
}

fn activity_with_argument(args: &Value, key: &str, verb: &str, fallback: &str) -> String {
    argument(args, key)
        .map(|value| format!("{verb} {}", snippet(value)))
        .unwrap_or_else(|| fallback.to_string())
}

fn platform_activity(name: &str, prefix: &str, label: &str) -> Option<String> {
    let action = name.strip_prefix(prefix)?;
    let action = action.replace(['_', '-'], " ");
    Some(format!("{label} {action}"))
}

pub(super) fn tool_title(name: &str, args: &Value) -> String {
    match name {
        "web_fetch" => activity_with_argument(args, "url", "Read", "Reading a web page"),
        "read_file" => activity_with_argument(args, "path", "Read", "Reading a file"),
        "read_skill" => activity_with_argument(args, "skill", "Read skill", "Reading a skill"),
        "list_dir" => activity_with_argument(args, "path", "List", "Listing files"),
        "glob" => activity_with_argument(args, "pattern", "Find files matching", "Finding files"),
        "grep" => activity_with_argument(args, "pattern", "Search for", "Searching files"),
        "write_file" => activity_with_argument(args, "path", "Write", "Writing a file"),
        "edit_file" => activity_with_argument(args, "path", "Edit", "Editing a file"),
        "apply_patch" => "Applying changes".to_string(),
        "bash" => argument(args, "command")
            .map(snippet)
            .unwrap_or_else(|| "a shell command".to_string()),
        "bash_output" => "background command status".to_string(),
        "bash_wait" => "background command wait".to_string(),
        "bash_input" => "background command input".to_string(),
        "bash_kill" => "background command stop".to_string(),
        "check_diagnostics" => "project diagnostics".to_string(),
        "browser" => match argument(args, "action") {
            Some("navigate") => {
                activity_with_argument(args, "url", "browser navigation to", "browser navigation")
            }
            Some("click") => "browser click".to_string(),
            Some("extract_text") => "page text extraction".to_string(),
            Some("screenshot") => "browser screenshot".to_string(),
            _ => "browser action".to_string(),
        },
        "organization_knowledge" => "Searched organization knowledge".to_string(),
        "propose_plan" => "Proposed a plan".to_string(),
        "enter_plan_mode" => "Entered plan mode".to_string(),
        "update_plan" => "Updated the plan".to_string(),
        "create_goal" => "Started a goal".to_string(),
        "update_goal" => "Updated the goal".to_string(),
        "get_goal" => "Checked the goal".to_string(),
        "verify_effect" => "Verifying an external result".to_string(),
        "memory" => match argument(args, "action") {
            Some("recall") => "Recalling memory".to_string(),
            Some("remember") => "Saving memory".to_string(),
            Some("forget") => "Forgetting memory".to_string(),
            _ => "Working with memory".to_string(),
        },
        "delegate_read_only" => "Delegating an investigation".to_string(),
        "resolve_delegation" => "Reviewing delegated results".to_string(),
        "delegate_coding_workstreams" => "parallel coding workstreams".to_string(),
        "resolve_coding_workstreams" => "coding workstream resolution".to_string(),
        "scout_capabilities" => "Surveying system capabilities".to_string(),
        "scout_repository_census" => "Reconciling local repositories".to_string(),
        "scout_adapter" => "Reading a business control plane".to_string(),
        "scout_enterprise" => "Updating the enterprise system map".to_string(),
        "scout_enterprise_query" => "Reading the enterprise system map".to_string(),
        "view_image" => argument(args, "path")
            .map(|path| format!("View image: {}", snippet(path)))
            .unwrap_or_else(|| "View image".to_string()),
        "generate_image" => argument(args, "output_path")
            .map(|path| format!("Generate image: {}", snippet(path)))
            .unwrap_or_else(|| "Generate image".to_string()),
        _ if name.starts_with("mcp_") => "Using a connected service".to_string(),
        _ => platform_activity(name, "android_", "Android")
            .or_else(|| platform_activity(name, "ios_", "iOS"))
            .unwrap_or_else(|| "Working".to_string()),
    }
}
