//! Middle-out truncation for tool output (the Codex approach): keep the head
//! and the tail — where commands put their setup and their verdict — and drop
//! the middle, labeling exactly what was cut. Applied when a tool result is
//! recorded, so one giant `cat`/`grep` can't flood the model's context and
//! ride there until compaction.

/// Default cap on a single tool result, in characters (~10k tokens at the
/// char/4 heuristic — the same budget Codex gives exec output).
pub const DEFAULT_TOOL_RESULT_MAX_CHARS: usize = 40_000;

/// Truncate `text` middle-out to at most roughly `max_chars`, returning `None`
/// when it already fits. The head and tail halves are kept on char
/// boundaries; the elision is labeled with the original size so the model
/// knows output was dropped (and can re-run with a narrower target).
pub fn truncate_middle(text: &str, max_chars: usize) -> Option<String> {
    if text.chars().count() <= max_chars {
        return None;
    }
    let total_chars = text.chars().count();
    let total_lines = text.lines().count();

    let keep = max_chars.saturating_sub(200).max(2); // header + elision marker slack
    let head_chars = keep / 2;
    let tail_chars = keep - head_chars;

    let head_end = text
        .char_indices()
        .nth(head_chars)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let tail_start = text
        .char_indices()
        .rev()
        .nth(tail_chars.saturating_sub(1))
        .map(|(i, _)| i)
        .unwrap_or(0);

    let head = &text[..head_end];
    let tail = &text[tail_start.max(head_end)..];
    let omitted = total_chars.saturating_sub(head.chars().count() + tail.chars().count());

    Some(format!(
        "Warning: truncated output ({total_chars} chars, {total_lines} lines in full). \
         Showing the beginning and the end; re-run with a narrower target (a path, a line \
         range, a stricter pattern) if the middle matters.\n\n{head}\n\n[... {omitted} chars \
         omitted ...]\n\n{tail}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_output_passes_through() {
        assert!(truncate_middle("hello", 100).is_none());
        assert!(truncate_middle("", 10).is_none());
    }

    #[test]
    fn long_output_keeps_head_and_tail_and_labels_the_cut() {
        let text: String = (0..5_000).map(|i| format!("line {i}\n")).collect();
        let out = truncate_middle(&text, 4_000).expect("must truncate");
        assert!(out.len() < text.len());
        assert!(out.starts_with("Warning: truncated output"));
        assert!(out.contains("line 0"), "head preserved");
        assert!(out.contains("line 4999"), "tail preserved");
        assert!(out.contains("chars omitted"));
        assert!(out.contains("5000 lines in full"));
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        let text = "é".repeat(50_000);
        let out = truncate_middle(&text, 10_000).expect("must truncate");
        assert!(out.contains("é"));
        // Would panic on a byte-boundary slice if boundaries were wrong.
        let _ = out.len();
    }

    #[test]
    fn result_is_roughly_within_budget() {
        let text = "x".repeat(200_000);
        let out = truncate_middle(&text, 40_000).expect("must truncate");
        assert!(out.chars().count() < 41_000);
    }
}
