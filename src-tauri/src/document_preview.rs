use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{ipc::Response, AppHandle, Manager};
use uuid::Uuid;

const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RENDERED_BYTES: usize = 32 * 1024 * 1024;
const MAX_HTML_BYTES: usize = 4 * 1024 * 1024;
const MAX_PAGES: usize = 100;
const PREVIEW_DPI: u32 = 110;
const PREVIEW_FORMATS: &[&str] = &[
    "doc", "docx", "odt", "pdf", "xls", "xlsx", "ods", "csv", "ppt", "pptx", "odp",
];
const PREVIEW_HEAD: &str = r#"<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'">
<style>
  :root { color-scheme: light; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
  * { box-sizing: border-box; }
  body { max-width: 72rem; margin: 0 auto; padding: 3.5rem 4rem 6rem; color: #202124; background: #fff; font-size: 15px; line-height: 1.65; }
  h1, h2, h3, h4, h5, h6 { color: #171717; line-height: 1.25; margin: 1.6em 0 .65em; }
  h1:first-child, h2:first-child, h3:first-child { margin-top: 0; }
  p { margin: 0 0 1em; }
  a { color: #5b45d6; }
  table { width: 100%; border-collapse: collapse; margin: 1.25rem 0 2.5rem; font-size: 13px; }
  td, th { border: 1px solid #d8d8dc; padding: .5rem .65rem; text-align: left; vertical-align: top; }
  th { position: sticky; top: 0; background: #f4f4f6; font-weight: 650; }
  code { padding: .12em .3em; border-radius: .25rem; background: #f1f1f4; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
  .slide { aspect-ratio: 16 / 9; overflow: auto; margin: 0 auto 2rem; padding: 2rem 2.5rem; border: 1px solid #dddde2; border-radius: .5rem; box-shadow: 0 8px 24px rgba(0,0,0,.08); }
  .slide h2 { margin-top: 0; }
  @media (max-width: 720px) { body { padding: 2rem 1.5rem 4rem; } }
</style>"#;

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentPreview {
    Html {
        html: String,
    },
    Pages {
        preview_id: String,
        page_count: usize,
    },
}

/// Render a supported office file from Agent Desktop's app-managed document workspace.
/// Writer documents, sheets, and PDFs use compact semantic HTML. Presentations
/// prefer paginated PNG output because slide geometry is part of the content;
/// raster failures fall back to semantic HTML.
#[tauri::command]
pub async fn render_document_preview(
    app: AppHandle,
    path: String,
) -> Result<DocumentPreview, String> {
    let root = provider_local::workspace_root()
        .ok_or_else(|| "no workspace directory".to_string())?
        .canonicalize()
        .map_err(|error| format!("workspace: {error}"))?;
    let canon = PathBuf::from(&path)
        .canonicalize()
        .map_err(|error| format!("{path}: {error}"))?;
    if !canon.starts_with(&root) {
        return Err("path is outside the document workspace".into());
    }
    let format = preview_format(&canon).ok_or_else(|| "unsupported preview format".to_string())?;
    let metadata = std::fs::metadata(&canon).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("not a file".into());
    }
    if metadata.len() > MAX_DOCUMENT_BYTES {
        return Err("document too large to preview".into());
    }

    let preview_root = preview_root(&app)?;
    tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(&canon).map_err(|error| format!("read failed: {error}"))?;
        render_preview(&bytes, &format, &preview_root)
    })
    .await
    .map_err(|error| format!("preview failed: {error}"))?
}

/// Return one cached preview page as a raw IPC response. Tauri delivers this
/// to JavaScript as an ArrayBuffer without JSON or base64 serialization.
#[tauri::command]
pub async fn read_document_preview_page(
    app: AppHandle,
    preview_id: String,
    page: usize,
) -> Result<Response, String> {
    let directory = preview_directory(&app, &preview_id)?;
    if page >= MAX_PAGES {
        return Err("preview page is out of range".into());
    }
    let path = directory.join(format!("page-{page:03}.png"));
    let bytes = tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .map_err(|error| format!("read preview page: {error}"))?
        .map_err(|error| format!("read preview page: {error}"))?;
    Ok(Response::new(bytes))
}

/// Remove one generated page-preview directory. UUID validation makes the
/// cleanup target an exact child of Agent Desktop's app cache, never a caller path.
#[tauri::command]
pub async fn cleanup_document_preview(app: AppHandle, preview_id: String) -> Result<(), String> {
    let directory = preview_directory(&app, &preview_id)?;
    tokio::task::spawn_blocking(move || match std::fs::remove_dir_all(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove document preview: {error}")),
    })
    .await
    .map_err(|error| format!("remove document preview: {error}"))?
}

fn preview_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_cache_dir()
        .map(|path| path.join("document-previews"))
        .map_err(|error| format!("document preview cache: {error}"))
}

fn preview_directory(app: &AppHandle, preview_id: &str) -> Result<PathBuf, String> {
    let id = Uuid::parse_str(preview_id).map_err(|_| "invalid document preview id".to_string())?;
    Ok(preview_root(app)?.join(id.to_string()))
}

fn preview_format(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    PREVIEW_FORMATS
        .contains(&extension.as_str())
        .then_some(extension)
}

fn render_preview(
    bytes: &[u8],
    format: &str,
    preview_root: &Path,
) -> Result<DocumentPreview, String> {
    if let Ok(pages) = render_pages(bytes, format) {
        if let Ok(preview) = cache_pages(pages, preview_root) {
            return Ok(preview);
        }
    }
    Ok(DocumentPreview::Html {
        html: render_html(bytes, format)?,
    })
}

fn render_pages(bytes: &[u8], format: &str) -> Result<Vec<Vec<u8>>, String> {
    match format {
        "pptx" => libreoffice_pure::pptx_to_png_pages(bytes, PREVIEW_DPI),
        "ppt" | "odp" => {
            let pptx = libreoffice_pure::convert_bytes(bytes, format, "pptx")
                .map_err(|error| error.to_string())?;
            libreoffice_pure::pptx_to_png_pages(&pptx, PREVIEW_DPI)
        }
        _ => return Err("page rendering is not available for this format".into()),
    }
    .map_err(|error| error.to_string())
}

fn cache_pages(pages: Vec<Vec<u8>>, preview_root: &Path) -> Result<DocumentPreview, String> {
    if pages.is_empty() {
        return Err("document renderer returned no pages".into());
    }
    if pages.len() > MAX_PAGES {
        return Err(format!(
            "document has too many pages to preview (max {MAX_PAGES})"
        ));
    }
    let rendered_bytes = pages.iter().map(Vec::len).sum::<usize>();
    if rendered_bytes > MAX_RENDERED_BYTES {
        return Err("rendered document is too large to preview".into());
    }
    let preview_id = Uuid::new_v4().to_string();
    let directory = preview_root.join(&preview_id);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("create document preview cache: {error}"))?;
    let page_count = pages.len();
    for (index, page) in pages.into_iter().enumerate() {
        if let Err(error) = std::fs::write(directory.join(format!("page-{index:03}.png")), page) {
            let _ = std::fs::remove_dir_all(&directory);
            return Err(format!("write document preview page: {error}"));
        }
    }
    Ok(DocumentPreview::Pages {
        preview_id,
        page_count,
    })
}

fn render_html(bytes: &[u8], format: &str) -> Result<String, String> {
    let rendered = libreoffice_pure::convert_bytes(bytes, format, "html")
        .map_err(|error| format!("{} rendering failed: {error}", format.to_ascii_uppercase()))?;
    if rendered.len() > MAX_HTML_BYTES {
        return Err("rendered document is too large to preview".into());
    }
    let html = String::from_utf8(rendered)
        .map_err(|error| format!("document renderer returned invalid text: {error}"))?;
    if !html.contains("</head>") {
        return Err("document renderer returned an invalid document".into());
    }
    Ok(html.replacen("</head>", &format!("{PREVIEW_HEAD}\n</head>"), 1))
}

#[cfg(test)]
mod tests {
    use super::{preview_format, render_html, render_preview, DocumentPreview};
    use std::path::Path;

    #[test]
    fn renders_docx_as_compact_semantic_html() {
        let docx = libreoffice_pure::convert_bytes(
            b"# Native preview\n\nHello **Agent Desktop**.",
            "md",
            "docx",
        )
        .expect("create DOCX fixture");

        let root = tempfile::tempdir().expect("preview cache");
        let preview = render_preview(&docx, "docx", root.path()).expect("render DOCX preview");
        let DocumentPreview::Html { html } = preview else {
            panic!("expected semantic HTML preview");
        };
        assert!(html.contains("Native preview"));
        assert!(html.len() < 16 * 1024);
    }

    #[test]
    fn renders_spreadsheet_as_styled_inert_html() {
        let xlsx =
            libreoffice_pure::convert_bytes(b"name,value\nAgent Desktop,42\n", "csv", "xlsx")
                .expect("create XLSX fixture");
        let html = render_html(&xlsx, "xlsx").expect("render XLSX preview");
        assert!(html.contains("Agent Desktop"));
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("default-src 'none'"));
    }

    #[test]
    fn accepts_only_supported_office_extensions() {
        assert_eq!(
            preview_format(Path::new("report.PDF")).as_deref(),
            Some("pdf")
        );
        assert_eq!(
            preview_format(Path::new("sheet.ods")).as_deref(),
            Some("ods")
        );
        assert!(preview_format(Path::new("payload.html")).is_none());
    }

    #[test]
    fn rejects_invalid_document_bytes() {
        assert!(render_html(b"not a zip file", "docx").is_err());
    }
}
