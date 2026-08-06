//! Private-reasoning rendering for the local compaction adapter.
//!
//! The public compaction crate owns the generic transcript protocol. This
//! adapter owns the `clark_agent`-specific boundary: it supplies readable
//! findings to the summarizer and withholds opaque provider replay payloads.

use clark_agent::{AssistantBlock, AssistantContent, ReasoningItem};
const HEADER: &str =
    "[private reasoning — non-user context; distill durable findings, do not quote]\n";

/// Translate Clark Agent's typed reasoning blocks into the public compaction
/// kernel's safe text-only boundary. Signatures and encrypted replay payloads
/// never leave this adapter.
pub(super) fn append_readable_findings(content: &AssistantContent, out: &mut String) -> bool {
    let findings = readable_reasoning(content);
    let findings = findings.trim();
    if findings.is_empty() {
        return false;
    }

    if !out.ends_with("\n") {
        out.push('\n');
    }
    out.push_str(HEADER);
    out.push_str(findings);
    out.push('\n');
    true
}

fn readable_reasoning(content: &AssistantContent) -> String {
    let mut parts = Vec::new();
    for block in &content.blocks {
        match block {
            AssistantBlock::Thinking(text) | AssistantBlock::Reasoning(text) => {
                if !text.text.trim().is_empty() {
                    parts.push(text.text.clone());
                }
            }
            AssistantBlock::ReasoningDetails(details) => {
                for item in details.as_items() {
                    match item {
                        ReasoningItem::Text { text, .. } if !text.trim().is_empty() => {
                            parts.push(text);
                        }
                        ReasoningItem::Summary { summary, .. } if !summary.trim().is_empty() => {
                            parts.push(summary);
                        }
                        ReasoningItem::Encrypted { .. }
                        | ReasoningItem::Text { .. }
                        | ReasoningItem::Summary { .. } => {}
                    }
                }
            }
            AssistantBlock::Text(_) | AssistantBlock::ToolCall(_) => {}
        }
    }
    parts.join("\n")
}
