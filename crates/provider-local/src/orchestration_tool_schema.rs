use serde_json::{json, Value};

pub(super) fn delegate_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "objective": {"type":"string","description":"The overall user-authorized objective."},
            "purpose": {"type":"string","enum":["explore","review","verify"]},
            "workstreams": {"type":"array","minItems":1,"maxItems":4,"items":{
                "type":"object","properties":{
                    "id":{"type":"string","pattern":"^[a-z0-9_-]{1,64}$"},
                    "objective":{"type":"string"},
                    "scopes":{"type":"array","minItems":1,"items":{"type":"string"},"uniqueItems":true},
                    "acceptance":{"type":"array","minItems":1,"items":{"type":"string"}}
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
