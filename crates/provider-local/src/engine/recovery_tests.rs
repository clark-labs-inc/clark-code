use super::*;

#[test]
fn recovery_marker_includes_bounded_background_completion() {
    let completion = crate::background::TaskStatus {
        command: "cargo check".to_string(),
        output: format!(
            "HEAD{}TAIL",
            "x".repeat(BACKGROUND_RECEIPT_OUTPUT_CHARS + 50)
        ),
        exit_code: Some(Some(101)),
        error: None,
    };

    let agent_loop::AgentMessage::User {
        content: agent_loop::UserContent::Text(content),
        ..
    } = transcript_marker(&[("bg-7".to_string(), completion)])
    else {
        panic!("recovery marker must be a user text message");
    };

    assert!(content.contains("Host-observed background completion `bg-7` (exit 101)"));
    assert!(content.contains("cargo check"));
    assert!(content.contains("characters omitted"));
    assert!(!content.contains("HEAD"));
    assert!(content.contains("TAIL"));
    assert!(content.contains("fix failures, and continue"));
}
