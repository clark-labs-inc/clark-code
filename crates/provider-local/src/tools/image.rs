//! Safe workspace image tools.
//!
//! `view_image` feeds a contained raster image back through the typed tool
//! result channel. `generate_image` calls Clark Code's authenticated platform relay,
//! writes the returned bytes through the active executor (local or remote), and
//! emits a typed artifact instead of relying on a prose convention.

use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_core::domain::{ArtifactKind, ToolKind};
use async_trait::async_trait;
use base64::Engine as _;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{arg_str, arg_str_opt, ProducedArtifact, ToolCtx, ToolExecutor, ToolOutcome};

const MAX_PROMPT_BYTES: usize = 16_000;
const MAX_INPUT_IMAGES: usize = 5;
const MAX_INPUT_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_OUTPUT_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Clark Code Platform relay configuration. The platform key is user-scoped; the
/// server alone owns the upstream image-provider credential and billing.
#[derive(Clone, Debug)]
pub struct ImageGenerationConfig {
    pub base_url: String,
    pub api_key: String,
}

pub struct ViewImage;

#[async_trait]
impl ToolExecutor for ViewImage {
    fn name(&self) -> &str {
        "view_image"
    }

    fn description(&self) -> &str {
        "Inspect a PNG, JPEG, WebP, or GIF image inside the active workspace. Use this for screenshots, mockups, diagrams, and generated images. The path must be inside the workspace; image bytes are returned through the typed image channel, not as text."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Image path relative to the active workspace."}
            },
            "required": ["path"]
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::ViewImage
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let path = match arg_str(&args, "path") {
            Ok(path) => path,
            Err(error) => return ToolOutcome::error(error),
        };
        let image = match read_workspace_image(&path, ctx).await {
            Ok(image) => image,
            Err(error) => return ToolOutcome::error(error),
        };
        let display = ctx.sandbox.display(&image.path);
        let data_base64 = base64::engine::general_purpose::STANDARD.encode(&image.bytes);
        let artifact = ctx.sandbox.docs_root().and_then(|docs| {
            image.path.starts_with(docs).then(|| {
                let uri = image.path.to_string_lossy().into_owned();
                ProducedArtifact {
                    id: format!("shot:{uri}"),
                    title: image
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("workspace-image")
                        .to_string(),
                    kind: ArtifactKind::Image,
                    mime_type: Some(image.mime_type.to_string()),
                    uri: Some(uri),
                }
            })
        });
        let mut outcome = ToolOutcome::ok(format!(
            "Viewed {display} ({}; {} bytes).",
            image.mime_type,
            image.bytes.len()
        ))
        .with_location(display.clone(), None)
        .with_image(
            image.mime_type,
            data_base64,
            Some(format!("Workspace image: {display}")),
        );
        if let Some(artifact) = artifact {
            outcome = outcome.with_artifact(artifact);
        }
        outcome
    }
}

pub struct GenerateImage {
    config: ImageGenerationConfig,
}

impl GenerateImage {
    pub fn new(config: ImageGenerationConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ToolExecutor for GenerateImage {
    fn name(&self) -> &str {
        "generate_image"
    }

    fn description(&self) -> &str {
        "Generate a still image from a prompt, or edit/reference one or more existing workspace images. Saves the result in the active workspace and shows it as an image artifact. Draft `prompt` first, then `input_images` only for edits/references, then `output_path`. Input paths must be workspace images; output_path is a workspace-relative image file path whose extension is normalized to the returned image format."
    }

    fn parameters(&self) -> Value {
        // Property order is deliberate: the model commits to the visual intent,
        // then references, then the irreversible output destination.
        json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "What to generate, or the edit to apply to the reference image(s)."},
                "input_images": {"type": "array", "items": {"type": "string"}, "description": "Optional image paths relative to the active workspace, for edits or visual references (maximum 5)."},
                "output_path": {"type": "string", "description": "Optional destination path relative to the active workspace. The saved extension matches the returned image format (defaults to images/<prompt-slug>.<returned-format>)."}
            },
            "required": ["prompt"]
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::GenerateImage
    }

    fn mutating(&self) -> bool {
        true
    }

    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        let prompt_value = arg_str_opt(args, "prompt")?;
        let prompt = prompt_value.trim();
        if prompt.is_empty() {
            return None;
        }
        let output = requested_output_path(args, prompt);
        Some(format!(
            "Generate an image through Clark Code (may consume credits)\nPrompt: {}\nSave to: {output} (extension matches returned image)",
            preview_prompt(prompt)
        ))
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let prompt = match arg_str(&args, "prompt") {
            Ok(prompt) => prompt.trim().to_string(),
            Err(error) => return ToolOutcome::error(error),
        };
        if prompt.is_empty() {
            return ToolOutcome::error("prompt must not be empty");
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return ToolOutcome::error(format!(
                "prompt is too large (max {MAX_PROMPT_BYTES} bytes)"
            ));
        }

