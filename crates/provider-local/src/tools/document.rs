//! Pure-Rust document conversion through the published libreoffice-rs crates.

use std::path::PathBuf;

use agent_core::domain::{ArtifactKind, ToolKind};
use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{arg_str, arg_str_opt, ProducedArtifact, ToolCtx, ToolExecutor, ToolOutcome};

const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

pub struct DocumentConvert;

#[async_trait]
impl ToolExecutor for DocumentConvert {
    fn name(&self) -> &str {
        "document_convert"
    }

    fn description(&self) -> &str {
        "Convert office and document files with Clark's bundled pure-Rust libreoffice-rs engine. Use this instead of textutil, soffice, Pandoc, or ad hoc Python for supported HTML, Markdown, DOCX, ODT, PDF, XLSX, ODS, PPTX, ODP, CSV, SVG, and text conversions. Provide the input path first, then the target format, then an optional output path."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Source document path relative to the active workspace."},
                "to": {"type": "string", "description": "Target format extension, such as pdf, docx, odt, html, md, txt, xlsx, csv, pptx, or svg."},
                "output_path": {"type": "string", "description": "Optional destination path relative to the active workspace. Defaults to the source filename with the target extension."}
            },
            "required": ["path", "to"]
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }

    fn mutating(&self) -> bool {
        true
    }

    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        let source = arg_str_opt(args, "path")?;
        let target = canonical_format(&arg_str_opt(args, "to")?)?;
        let output = requested_output(args, &source, target).ok()?;
        Some(format!(
            "Convert document with libreoffice-rs\nSource: {source}\nTarget: {output} ({target})"
        ))
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let source_arg = match arg_str(&args, "path") {
            Ok(path) => path,
            Err(error) => return ToolOutcome::error(error),
        };
        let target_arg = match arg_str(&args, "to") {
            Ok(format) => format,
            Err(error) => return ToolOutcome::error(error),
        };
        let target = match canonical_format(&target_arg) {
            Some(format) => format,
            None => return ToolOutcome::error(format!("unsupported target format: {target_arg}")),
        };
        let output_arg = match requested_output(&args, &source_arg, target) {
            Ok(path) => path,
            Err(error) => return ToolOutcome::error(error),
        };

        let source = match ctx.sandbox.resolve_existing(&source_arg) {
            Ok(path) => path,
            Err(error) => return ToolOutcome::error(error),
        };
        let output = match ctx.sandbox.resolve_for_write(&output_arg) {
            Ok(path) => path,
            Err(error) => return ToolOutcome::error(error),
        };
        if source == output {
            return ToolOutcome::error("source and destination must be different files");
        }
        if let Err(error) = ctx.guard_mutation(&output, false).await {
            return ToolOutcome::error(error);
        }

        let input = match ctx.executor.read(&source).await {
            Ok(bytes) => bytes,
            Err(error) => return ToolOutcome::error(format!("reading {source_arg}: {error}")),
        };
        if input.len() > MAX_DOCUMENT_BYTES {
            return ToolOutcome::error(format!(
                "document is too large (max {} MiB)",
                MAX_DOCUMENT_BYTES / (1024 * 1024)
            ));
        }

        ctx.report(format!(
            "Converting {source_arg} to {target} with libreoffice-rs…"
        ));
        let converter_path = source.clone();
        let converted = match tokio::task::spawn_blocking(move || {
            let path = converter_path.to_string_lossy();
            libreoffice_pure::convert_path_bytes(&path, &input, target)
                .map_err(|error| error.to_string())
        })
        .await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => return ToolOutcome::error(format!("conversion failed: {error}")),
            Err(error) => return ToolOutcome::error(format!("conversion task failed: {error}")),
        };

        if let Some(parent) = output.parent() {
            if let Err(error) = ctx.executor.create_dir_all(parent).await {
                return ToolOutcome::error(format!("creating {}: {error}", parent.display()));
            }
        }
        if let Err(error) = ctx.executor.write(&output, &converted).await {
            return ToolOutcome::error(format!("writing {output_arg}: {error}"));
        }
        ctx.note_read(&output).await;

        let display = ctx.sandbox.display(&output);
        let size_bytes = converted.len();
        let sha256 = format!("{:x}", Sha256::digest(&converted));
        let content_type = mime_type(target).map(str::to_string);
        ToolOutcome::ok(format!(
            "Converted {source_arg} to {display} with libreoffice-rs ({size_bytes} bytes; sha256 {sha256})."
        ))
        .with_location(display.clone(), None)
        .with_artifact(ProducedArtifact {
            id: format!("document:{display}"),
            title: output
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Converted document")
                .to_string(),
            kind: artifact_kind(target),
            mime_type: content_type.clone(),
            uri: Some(output.to_string_lossy().into_owned()),
        })
        .with_details(json!({
            "artifact": {
                "path": display,
                "mime_type": content_type,
                "size_bytes": size_bytes,
                "sha256": sha256,
            }
        }))
    }
}

