use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementChange {
    pub id: String,
    pub fields: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityDiff {
    pub base_observation_id: String,
    pub added_ids: Vec<String>,
    pub removed_ids: Vec<String>,
    pub changed: Vec<ElementChange>,
    pub focus_changed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationSettlement {
    pub stable: bool,
    pub elapsed_ms: u64,
    pub samples: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValueConstraints {
    pub minimum: f64,
    pub maximum: f64,
    pub step: Option<f64>,
}

pub(crate) fn diff_elements(
    base_observation_id: String,
    before: &[crate::ElementInfo],
    after: &[crate::ElementInfo],
) -> AccessibilityDiff {
    use std::collections::HashMap;

    let before_by_id = before
        .iter()
        .map(|element| (element.id.as_str(), element))
        .collect::<HashMap<_, _>>();
    let after_by_id = after
        .iter()
        .map(|element| (element.id.as_str(), element))
        .collect::<HashMap<_, _>>();
    let mut added_ids = after_by_id
        .keys()
        .filter(|id| !before_by_id.contains_key(**id))
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let mut removed_ids = before_by_id
        .keys()
        .filter(|id| !after_by_id.contains_key(**id))
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let mut changed = Vec::new();
    for (id, current) in &after_by_id {
        let Some(previous) = before_by_id.get(id) else {
            continue;
        };
        let mut fields = Vec::new();
        if previous.role != current.role {
            fields.push("role".to_string());
        }
        if previous.name != current.name {
            fields.push("name".to_string());
        }
        if previous.value != current.value {
            fields.push("value".to_string());
        }
        if previous.description != current.description {
            fields.push("description".to_string());
        }
        if previous.bounds != current.bounds {
            fields.push("bounds".to_string());
        }
        if previous.enabled != current.enabled {
            fields.push("enabled".to_string());
        }
        if previous.focused != current.focused {
            fields.push("focused".to_string());
        }
        if previous.actionable != current.actionable {
            fields.push("actionable".to_string());
        }
        if previous.actions != current.actions {
            fields.push("actions".to_string());
        }
        if previous.value_settable != current.value_settable
            || previous.value_constraints != current.value_constraints
        {
            fields.push("value_constraints".to_string());
        }
        if !fields.is_empty() {
            changed.push(ElementChange {
                id: (*id).to_string(),
                fields,
            });
        }
    }
    added_ids.sort();
    removed_ids.sort();
    changed.sort_by(|left, right| left.id.cmp(&right.id));
    let focused_before = before
        .iter()
        .find(|element| element.focused)
        .map(|element| element.id.as_str());
    let focused_after = after
        .iter()
        .find(|element| element.focused)
        .map(|element| element.id.as_str());
    AccessibilityDiff {
        base_observation_id,
        added_ids,
        removed_ids,
        changed,
        focus_changed: focused_before != focused_after,
    }
}

#[cfg(any(all(feature = "helper-service", target_os = "macos"), test))]
pub(crate) fn settlement_fingerprint(elements: &[crate::ElementInfo]) -> String {
    let mut output = String::new();
    for element in elements {
        use std::fmt::Write;
        let _ = write!(
            output,
            "{}\u{1f}{}\u{1f}{:?}\u{1f}{:?}\u{1f}{:.1},{:.1},{:.1},{:.1}\u{1f}{}\u{1f}{};",
            element.id,
            element.role,
            element.name,
            element.value,
            element.bounds.x,
            element.bounds.y,
            element.bounds.width,
            element.bounds.height,
            element.enabled,
            element.focused,
        );
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(id: &str, value: &str, focused: bool) -> crate::ElementInfo {
        crate::ElementInfo {
            id: id.to_string(),
            role: "AXStaticText".to_string(),
            name: Some(id.to_string()),
            value: Some(value.to_string()),
            description: None,
            bounds: crate::Rect::default(),
            enabled: true,
            focused,
            actionable: false,
            actions: Vec::new(),
            sensitive_text: false,
            value_settable: false,
            value_constraints: None,
        }
    }

    #[test]
    fn accessibility_diff_is_deterministic_and_field_specific() {
        let before = vec![element("ax-1", "old", true), element("ax-2", "gone", false)];
        let after = vec![
            element("ax-1", "new", false),
            element("ax-3", "added", true),
        ];
        let diff = diff_elements("obs-1".to_string(), &before, &after);
        assert_eq!(diff.added_ids, ["ax-3"]);
        assert_eq!(diff.removed_ids, ["ax-2"]);
        assert_eq!(diff.changed[0].id, "ax-1");
        assert_eq!(diff.changed[0].fields, ["value", "focused"]);
        assert!(diff.focus_changed);
        assert_ne!(
            settlement_fingerprint(&before),
            settlement_fingerprint(&after)
        );
    }
}
