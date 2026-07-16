//! Prompt and per-turn environment rendering helpers.

use agent_core::domain::ContentBlock;
use agent_core::provider::PromptInput;

use crate::sandbox::Sandbox;

pub(super) fn environment_context(sandbox: &Sandbox, remote: bool) -> String {
    let mut roots = vec![sandbox.root().display().to_string()];
    if let Some(docs) = sandbox.docs_root() {
        roots.push(docs.display().to_string());
    }
    format!(
        "[runtime context — derived from the active session, not user instruction]\n\
<environment_context>\n  <cwd>{}</cwd>\n  <workspace_roots>{}</workspace_roots>\n  <remote>{remote}</remote>\n</environment_context>",
        sandbox.root().display(),
        roots.join(" | ")
    )
}

/// Flatten a prompt's text blocks (and inline any text attachments) into one
/// user message. Non-text attachments (images, PDFs, DOCX, anything else) are
/// handled separately by [`crate::attachments::process_attachments`], which
/// needs an async context this sync helper doesn't have.
pub(super) fn prompt_text(input: &PromptInput) -> String {
    let mut text: String = input
        .blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    for att in &input.attachments {
        if att.is_text() {
            if let Ok(decoded) = decode_base64_text(&att.data_base64) {
                text.push_str(&format!(
                    "\n\n--- attached file: {} ---\n{decoded}\n",
                    att.filename
                ));
            }
        }
    }
    text
}

/// Minimal standard-base64 decoder (no external dep) for inlining text files.
pub(super) fn decode_base64_text(data: &str) -> std::result::Result<String, ()> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in data.as_bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = val(c).ok_or(())? as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    String::from_utf8(out).map_err(|_| ())
}