fn canonical_format(format: &str) -> Option<&'static str> {
    match format
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => Some("pdf"),
        "docx" => Some("docx"),
        "odt" => Some("odt"),
        "html" | "htm" => Some("html"),
        "md" | "markdown" => Some("md"),
        "txt" | "text" => Some("txt"),
        "xlsx" => Some("xlsx"),
        "ods" => Some("ods"),
        "csv" => Some("csv"),
        "pptx" => Some("pptx"),
        "odp" => Some("odp"),
        "svg" => Some("svg"),
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpeg"),
        _ => None,
    }
}

fn requested_output(args: &Value, source: &str, target: &str) -> Result<String, String> {
    let mut output = arg_str_opt(args, "output_path")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(source));
    if output.as_os_str().is_empty() {
        return Err("output_path must not be empty".to_string());
    }
    output.set_extension(target);
    Ok(output.to_string_lossy().into_owned())
}

fn artifact_kind(format: &str) -> ArtifactKind {
    match format {
        "pdf" => ArtifactKind::Pdf,
        "docx" | "odt" | "xlsx" | "ods" => ArtifactKind::Office,
        "pptx" | "odp" => ArtifactKind::Slides,
        "png" | "jpeg" | "svg" => ArtifactKind::Image,
        _ => ArtifactKind::File,
    }
}

fn mime_type(format: &str) -> Option<&'static str> {
    match format {
        "pdf" => Some("application/pdf"),
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "odt" => Some("application/vnd.oasis.opendocument.text"),
        "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "ods" => Some("application/vnd.oasis.opendocument.spreadsheet"),
        "pptx" => Some("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        "odp" => Some("application/vnd.oasis.opendocument.presentation"),
        "html" => Some("text/html"),
        "md" => Some("text/markdown"),
        "txt" | "csv" => Some("text/plain"),
        "svg" => Some("image/svg+xml"),
        "png" => Some("image/png"),
        "jpeg" => Some("image/jpeg"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::Sandbox;
    use crate::tools::ReadTracker;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn ctx(root: &Path) -> ToolCtx {
        ToolCtx {
            sandbox: Arc::new(Sandbox::new(root).unwrap()),
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
    async fn converts_markdown_to_tagged_pdf_without_system_utilities() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("report.md"),
            "# Rust report\n\nHello **Clark**.",
        )
        .unwrap();

        let outcome = DocumentConvert
            .invoke(json!({"path": "report.md", "to": "pdf"}), &ctx(root.path()))
            .await;

        assert!(!outcome.is_error, "{}", outcome.content);
        let pdf = std::fs::read(root.path().join("report.pdf")).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(String::from_utf8_lossy(&pdf).contains("/StructTreeRoot"));
        assert_eq!(outcome.artifacts[0].kind, ArtifactKind::Pdf);
        assert_eq!(outcome.details["artifact"]["size_bytes"], pdf.len());
        let expected_sha256 = format!("{:x}", Sha256::digest(&pdf));
        assert_eq!(outcome.details["artifact"]["sha256"], expected_sha256);
        assert!(outcome.content.contains(&expected_sha256));
    }

    #[tokio::test]
    async fn converts_html_to_real_docx_without_textutil() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("offer.html"), "<h1>Offer</h1><p>Terms</p>").unwrap();

        let outcome = DocumentConvert
            .invoke(
                json!({"path": "offer.html", "to": "docx", "output_path": "deliverables/offer.docx"}),
                &ctx(root.path()),
            )
            .await;

        assert!(!outcome.is_error, "{}", outcome.content);
        let docx = std::fs::read(root.path().join("deliverables/offer.docx")).unwrap();
        assert!(docx.starts_with(b"PK"));
        assert_eq!(outcome.artifacts[0].kind, ArtifactKind::Office);
    }
}
