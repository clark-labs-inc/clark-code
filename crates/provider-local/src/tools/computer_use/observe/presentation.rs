const DIFF_ITEM_CAP: usize = 20;

pub(super) fn format_element(element: &computer_use::ElementInfo) -> String {
    let mut fields = vec![
        format!("- {} {}", element.id, element.role),
        format!(
            "bounds=({:.0},{:.0} {:.0}x{:.0})",
            element.bounds.x, element.bounds.y, element.bounds.width, element.bounds.height
        ),
        format!("enabled={}", element.enabled),
    ];
    if let Some(name) = element.name.as_deref() {
        fields.push(format!("name={name:?}"));
    }
    if let Some(value) = element.value.as_deref() {
        fields.push(format!("value={value:?}"));
    }
    if let Some(description) = element.description.as_deref() {
        fields.push(format!("description={description:?}"));
    }
    if element.focused {
        fields.push("focused=true".to_string());
    }
    if element.actionable {
        fields.push("actionable=true".to_string());
    }
    if element.sensitive_text {
        fields.push("sensitive_text=true".to_string());
    }
    if !element.actions.is_empty() {
        fields.push(format!("actions={:?}", element.actions));
    }
    fields.join(" ")
}

pub(super) fn format_settlement(settlement: &computer_use::ObservationSettlement) -> String {
    format!(
        "Accessibility settling: stable={} samples={} elapsed_ms={}",
        settlement.stable, settlement.samples, settlement.elapsed_ms
    )
}

pub(super) fn format_diff(diff: Option<&computer_use::AccessibilityDiff>) -> String {
    let Some(diff) = diff else {
        return "Accessibility diff: baseline established".to_string();
    };
    let added = bounded_items(diff.added_ids.iter().map(String::as_str));
    let removed = bounded_items(diff.removed_ids.iter().map(String::as_str));
    let changed = bounded_items(
        diff.changed
            .iter()
            .map(|change| format!("{}:{}", change.id, change.fields.join(","))),
    );
    format!(
        "Accessibility diff from {}: added=[{}] removed=[{}] changed=[{}] focus_changed={}",
        diff.base_observation_id, added, removed, changed, diff.focus_changed
    )
}

fn bounded_items<I, T>(items: I) -> String
where
    I: IntoIterator<Item = T>,
    T: ToString,
{
    let mut items = items.into_iter();
    let visible = items
        .by_ref()
        .take(DIFF_ITEM_CAP)
        .map(|item| item.to_string())
        .collect::<Vec<_>>();
    if items.next().is_some() {
        format!("{},…", visible.join(","))
    } else {
        visible.join(",")
    }
}

pub(super) fn granted(value: bool) -> &'static str {
    if value {
        "granted"
    } else {
        "missing"
    }
}