        let requested_output_path = requested_output_path(&args, &prompt);
        // Validate containment before the billed request. The exact extension is
        // chosen only after the relay tells us the actual returned image format.
        if let Err(error) = ctx.sandbox.resolve_for_write(&requested_output_path) {
            return ToolOutcome::error(error);
        }

        let input_paths = match string_array(&args, "input_images") {
            Ok(paths) => paths,
            Err(error) => return ToolOutcome::error(error),
        };
        if input_paths.len() > MAX_INPUT_IMAGES {
            return ToolOutcome::error(format!(
                "input_images accepts at most {MAX_INPUT_IMAGES} images"
            ));
        }

        let mut input_images = Vec::with_capacity(input_paths.len());
        for path in input_paths {
            if ctx.cancel.is_cancelled() {
                return ToolOutcome::error("image generation cancelled");
            }
            let image = match read_workspace_image(&path, ctx).await {
                Ok(image) => image,
                Err(error) => return ToolOutcome::error(error),
            };
            input_images.push(format!(
                "data:{};base64,{}",
                image.mime_type,
                base64::engine::general_purpose::STANDARD.encode(image.bytes)
            ));
        }

        ctx.report(if input_images.is_empty() {
            "Generating image through Clark Code…"
        } else {
            "Generating image with workspace references through Clark Code…"
        });
        let generated = match self.generate(&prompt, input_images, &ctx.cancel).await {
            Ok(generated) => generated,
            Err(error) => return ToolOutcome::error(error),
        };

        let output_path = output_path_for_mime(&requested_output_path, generated.mime_type);
        let output = match ctx.sandbox.resolve_for_write(&output_path) {
            Ok(path) => path,
            Err(error) => return ToolOutcome::error(error),
        };
        if let Err(error) = ctx.guard_mutation(&output, false).await {
            return ToolOutcome::error(error);
        }

        if let Some(parent) = output.parent() {
            if let Err(error) = ctx.executor.create_dir_all(parent).await {
                return ToolOutcome::error(format!("creating {}: {error}", parent.display()));
            }
        }
        if let Err(error) = ctx.executor.write(&output, &generated.bytes).await {
            return ToolOutcome::error(format!("writing {output_path}: {error}"));
        }
        ctx.note_read(&output).await;

        let display = ctx.sandbox.display(&output);
        let data_base64 = base64::engine::general_purpose::STANDARD.encode(&generated.bytes);
        let data_url = format!("data:{};base64,{data_base64}", generated.mime_type);
        let artifact = ProducedArtifact {
            id: format!("image:{display}"),
            title: output
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("generated-image")
                .to_string(),
            kind: ArtifactKind::Image,
            mime_type: Some(generated.mime_type.to_string()),
            // A data URL is intentional: it renders safely for local and remote
            // workspaces without broadening Clark Code's file-read scope.
            uri: Some(data_url),
        };
        ToolOutcome::ok(format!(
            "Generated an image and saved it to {display} ({}; {} bytes).",
            generated.mime_type,
            generated.bytes.len()
        ))
        .with_location(display.clone(), None)
        .with_image(
            generated.mime_type,
            data_base64,
            Some(format!("Generated image: {display}")),
        )
        .with_artifact(artifact)
    }
}

impl GenerateImage {
    async fn generate(
        &self,
        prompt: &str,
        input_images: Vec<String>,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<GeneratedImage, String> {
        let client = desktop_http::build_client(desktop_http::ClientOptions {
            request_timeout: Some(REQUEST_TIMEOUT),
            user_agent: Some("agent-desktop"),
            ..Default::default()
        })
        .map_err(|error| format!("creating image client: {error}"))?;
        let url = generation_url(&self.config.base_url);
        let request = client
            .post(url)
            .bearer_auth(&self.config.api_key)
            .header("idempotency-key", uuid::Uuid::new_v4().to_string())
            .json(&json!({"prompt": prompt, "input_images": input_images}));
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err("image generation cancelled".to_string()),
            response = request.send() => response.map_err(|error| format!("calling Clark Code image generation: {error}"))?,
        };
        parse_generation_response(response).await
    }
}

struct WorkspaceImage {
    path: PathBuf,
    mime_type: &'static str,
    bytes: Vec<u8>,
}

async fn read_workspace_image(path: &str, ctx: &ToolCtx) -> Result<WorkspaceImage, String> {
    let resolved = ctx.sandbox.resolve_existing(path)?;
    let meta = ctx
        .executor
        .metadata(&resolved)
        .await
        .map_err(|error| format!("{path}: {error}"))?;
    if meta.is_dir {
        return Err(format!("{path} is a directory, not an image"));
    }
    if meta.len > MAX_INPUT_IMAGE_BYTES as u64 {
        return Err(format!(
            "{path} is too large to inspect (max {MAX_INPUT_IMAGE_BYTES} bytes)"
        ));
    }
    let bytes = ctx
        .executor
        .read(&resolved)
        .await
        .map_err(|error| format!("{path}: {error}"))?;
    if bytes.len() > MAX_INPUT_IMAGE_BYTES {
        return Err(format!(
            "{path} is too large to inspect (max {MAX_INPUT_IMAGE_BYTES} bytes)"
        ));
    }
    let mime_type = detect_image_mime(&bytes).ok_or_else(|| {
        format!("{path} is not a supported PNG, JPEG, WebP, or GIF image (validated from bytes)")
    })?;
    ctx.note_read(&resolved).await;
    Ok(WorkspaceImage {
        path: resolved,
        mime_type,
        bytes,
    })
}

