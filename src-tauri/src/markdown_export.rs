use std::path::{Path, PathBuf};

const MAX_MARKDOWN_BYTES: usize = 8 * 1024 * 1024;

/// Render the Markdown text already displayed by the artifact workspace and
/// write it to a user-selected PDF path. A local source path supplies the base
/// directory for relative images; remote documents still export when they do
/// not depend on local assets.
#[tauri::command]
pub async fn export_markdown_pdf(
    path: String,
    text: String,
    source_path: Option<String>,
) -> Result<(), String> {
    if text.len() > MAX_MARKDOWN_BYTES {
        return Err("document too large to export".into());
    }

    let destination = PathBuf::from(path);
    let source = source_path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("document.md"));

    tokio::task::spawn_blocking(move || {
        let pdf = render_markdown_pdf(&source, text.as_bytes())?;
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| format!("create dir: {error}"))?;
        }
        std::fs::write(&destination, pdf).map_err(|error| format!("write failed: {error}"))
    })
    .await
    .map_err(|error| format!("PDF export failed: {error}"))?
}

fn render_markdown_pdf(source: &Path, markdown: &[u8]) -> Result<Vec<u8>, String> {
    libreoffice_pure::markdown_to_pdf_bytes(source, markdown)
        .map_err(|error| format!("PDF rendering failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::render_markdown_pdf;

    #[test]
    fn renders_tagged_markdown_pdf_through_published_library() {
        let pdf = render_markdown_pdf(
            std::path::Path::new("artifact.md"),
            b"# Exported artifact\n\n<span style=\"color:#6d28d9\">Styled text</span>\n",
        )
        .expect("render Markdown PDF");

        assert!(pdf.starts_with(b"%PDF-"));
        let body = String::from_utf8_lossy(&pdf);
        assert!(body.contains("/StructTreeRoot"));
        assert!(body.contains("/MarkInfo << /Marked true >>"));
        assert!(body.contains("/S /H1"));
    }
}
