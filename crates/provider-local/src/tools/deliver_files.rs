use std::collections::BTreeSet;

use agent_core::domain::{ArtifactKind, ToolKind};
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{ProducedArtifact, ToolCtx, ToolExecutor, ToolOutcome};

pub struct DeliverFiles;

#[async_trait]
impl ToolExecutor for DeliverFiles {
    fn name(&self) -> &str {
        "deliver_files"
    }

    fn description(&self) -> &str {
        "Attach existing generated files for the user to open or save. Validate every path before publishing attachments. After this tool succeeds, finish with an ordinary assistant response."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "minItems": 1,
                    "description": "Existing user-facing deliverable file paths, relative to the active workspace when possible. List generated reports, archives, images, documents, and other files the user should be able to open or save. Omit ordinary source files changed as part of implementation work."
                }
            },
            "required": ["files"]
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let files = match deliverable_files(&args, ctx) {
            Ok(files) => files,
            Err(error) => return ToolOutcome::error(error),
        };
        let mut outcome = ToolOutcome::ok("Files delivered. Finish with an ordinary assistant response.");
        for file in files {
            outcome = outcome.with_artifact(file);
        }
        outcome
    }
}

fn deliverable_files(args: &Value, ctx: &ToolCtx) -> Result<Vec<ProducedArtifact>, String> {
    let files = args
        .get("files")
        .ok_or_else(|| "`files` is required".to_string())?;
    let files = files
        .as_array()
        .ok_or_else(|| "`files` must be an array of existing file paths".to_string())?;
    if files.is_empty() {
        return Err("`files` must contain at least one existing file path".to_string());
    }
    let mut seen = BTreeSet::new();
    let mut artifacts = Vec::with_capacity(files.len());
    for file in files {
        let requested = file
            .as_str()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| "every `files` entry must be a non-empty path".to_string())?;
        let path = ctx
            .sandbox
            .resolve_existing(requested)
            .map_err(|error| format!("cannot deliver `{requested}`: {error}"))?;
        if !path.is_file() {
            return Err(format!("cannot deliver `{requested}`: path is not a file"));
        }
        let display = ctx.sandbox.display(&path);
        if !seen.insert(display.clone()) {
            continue;
        }
        let title = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Generated file")
            .to_string();
        let (kind, mime_type) = artifact_type(&path);
        artifacts.push(ProducedArtifact {
            id: format!("deliverable:{display}"),
            title,
            kind,
            mime_type: mime_type.map(str::to_string),
            uri: Some(path.to_string_lossy().into_owned()),
        });
    }
    Ok(artifacts)
}

fn artifact_type(path: &std::path::Path) -> (ArtifactKind, Option<&'static str>) {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("png") => (ArtifactKind::Image, Some("image/png")),
        Some("jpg" | "jpeg") => (ArtifactKind::Image, Some("image/jpeg")),
        Some("gif") => (ArtifactKind::Image, Some("image/gif")),
        Some("webp") => (ArtifactKind::Image, Some("image/webp")),
        Some("svg") => (ArtifactKind::Image, Some("image/svg+xml")),
        Some("pdf") => (ArtifactKind::Pdf, Some("application/pdf")),
        Some("docx") => (
            ArtifactKind::Office,
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        ),
        Some("xlsx") => (
            ArtifactKind::Office,
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        ),
        Some("pptx") => (
            ArtifactKind::Slides,
            Some("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        ),
        Some("mp4") => (ArtifactKind::Video, Some("video/mp4")),
        Some("webm") => (ArtifactKind::Video, Some("video/webm")),
        Some("mov") => (ArtifactKind::Video, Some("video/quicktime")),
        Some("mp3") => (ArtifactKind::Media, Some("audio/mpeg")),
        Some("wav") => (ArtifactKind::Media, Some("audio/wav")),
        Some("zip") => (ArtifactKind::File, Some("application/zip")),
        Some("json") => (ArtifactKind::File, Some("application/json")),
        Some("csv") => (ArtifactKind::File, Some("text/csv")),
        Some("txt") => (ArtifactKind::File, Some("text/plain")),
        Some("md" | "markdown") => (ArtifactKind::File, Some("text/markdown")),
        _ => (ArtifactKind::File, None),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio_util::sync::CancellationToken;

    use crate::tools::ReadTracker;

    use super::*;

    fn context(root: &std::path::Path) -> ToolCtx {
        ToolCtx {
            sandbox: Arc::new(crate::sandbox::Sandbox::new(root).unwrap()),
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

    #[tokio::test]
    async fn turns_validated_deliverable_paths_into_typed_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("parrot-emoji-pack.zip");
        std::fs::write(&archive, b"PK fake archive").unwrap();

        let outcome = DeliverFiles
            .invoke(
                json!({
                    "files": ["parrot-emoji-pack.zip", "parrot-emoji-pack.zip"]
                }),
                &context(root.path()),
            )
            .await;

        assert!(!outcome.is_error);
        assert_eq!(outcome.artifacts.len(), 1);
        assert_eq!(outcome.artifacts[0].id, "deliverable:parrot-emoji-pack.zip");
        assert_eq!(outcome.artifacts[0].title, "parrot-emoji-pack.zip");
        assert_eq!(outcome.artifacts[0].kind, ArtifactKind::File);
        assert_eq!(
            outcome.artifacts[0].mime_type.as_deref(),
            Some("application/zip")
        );
        assert_eq!(
            outcome.artifacts[0].uri.as_deref(),
            Some(archive.canonicalize().unwrap().to_string_lossy().as_ref())
        );
    }

    #[tokio::test]
    async fn refuses_to_claim_a_missing_deliverable() {
        let root = tempfile::tempdir().unwrap();
        let outcome = DeliverFiles
            .invoke(
                json!({
                    "files": ["missing.zip"]
                }),
                &context(root.path()),
            )
            .await;

        assert!(outcome.is_error);
        assert!(outcome.content.contains("cannot deliver `missing.zip`"));
        assert!(outcome.artifacts.is_empty());
    }
}