#[derive(Deserialize)]
struct ImageGenerationResponse {
    #[serde(default)]
    data: Vec<ImageGenerationData>,
}

#[derive(Deserialize)]
struct ImageGenerationData {
    b64_json: String,
    #[serde(default, alias = "mime_type")]
    media_type: Option<String>,
}

struct GeneratedImage {
    bytes: Vec<u8>,
    mime_type: &'static str,
}

async fn parse_generation_response(response: reqwest::Response) -> Result<GeneratedImage, String> {
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("reading Clark Code image response: {error}"))?;
    if status != StatusCode::OK {
        return Err(format!(
            "Clark Code image generation returned HTTP {status}: {}",
            compact_response(&body)
        ));
    }
    let parsed: ImageGenerationResponse = serde_json::from_slice(&body).map_err(|error| {
        format!(
            "Clark Code image generation returned an invalid response ({error}): {}",
            compact_response(&body)
        )
    })?;
    let image = parsed
        .data
        .into_iter()
        .next()
        .ok_or_else(|| "Clark Code image generation completed without an image".to_string())?;
    decode_generated_image(image)
}

fn decode_generated_image(image: ImageGenerationData) -> Result<GeneratedImage, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image.b64_json.as_bytes())
        .map_err(|error| format!("decoding generated image: {error}"))?;
    if bytes.len() > MAX_OUTPUT_IMAGE_BYTES {
        return Err(format!(
            "generated image is too large (max {MAX_OUTPUT_IMAGE_BYTES} bytes)"
        ));
    }
    let detected = detect_image_mime(&bytes).ok_or_else(|| {
        "Clark Code returned bytes that are not a supported raster image".to_string()
    })?;
    if let Some(reported) = image.media_type.as_deref() {
        let reported = normalize_image_mime(reported).ok_or_else(|| {
            "Clark Code returned an unsupported generated image MIME type".to_string()
        })?;
        if reported != detected {
            return Err(format!(
                "Clark Code returned inconsistent generated image MIME types ({reported} vs {detected})"
            ));
        }
    }
    Ok(GeneratedImage {
        bytes,
        mime_type: detected,
    })
}

fn generation_url(base_url: &str) -> String {
    format!("{}/images/generations", base_url.trim_end_matches('/'))
}

fn string_array(args: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("{key} must be an array of image paths"))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(ToString::to_string)
                .ok_or_else(|| format!("{key} must contain only non-empty strings"))
        })
        .collect()
}

/// The requested destination before the relay reports the output MIME type.
/// A suffix is added or corrected after generation so the on-disk filename
/// always describes the bytes it contains.
fn requested_output_path(args: &Value, prompt: &str) -> String {
    arg_str_opt(args, "output_path")
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| format!("images/{}", prompt_slug(prompt)))
}

fn output_path_for_mime(requested: &str, mime_type: &str) -> String {
    let path = Path::new(requested);
    if output_extension_matches_mime(path, mime_type) {
        return requested.to_string();
    }
    path.with_extension(image_extension(mime_type))
        .to_string_lossy()
        .into_owned()
}

fn output_extension_matches_mime(path: &Path, mime_type: &str) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    matches!(
        (mime_type, extension.to_ascii_lowercase().as_str()),
        ("image/png", "png")
            | ("image/jpeg", "jpg" | "jpeg")
            | ("image/webp", "webp")
            | ("image/gif", "gif")
    )
}

fn image_extension(mime_type: &str) -> &'static str {
    match mime_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        // GeneratedImage is constructed only after `detect_image_mime`, so
        // this branch remains defensive if a future caller bypasses it.
        _ => "img",
    }
}

fn preview_prompt(prompt: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 300;
    let mut preview = prompt.chars().take(MAX_PREVIEW_CHARS).collect::<String>();
    if prompt.chars().nth(MAX_PREVIEW_CHARS).is_some() {
        preview.push('…');
    }
    preview
}

fn prompt_slug(prompt: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for character in prompt.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            dash = false;
        } else if !slug.is_empty() && !dash {
            slug.push('-');
            dash = true;
        }
        if slug.len() >= 48 {
            break;
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "generated-image".to_string()
    } else {
        trimmed.to_string()
    }
}

fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

fn normalize_image_mime(value: &str) -> Option<&'static str> {
    match value
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" => Some("image/jpeg"),
        "image/webp" => Some("image/webp"),
        "image/gif" => Some("image/gif"),
        _ => None,
    }
}

fn compact_response(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .take(300)
        .collect::<String>()
        .replace('\n', " ")
}

#[cfg(test)]
mod tests {
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
        let dir = tempfile::tempdir().unwrap();
        let outcome = GenerateImage::new(ImageGenerationConfig { base_url, api_key })
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
}
