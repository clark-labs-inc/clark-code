//! Safe workspace image tools.
//!
//! `view_image` feeds a contained raster image back through the typed tool
//! result channel. `generate_image` calls Clark Code's authenticated platform relay,
//! writes the returned bytes through the active executor (local or remote), and
//! emits a typed artifact instead of relying on a prose convention.

use std::path::{Path, PathBuf};

use agent_core::domain::{ArtifactKind, ToolKind};
use async_trait::async_trait;
use base64::Engine as _;
use serde_json::{json, Value};

use super::{arg_str, arg_str_opt, ProducedArtifact, ToolCtx, ToolExecutor, ToolOutcome};

mod relay;

const MAX_PROMPT_BYTES: usize = 16_000;
const MAX_INPUT_IMAGES: usize = 5;
const MAX_INPUT_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// Clark Code Platform relay configuration. The platform key is user-scoped; the
/// server alone owns the upstream image-provider credential and billing.
#[derive(Clone, Debug)]
pub struct ImageGenerationConfig {
    pub base_url: String,
    pub api_key: String,
    /// Host-selected chat model, used by the relay for billing policy.
    pub model: String,
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

#[cfg(test)]
mod tests;
