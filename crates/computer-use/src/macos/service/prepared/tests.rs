use crate::{ComputerAction, Rect, WindowInfo, WindowTarget};

use super::redacted_preview;

fn window() -> WindowInfo {
    WindowInfo {
        target: WindowTarget {
            pid: 42,
            window_id: 7,
            bundle_id: "com.example.fixture".to_string(),
        },
        app_name: "Fixture".to_string(),
        title: "Sensitive document title".to_string(),
        frame: Rect::default(),
        layer: 0,
        on_screen: true,
    }
}

#[test]
fn native_previews_never_retain_text_or_numeric_control_values() {
    let secret = "sensitive-marker-that-must-not-persist";
    let typed = redacted_preview(
        &window(),
        &ComputerAction::TypeText {
            element_id: "ax-1".to_string(),
            text: secret.to_string(),
            replace: true,
        },
    );
    let numeric = redacted_preview(
        &window(),
        &ComputerAction::SetValue {
            element_id: "ax-2".to_string(),
            value: 73.125,
        },
    );

    let serialized = serde_json::to_string(&(typed, numeric)).unwrap();
    assert!(!serialized.contains(secret));
    assert!(!serialized.contains("73.125"));
    assert!(!serialized.contains("Sensitive document title"));
    assert!(serialized.contains("text redacted"));
    assert!(serialized.contains("numeric value redacted"));
}
