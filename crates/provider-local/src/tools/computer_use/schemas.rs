use serde_json::{json, Map, Value};

pub fn click() -> Value {
    action_schema(
        "Provide exactly one click target: element_id, or both x and y.",
        vec![
            (
                "element_id",
                json!({"type": "string", "description": "Preferred element id from the latest Accessibility tree."}),
            ),
            (
                "x",
                json!({"type": "number", "description": "Fallback screenshot-local x pixel; pair with y."}),
            ),
            (
                "y",
                json!({"type": "number", "description": "Fallback screenshot-local y pixel; pair with x."}),
            ),
            (
                "button",
                json!({"type": "string", "enum": ["left", "right"], "description": "Mouse button; defaults to left."}),
            ),
        ],
        &[],
    )
}

pub fn type_text() -> Value {
    action_schema(
        "Prepare text entry into one observed accessible text control.",
        vec![
            (
                "element_id",
                json!({"type": "string", "description": "Text-control id from the latest Accessibility tree."}),
            ),
            (
                "text",
                json!({"type": "string", "description": "Literal text to enter; maximum 2,000 characters. Agent Desktop redacts it from durable records."}),
            ),
            (
                "replace",
                json!({"type": "boolean", "description": "Replace existing contents; defaults to false."}),
            ),
        ],
        &["element_id", "text"],
    )
}

pub fn keypress() -> Value {
    action_schema(
        "Prepare one bounded named key or single-character keypress.",
        vec![
            (
                "key",
                json!({"type": "string", "description": "One character or return, escape, tab, space, backspace, delete, arrow_up/down/left/right, home, end, page_up, or page_down."}),
            ),
            (
                "modifiers",
                json!({
                    "type": "array",
                    "items": {"type": "string", "enum": ["command", "control", "option", "shift"]},
                    "maxItems": 4,
                    "uniqueItems": true
                }),
            ),
        ],
        &["key"],
    )
}

pub fn scroll() -> Value {
    action_schema(
        "Prepare a bounded pixel scroll in the observed window.",
        vec![
            (
                "element_id",
                json!({"type": "string", "description": "Optional observed element whose center receives the scroll."}),
            ),
            (
                "delta_x",
                json!({"type": "integer", "minimum": -1200, "maximum": 1200}),
            ),
            (
                "delta_y",
                json!({"type": "integer", "minimum": -1200, "maximum": 1200}),
            ),
        ],
        &["delta_x", "delta_y"],
    )
}

pub fn drag() -> Value {
    action_schema(
        "Prepare a left-button drag. Each endpoint must use either an observed element id or screenshot-local x/y coordinates.",
        vec![
            ("start_element_id", json!({"type": "string"})),
            ("start_x", json!({"type": "number"})),
            ("start_y", json!({"type": "number"})),
            ("end_element_id", json!({"type": "string"})),
            ("end_x", json!({"type": "number"})),
            ("end_y", json!({"type": "number"})),
            (
                "duration_ms",
                json!({"type": "integer", "minimum": 50, "maximum": 2000}),
            ),
        ],
        &["duration_ms"],
    )
}

pub fn secondary_action() -> Value {
    action_schema(
        "Prepare one Accessibility action that the observed element explicitly advertised.",
        vec![
            ("element_id", json!({"type": "string"})),
            (
                "action",
                json!({
                    "type": "string",
                    "enum": ["AXPress", "AXShowMenu", "AXConfirm", "AXCancel", "AXIncrement", "AXDecrement"]
                }),
            ),
        ],
        &["element_id", "action"],
    )
}

pub fn select_text() -> Value {
    action_schema(
        "Prepare a bounded UTF-16-compatible selection range in an observed text control.",
        vec![
            ("element_id", json!({"type": "string"})),
            (
                "start",
                json!({"type": "integer", "minimum": 0, "maximum": 20000}),
            ),
            (
                "end",
                json!({"type": "integer", "minimum": 0, "maximum": 20000}),
            ),
        ],
        &["element_id", "start", "end"],
    )
}

pub fn set_value() -> Value {
    action_schema(
        "Prepare a numeric value change constrained by the observed slider or incrementor range.",
        vec![
            ("element_id", json!({"type": "string"})),
            ("value", json!({"type": "number"})),
        ],
        &["element_id", "value"],
    )
}

pub fn commit_action() -> Value {
    json!({
        "type": "object",
        "properties": {
            "prepared_action_id": {
                "type": "string",
                "description": "Opaque one-use id returned by a computer action preparation tool."
            }
        },
        "required": ["prepared_action_id"]
    })
}

fn action_schema(
    description: &str,
    action_properties: Vec<(&str, Value)>,
    action_required: &[&str],
) -> Value {
    let mut properties = Map::new();
    properties.insert(
        "risk".to_string(),
        json!({
            "type": "string",
            "enum": ["routine", "destructive", "financial", "external_communication", "credential", "security_sensitive", "ambiguous"],
            "description": "Advisory expected effect. The trusted backend independently classifies the observed target."
        }),
    );
    properties.insert(
        "reason".to_string(),
        json!({"type": "string", "description": "Specific reason for this action; maximum 500 characters."}),
    );
    properties.insert(
        "app_bundle_id".to_string(),
        json!({"type": "string", "description": "Exact bundle id from computer_get_state."}),
    );
    properties.insert(
        "pid".to_string(),
        json!({"type": "integer", "minimum": 1, "description": "Exact process id from computer_get_state."}),
    );
    properties.insert(
        "window_id".to_string(),
        json!({"type": "integer", "minimum": 1, "description": "Exact window id from computer_get_state."}),
    );
    properties.insert(
        "observation_id".to_string(),
        json!({"type": "string", "description": "One-use observation capability from computer_get_state."}),
    );
    for (name, schema) in action_properties {
        properties.insert(name.to_string(), schema);
    }
    properties.insert(
        "dry_run".to_string(),
        json!({"type": "boolean", "description": "Validate and produce a receipt without synthesizing input."}),
    );

    let mut required = vec![
        "risk",
        "reason",
        "app_bundle_id",
        "pid",
        "window_id",
        "observation_id",
    ];
    required.extend_from_slice(action_required);
    json!({
        "type": "object",
        "description": description,
        "properties": Value::Object(properties),
        "required": required
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_avoid_root_composition_and_keep_decision_before_payload() {
        for schema in [
            click(),
            type_text(),
            keypress(),
            scroll(),
            drag(),
            secondary_action(),
            select_text(),
            set_value(),
        ] {
            assert_eq!(schema.get("type").and_then(Value::as_str), Some("object"));
            assert!(schema.get("anyOf").is_none());
            let names = schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>();
            assert_eq!(
                &names[..6],
                [
                    "risk",
                    "reason",
                    "app_bundle_id",
                    "pid",
                    "window_id",
                    "observation_id"
                ]
            );
        }
    }
}
