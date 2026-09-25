use serde_json::{json, Value};

pub(super) fn mutation_schema() -> Value {
    let text = json!({"type":"string","minLength":1,"maxLength":2048});
    let refs = json!({"type":"array","maxItems":16,"items":{"type":"string","minLength":1,"maxLength":512}});
    let score = json!({"type":"number","minimum":0,"maximum":1});
    json!({
        "type":"object",
        "properties":{
            "action":{"type":"string","enum":["begin","apply","select"]},
            "tree_id":{"type":"string","minLength":1,"maxLength":80,"description":"Stable local artifact name, ASCII letters/digits/hyphens/underscores."},
            "expected_revision":{"type":"integer","minimum":1,"description":"Required for apply/select. Use exact revision returned by status or last update; stale writes fail."},
            "objective":{"type":"string","minLength":1,"maxLength":2048,"description":"Required for begin: the user question, not a finding."},
            "scope":{"type":"string","description":"Required for begin. Describe the user-authorized question/resources. A scope declaration grants no execution permissions.","minLength":1,"maxLength":2048},
            "sources":{"type":"array","maxItems":32,"items":{"type":"string"},"description":"Required for begin, may be empty for non-file questions. Relative source files to hash (total 16 MiB). Changed/missing sources block updates; start a new tree. These files are not a complete repository snapshot."},
            "max_nodes":{"type":"integer","minimum":1,"maximum":256,"description":"Required for begin. Maximum unique branch nodes; start with 32."},
            "max_steps":{"type":"integer","minimum":1,"maximum":1000,"description":"Required for begin. Node selections, including retries; start with 64. Not a tool-call, time, or token budget."},
            "event":{
                "type":"object",
                "description":"Required for apply. proposed: all proposal fields. recorded: key,outcome,evidence_refs,rationale. reopened: key,evidence_refs,rationale (new evidence required). reassessed: key,evidence_refs,rationale,assessment (pending only; new evidence, unchanged scope). pruned: key,rationale. Omit fields irrelevant to the event type.",
                "properties":{
                    "type":{"type":"string","enum":["proposed","recorded","reopened","reassessed","pruned"]},
                    "key":{"type":"string","description":"Stable semantic branch identity, reused for duplicate proposals."},
                    "parent":{"type":["string","null"],"description":"Primary discovery parent, null for root work."},
                    "links":{"type":"array","items":{"type":"string"},"description":"Additional existing related nodes; preserved when discoveries converge."},
                    "dependencies":{"type":"array","items":{"type":"string"},"description":"Existing nodes which must complete or be refuted before selection."},
                    "evidence_refs":refs,
                    "rationale":text,
                    "assessment":{
                        "type":"object",
                        "properties":{
                            "surface":{"type":"string","enum":["target","endpoint","finding","hypothesis","evidence"]},
                            "in_scope":{"type":"boolean"},
                            "requires_validation":{"type":"boolean"},
                            "priority":score.clone(),"novelty":score.clone(),"confidence":score.clone(),"impact":score
                        },
                        "required":["surface","in_scope","requires_validation","priority","novelty","confidence","impact"],
                        "additionalProperties":false
                    },
                    "outcome":{"type":"string","enum":["supports","refutes","expands","inconclusive","blocked"]}
                },
                "required":["type","key","rationale"],"additionalProperties":false
            }
        },
        "required":["action","tree_id"],"additionalProperties":false
    })
}

pub(super) fn status_schema() -> Value {
    json!({
        "type":"object","properties":{
            "tree_id":{"type":"string","minLength":1,"maxLength":80},
            "cursor":{"type":"integer","minimum":0,"description":"Node offset, default 0. Returns at most 32 nodes plus up to 16 frontier hints."}
        },"required":["tree_id"],"additionalProperties":false
    })
}
