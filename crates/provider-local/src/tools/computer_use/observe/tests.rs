use super::*;

#[test]
fn global_window_discovery_requires_a_human_even_under_full_access() {
    let tool = ListWindows::new(Arc::new(computer_use::SimulatedComputerBackend::new()));
    let scope = tool.permission_scope(&json!({})).unwrap();
    assert_eq!(scope.key, "computer:window-discovery");
    assert_eq!(scope.risk.as_deref(), Some("confirm"));
    assert!(scope.remember);
}

#[test]
fn observation_presentation_exposes_settling_and_bounded_diffs() {
    let settlement = computer_use::ObservationSettlement {
        stable: true,
        elapsed_ms: 200,
        samples: 3,
    };
    assert_eq!(
        presentation::format_settlement(&settlement),
        "Accessibility settling: stable=true samples=3 elapsed_ms=200"
    );

    let diff = computer_use::AccessibilityDiff {
        base_observation_id: "obs-before".to_string(),
        added_ids: (0..25).map(|index| format!("added-{index}")).collect(),
        removed_ids: vec!["removed-1".to_string()],
        changed: vec![computer_use::ElementChange {
            id: "ax-3".to_string(),
            fields: vec!["value".to_string(), "focused".to_string()],
        }],
        focus_changed: true,
    };
    let rendered = presentation::format_diff(Some(&diff));
    assert!(rendered.contains("Accessibility diff from obs-before"));
    assert!(rendered.contains("added-19,…"));
    assert!(!rendered.contains("added-20"));
    assert!(rendered.contains("ax-3:value,focused"));
    assert!(rendered.contains("focus_changed=true"));
    assert_eq!(
        presentation::format_diff(None),
        "Accessibility diff: baseline established"
    );
}
