use super::relay::{decode_generated_image, generation_url, ImageGenerationData};
use super::*;
use crate::sandbox::Sandbox;
use crate::tools::ReadTracker;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn ctx(dir: &Path) -> ToolCtx {
    ToolCtx {
        sandbox: Arc::new(Sandbox::new(dir).unwrap()),
        executor: Arc::new(crate::exec::LocalExecutor),
        reads: Arc::new(Mutex::new(ReadTracker::default())),
        cancel: CancellationToken::new(),
        background: Arc::new(crate::background::BackgroundTasks::default()),
        session: Arc::new(tokio::sync::Mutex::new(
            crate::loop_state::SessionState::default(),
        )),
        progress: None,
        agent_progress: None,
        call_progress: None,
        model_override: None,
    }
}

fn docs_ctx(dir: &Path) -> ToolCtx {
    let mut context = ctx(dir);
    context.sandbox = Arc::new(Sandbox::new(dir).unwrap().with_docs(dir.to_path_buf()));
    context
}

#[tokio::test]
async fn view_image_returns_a_typed_image_for_a_contained_png() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("design.png"),
        base64::engine::general_purpose::STANDARD
            .decode(PNG_1X1)
            .unwrap(),
    )
    .unwrap();

    let outcome = ViewImage
        .invoke(json!({"path": "design.png"}), &ctx(dir.path()))
        .await;

    assert!(!outcome.is_error, "{}", outcome.content);
    assert_eq!(outcome.images.len(), 1);
    assert_eq!(outcome.images[0].mime_type, "image/png");
    assert_eq!(outcome.locations[0].path, "design.png");
    assert!(outcome.artifacts.is_empty());
}

#[tokio::test]
async fn view_image_promotes_managed_workspace_image_to_a_durable_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("spectro.png");
    std::fs::write(
        &path,
        base64::engine::general_purpose::STANDARD
            .decode(PNG_1X1)
            .unwrap(),
    )
    .unwrap();

    let outcome = ViewImage
        .invoke(json!({"path": "spectro.png"}), &docs_ctx(dir.path()))
        .await;

    assert!(!outcome.is_error, "{}", outcome.content);
    assert_eq!(outcome.artifacts.len(), 1);
    let artifact = &outcome.artifacts[0];
    let canonical = path.canonicalize().unwrap();
    assert_eq!(artifact.id, format!("shot:{}", canonical.display()));
    assert_eq!(artifact.title, "spectro.png");
    assert_eq!(artifact.kind, ArtifactKind::Image);
    assert_eq!(artifact.mime_type.as_deref(), Some("image/png"));
    assert_eq!(artifact.uri.as_deref(), canonical.to_str());
}

#[tokio::test]
async fn view_image_rejects_non_image_bytes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("not-an-image.png"), "not image bytes").unwrap();

    let outcome = ViewImage
        .invoke(json!({"path": "not-an-image.png"}), &ctx(dir.path()))
        .await;

    assert!(outcome.is_error);
    assert!(outcome.content.contains("validated from bytes"));
}

#[test]
fn image_wire_helpers_validate_mime_and_match_output_extensions() {
    assert_eq!(
        detect_image_mime(b"\x89PNG\r\n\x1a\nrest"),
        Some("image/png")
    );
    assert_eq!(
        normalize_image_mime("image/jpeg; charset=binary"),
        Some("image/jpeg")
    );
    assert_eq!(prompt_slug("A neon fox / city"), "a-neon-fox-city");
    assert_eq!(
        requested_output_path(&json!({}), "A neon fox"),
        "images/a-neon-fox"
    );
    assert_eq!(
        output_path_for_mime("images/a-neon-fox", "image/jpeg"),
        "images/a-neon-fox.jpg"
    );
    assert_eq!(
        output_path_for_mime("images/a-neon-fox.png", "image/jpeg"),
        "images/a-neon-fox.jpg"
    );
    assert_eq!(
        output_path_for_mime("images/a-neon-fox.jpeg", "image/jpeg"),
        "images/a-neon-fox.jpeg"
    );
    assert_eq!(
        output_path_for_mime("images/a-neon-fox.png", "image/png"),
        "images/a-neon-fox.png"
    );
    assert_eq!(preview_prompt("neon fox"), "neon fox");
    assert!(preview_prompt(&"x".repeat(301)).ends_with('…'));
    assert_eq!(
        generation_url("https://product.example/v1/"),
        "https://product.example/v1/images/generations"
    );
}

#[test]
fn platform_generation_payload_requires_consistent_raster_metadata() {
    let generated = decode_generated_image(ImageGenerationData {
        b64_json: PNG_1X1.to_string(),
        media_type: Some("image/png".to_string()),
    })
    .expect("valid platform image response");
    assert_eq!(generated.mime_type, "image/png");
    assert!(!generated.bytes.is_empty());

    let error = decode_generated_image(ImageGenerationData {
        b64_json: PNG_1X1.to_string(),
        media_type: Some("image/jpeg".to_string()),
    })
    .err()
    .expect("mismatched media type must fail");
    assert!(error.contains("inconsistent"));
}

#[ignore = "paid live image generation; run only with explicit user authorization"]
#[tokio::test]
async fn paid_generation_writes_a_filename_matching_the_returned_image_format() {
    let api_key = std::env::var("IMAGE_GENERATION_E2E_API_KEY")
        .expect("set IMAGE_GENERATION_E2E_API_KEY for the paid image E2E test");
    let base_url = std::env::var("IMAGE_GENERATION_E2E_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:11434/v1".to_string());
    let model = std::env::var("IMAGE_GENERATION_E2E_MODEL")
        .expect("set IMAGE_GENERATION_E2E_MODEL to the host-selected chat model");
    let dir = tempfile::tempdir().unwrap();
    let outcome = GenerateImage::new(ImageGenerationConfig {
        base_url,
        api_key,
        model,
    })
    .invoke(
        json!({
            "prompt": "A single cobalt-blue circle on an off-white background, no text.",
            // Deliberately request the wrong suffix; the relay decides
            // the actual output format and the tool must name it truthfully.
            "output_path": "paid-e2e/result.png",
        }),
        &ctx(dir.path()),
    )
    .await;

    assert!(!outcome.is_error, "{}", outcome.content);
    let mime_type = &outcome.images[0].mime_type;
    let saved = &outcome.locations[0].path;
    assert_eq!(
        saved,
        &output_path_for_mime("paid-e2e/result.png", mime_type)
    );
    assert!(dir.path().join(saved).is_file());
    assert_eq!(
        detect_image_mime(&std::fs::read(dir.path().join(saved)).unwrap()),
        Some(mime_type.as_str())
    );
}
