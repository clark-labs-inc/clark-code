use serde_json::{json, Value};

use crate::orchestration::AcpHarnessConfig;

pub(super) fn delegate_schema(acp: &[AcpHarnessConfig]) -> Value {
    let mut harnesses = vec![Value::String("local".to_string())];
    harnesses.extend(acp.iter().map(|config| Value::String(config.id.clone())));
    json!({
        "type": "object",
        "properties": {
            "objective": {"type":"string","description":"The overall user-authorized objective."},
            "purpose": {"type":"string","enum":["explore","review","verify"]},
            "root_estimated_output_tokens": {"type":"integer","minimum":1},
            "risk": {"type":"object","properties": {
                "changed_paths":{"type":"integer","minimum":0},
                "touches_public_api":{"type":"boolean"},
                "touches_auth_or_security":{"type":"boolean"},
                "touches_data_migration":{"type":"boolean"},
                "touches_dependencies":{"type":"boolean"},
                "touches_concurrency":{"type":"boolean"},
                "verification_missing":{"type":"boolean"},
                "user_requested_review":{"type":"boolean"},
                "prior_attempt_failed":{"type":"boolean"}
            },"additionalProperties":false},
            "workstreams": {"type":"array","minItems":1,"maxItems":4,"items":{
                "type":"object","properties":{
                    "id":{"type":"string","pattern":"^[a-z0-9_-]{1,64}$"},
                    "objective":{"type":"string"},
                    "scopes":{"type":"array","minItems":1,"items":{"type":"string"},"uniqueItems":true},
                    "acceptance":{"type":"array","minItems":1,"items":{"type":"string"}},
                    "harness":{"type":"string","enum":harnesses},
                    "estimated_output_tokens":{"type":"integer","minimum":1}
                },"required":["id","objective","scopes","acceptance"],"additionalProperties":false
            }}
        },
        "required":["objective","purpose","workstreams"],
        "additionalProperties":false
    })
}

pub(super) fn resolve_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "orchestration_id":{"type":"string"},
            "decisions":{"type":"array","minItems":1,"items":{
                "type":"object","properties":{
                    "task_id":{"type":"string"},
                    "decision":{"type":"string","enum":["accept","rework"]},
                    "feedback":{"type":"string"}
                },"required":["task_id","decision"],"additionalProperties":false
            }}
        },
        "required":["orchestration_id","decisions"],
        "additionalProperties":false
    })
}
