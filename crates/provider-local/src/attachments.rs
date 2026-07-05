//! Attachment ingestion for turn text: neither `clark-code` (GLM 5.2) nor
//! `clark-code:kimi_k27_code` (Kimi K2.7 Code) is vision-capable, so images
//! are described by a fallback call to Clark's agentic tier and the
//! description is inlined as text. PDF/DOCX get their text extracted and
//! inlined (capped + truncation-noted), the same way `prompt_text` already
//! inlines a plain-text attachment. Anything else gets an honest
//! "content not available" note — never a bare filename, which invites the
//! model to `find`/`ls` a file that only ever existed as inline base64.

use agent_core::domain::PendingUpload;
use base64::Engine as _;
use docx_rs::{DocumentChild, ParagraphChild, RunChild};
use tokio_util::sync::CancellationToken;

use crate::config::AgenticClarkConfig;
use crate::llm::LlmClient;

/// Cap on characters inlined from one extracted PDF/DOCX. ~20,000 tokens at
/// the 4-chars/token heuristic `clark-agent-compaction` itself uses to size
/// `DEFAULT_COMPACT_RECENT_USER_TOKEN_BUDGET` — generous for a long spec/report
/// while staying a small fraction of the auto-compact token budgets.
const MAX_EXTRACTED_DOC_CHARS: usize = 80_000;

const VISION_SYSTEM_PROMPT: &str = "You are helping a coding agent that cannot see images itself. \
Describe each attached image thoroughly and precisely: transcribe ALL visible text verbatim \
(UI labels, buttons, error messages, stack traces, code, terminal output, file paths), describe \
layout and relevant colors/icons, and note anything actionable. If there are multiple images, \
address them in order (Image 1, Image 2, ...). Do not speculate about content outside the image.";

enum DocKind {
    Pdf,
    Docx,
}

/// Process every attachment `prompt_text` doesn't already inline (i.e.
/// everything with `!att.is_text()`): PDF/DOCX get extracted text inlined;
/// every image in the turn is described by ONE batched vision-model call;
/// anything else gets an honest note. Returns `""` when there's nothing to do.
pub(crate) async fn process_attachments(
    attachments: &[PendingUpload],
    user_text: &str,
    vision: Option<&AgenticClarkConfig>,
    cancel: &CancellationToken,
) -> String {
    let mut out = String::new();
    let mut images = Vec::new();

    for att in attachments {
        if att.is_text() {
            continue; // already inlined by `prompt_text`.
        }
        if att.is_image() {
            images.push(att);
            continue;
        }
        match sniff_doc_kind(att) {
            Some(kind) => out.push_str(&extract_doc_text(att, kind).await),
            None => out.push_str(&unavailable_note(&att.filename, "binary attachment")),
        }
    }

    if !images.is_empty() {
        out.push_str(&describe_images_block(&images, user_text, vision, cancel).await);
    }

    out
}

/// Recognize PDF/DOCX by MIME first, falling back to a filename-extension
/// check — necessary because the client coerces an empty/unknown `file.type`
/// to `application/octet-stream` (`app/src/lib/attachments.ts`), and legacy
/// DOCX sometimes arrives as `application/zip`.
fn sniff_doc_kind(att: &PendingUpload) -> Option<DocKind> {
    match att.content_type.to_ascii_lowercase().as_str() {
        "application/pdf" => return Some(DocKind::Pdf),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            return Some(DocKind::Docx)
        }
        _ => {}
    }
    let name = att.filename.to_ascii_lowercase();
    if name.ends_with(".pdf") {
        return Some(DocKind::Pdf);
    }
    if name.ends_with(".docx") {
        return Some(DocKind::Docx);
    }
    None
}

/// Decode + extract text on a blocking thread (both parsers are sync,
/// CPU-bound work over untrusted, user-supplied bytes up to the client's
/// 12MB attachment cap — matches the `spawn_blocking` precedent at
/// `engine.rs`'s git-checkpoint step; a panicking parse surfaces as a
/// `JoinError` here instead of taking down the run).
async fn extract_doc_text(att: &PendingUpload, kind: DocKind) -> String {
    let label = match kind {
        DocKind::Pdf => "PDF",
        DocKind::Docx => "DOCX",
    };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&att.data_base64) else {
        return unavailable_note(&att.filename, "corrupt attachment data");
    };

    match tokio::task::spawn_blocking(move || extract_bytes(&bytes, kind)).await {
        Ok(Ok(text)) => inline_doc_block(&att.filename, label, &text),
        Ok(Err(e)) => unavailable_note(&att.filename, &format!("could not extract text ({e})")),
        Err(_) => unavailable_note(&att.filename, "text extraction crashed"),
    }
}

fn extract_bytes(bytes: &[u8], kind: DocKind) -> Result<String, String> {
    match kind {
        DocKind::Pdf => pdf_extract::extract_text_from_mem(bytes).map_err(|e| e.to_string()),
        DocKind::Docx => extract_docx_text(bytes),
    }
}

/// Best-effort, paragraph-body text only — table cells and tracked-changes/
/// hyperlink runs are not walked. A documented v1 scope, not silently claimed
/// as exhaustive.
fn extract_docx_text(bytes: &[u8]) -> Result<String, String> {
    let docx = docx_rs::read_docx(bytes).map_err(|e| e.to_string())?;
    let mut text = String::new();
    for child in &docx.document.children {
        let DocumentChild::Paragraph(paragraph) = child else {
            continue;
        };
        for pc in &paragraph.children {
            let ParagraphChild::Run(run) = pc else {
                continue;
            };
            for rc in &run.children {
                if let RunChild::Text(t) = rc {
                    text.push_str(&t.text);
                }
            }
        }
        text.push('\n');
    }
    Ok(text)
}

