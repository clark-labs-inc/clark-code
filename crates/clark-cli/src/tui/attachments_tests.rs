use agent_core::AttachmentKind;

use super::*;

fn capabilities(kinds: Vec<AttachmentKind>) -> ProviderCapabilities {
    ProviderCapabilities {
        attachment_kinds: kinds,
        ..Default::default()
    }
}

#[test]
fn exact_image_bytes_filename_and_mime_survive_submission() {
    let root = tempfile::tempdir().unwrap();
    let bytes = [0_u8, 1, 2, 254, 255];
    std::fs::write(root.path().join("evidence.png"), bytes).unwrap();
    let mut input = AttachmentInput::default();
    input
        .execute(
            "/attach evidence.png",
            root.path(),
            &capabilities(vec![AttachmentKind::Image]),
        )
        .unwrap()
        .unwrap();

    let prompt = input.prompt("inspect it".into());
    assert_eq!(prompt.attachments.len(), 1);
    assert_eq!(prompt.attachments[0].filename, "evidence.png");
    assert_eq!(prompt.attachments[0].content_type, "image/png");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&prompt.attachments[0].data_base64)
            .unwrap(),
        bytes
    );
    assert_eq!(input.count(), 1);
    input.clear_after_start();
    assert_eq!(input.count(), 0);
}

#[test]
fn unsupported_media_fails_before_staging_any_file() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("voice.wav"), b"audio").unwrap();
    let mut input = AttachmentInput::default();
    let error = input
        .execute(
            "/attach voice.wav",
            root.path(),
            &capabilities(vec![AttachmentKind::Text]),
        )
        .unwrap()
        .unwrap_err();
    assert!(error.contains("no provider turn started"));
    assert_eq!(input.count(), 0);
}

#[test]
fn fuzzy_results_are_stable_and_require_exact_selection() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src/nested")).unwrap();
    std::fs::write(root.path().join("src/parser.rs"), "one").unwrap();
    std::fs::write(root.path().join("src/nested/parser_test.rs"), "two").unwrap();
    let mut input = AttachmentInput::default();
    let report = input
        .execute(
            "/attach parser",
            root.path(),
            &capabilities(vec![AttachmentKind::Text]),
        )
        .unwrap()
        .unwrap();
    assert!(report.contains("1. src/parser.rs"));
    assert_eq!(input.count(), 0);
    input
        .execute(
            "/attach 1",
            root.path(),
            &capabilities(vec![AttachmentKind::Text]),
        )
        .unwrap()
        .unwrap();
    assert_eq!(input.count(), 1);
}

#[test]
fn editor_context_is_atomic_when_one_file_is_invalid() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".clark")).unwrap();
    std::fs::write(root.path().join("valid.rs"), "valid").unwrap();
    std::fs::write(
        root.path().join(".clark/ide-context.json"),
        r#"{"files":[{"path":"valid.rs"},{"path":"missing.rs"}]}"#,
    )
    .unwrap();
    let mut input = AttachmentInput::default();
    let error = input
        .execute(
            "/attach --ide",
            root.path(),
            &capabilities(vec![AttachmentKind::Text]),
        )
        .unwrap()
        .unwrap_err();
    assert!(error.contains("missing.rs"));
    assert_eq!(input.count(), 0);
}

#[test]
fn legacy_imported_attachment_commands_are_not_accepted() {
    assert!(!AttachmentInput::handles_line("/mention evidence.png"));
    assert!(!AttachmentInput::handles_line("/ide src/main.rs@1"));
    assert!(AttachmentInput::handles_line("/attach --ide src/main.rs@1"));
}