fn inline_doc_block(filename: &str, label: &str, text: &str) -> String {
    let total = text.chars().count();
    if total <= MAX_EXTRACTED_DOC_CHARS {
        return format!(
            "\n\n--- attached file: {filename} ({label}, text extracted) ---\n{text}\n"
        );
    }
    let truncated: String = text.chars().take(MAX_EXTRACTED_DOC_CHARS).collect();
    format!(
        "\n\n--- attached file: {filename} ({label}, text extracted, TRUNCATED) ---\n\
         {truncated}\n\
         [... extraction truncated: {total} characters extracted, only the first \
         {MAX_EXTRACTED_DOC_CHARS} shown to preserve context budget ...]\n"
    )
}

/// An honest note for content the model cannot access — deliberately never a
/// bare filename, which is exactly what sent GLM 5.2 hunting the filesystem
/// for a file that only ever existed as inline base64.
fn unavailable_note(filename: &str, reason: &str) -> String {
    format!(
        "\n\n[attached file: {filename} — {reason}; content not available to you. \
         It exists only as data the user's client sent, not as a file on disk.]"
    )
}

/// Batch every image in the turn into ONE vision-model call (not one call per
/// image) and splice the result in as a single labeled block.
async fn describe_images_block(
    images: &[&PendingUpload],
    user_text: &str,
    vision: Option<&AgenticClarkConfig>,
    cancel: &CancellationToken,
) -> String {
    let Some(vision) = vision else {
        return images
            .iter()
            .map(|att| unavailable_note(&att.filename, "no vision model configured"))
            .collect();
    };
    let Ok(client) = LlmClient::from_parts(
        &vision.base_url,
        &vision.model,
        vision.api_key.clone(),
        Vec::new(),
        None,
    ) else {
        return images
            .iter()
            .map(|att| unavailable_note(&att.filename, "vision client could not be built"))
            .collect();
    };

    let mut prompt = String::new();
    let mut urls = Vec::with_capacity(images.len());
    for (i, att) in images.iter().enumerate() {
        prompt.push_str(&format!("Image {}: {}\n", i + 1, att.filename));
        urls.push(format!(
            "data:{};base64,{}",
            att.content_type, att.data_base64
        ));
    }
    if !user_text.trim().is_empty() {
        prompt.push_str("\nContext from the user's message:\n");
        prompt.push_str(user_text);
    }

    match client
        .describe_images(VISION_SYSTEM_PROMPT, &prompt, urls, cancel)
        .await
    {
        Ok(description) if !description.is_empty() => format!(
            "\n\n[{n} image attachment(s) processed by a vision model — the active coding \
             model can't read images directly]\n{description}\n",
            n = images.len()
        ),
        Ok(_) => images
            .iter()
            .map(|att| unavailable_note(&att.filename, "vision model returned no description"))
            .collect(),
        Err(e) => images
            .iter()
            .map(|att| unavailable_note(&att.filename, &format!("vision call failed ({e})")))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload(filename: &str, content_type: &str, data_base64: &str) -> PendingUpload {
        PendingUpload {
            filename: filename.into(),
            content_type: content_type.into(),
            data_base64: data_base64.into(),
        }
    }

    #[test]
    fn sniffs_pdf_and_docx_by_mime() {
        let pdf = upload("report.bin", "application/pdf", "");
        assert!(matches!(sniff_doc_kind(&pdf), Some(DocKind::Pdf)));

        let docx = upload(
            "report.bin",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "",
        );
        assert!(matches!(sniff_doc_kind(&docx), Some(DocKind::Docx)));
    }

    #[test]
    fn falls_back_to_filename_extension_when_mime_is_generic() {
        let pdf = upload("spec.PDF", "application/octet-stream", "");
        assert!(matches!(sniff_doc_kind(&pdf), Some(DocKind::Pdf)));

        let docx = upload("notes.docx", "application/zip", "");
        assert!(matches!(sniff_doc_kind(&docx), Some(DocKind::Docx)));
    }

    #[test]
    fn unrecognized_attachment_sniffs_to_none() {
        let bin = upload("archive.tar.gz", "application/gzip", "");
        assert!(sniff_doc_kind(&bin).is_none());
    }

    #[test]
    fn inline_doc_block_passes_short_text_through_unmodified() {
        let block = inline_doc_block("a.pdf", "PDF", "hello world");
        assert!(block.contains("hello world"));
        assert!(!block.contains("TRUNCATED"));
    }

    #[test]
    fn inline_doc_block_truncates_and_notes_it() {
        let long = "x".repeat(MAX_EXTRACTED_DOC_CHARS + 500);
        let block = inline_doc_block("a.pdf", "PDF", &long);
        assert!(block.contains("TRUNCATED"));
        assert!(block.contains(&format!("{} characters extracted", long.chars().count())));
        // Only the capped prefix is inlined, not the full text.
        assert!(!block.contains(&"x".repeat(MAX_EXTRACTED_DOC_CHARS + 1)));
    }

    #[test]
    fn unavailable_note_never_implies_an_on_disk_file() {
        let note = unavailable_note("image.webp", "no vision model configured");
        assert!(note.contains("not available to you"));
        assert!(note.contains("not as a file on disk"));
    }

    #[test]
    fn extracts_text_from_an_in_memory_docx_round_trip() {
        use docx_rs::{Docx, Paragraph, Run};
        use std::io::Cursor;

        let docx = Docx::new()
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("hello from docx")));
        let mut cursor = Cursor::new(Vec::new());
        docx.build().pack(&mut cursor).expect("pack docx");

        let text = extract_docx_text(cursor.get_ref()).expect("extract docx text");
        assert!(text.contains("hello from docx"));
    }
}
