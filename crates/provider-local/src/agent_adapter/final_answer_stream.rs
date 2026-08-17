//! Incremental projection of the `final_answer.content` JSON string.
//!
//! OpenAI-compatible providers stream tool-call arguments as JSON fragments.
//! The terminal answer lives in that tool rather than ordinary assistant text,
//! so waiting for tool execution would turn a real provider stream into one
//! large final UI update. This parser exposes only completed words from the
//! single string field and reconciles them with the validated tool result.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::llm::output_quarantine;
use crate::tools::final_answer::FINAL_ANSWER_TOOL;

#[derive(Default)]
struct Candidate {
    name: String,
    arguments: String,
    emitted: String,
    quarantined: bool,
}

#[derive(Default)]
pub(super) struct FinalAnswerStreams {
    candidates: BTreeMap<usize, Candidate>,
    active: BTreeMap<String, String>,
}

impl FinalAnswerStreams {
    pub(super) fn reset_message(&mut self) {
        self.candidates.clear();
    }

    pub(super) fn observe_delta(
        &mut self,
        index: usize,
        name_delta: Option<&str>,
        arguments_delta: Option<&str>,
    ) -> Option<String> {
        let candidate = self.candidates.entry(index).or_default();
        if let Some(name) = name_delta {
            candidate.name.push_str(name);
        }
        if let Some(arguments) = arguments_delta {
            candidate.arguments.push_str(arguments);
        }
        if candidate.name != FINAL_ANSWER_TOOL || candidate.quarantined {
            return None;
        }

        let (content, complete) = partial_json_string_field(&candidate.arguments, "content")?;
        // `FinalAnswer::invoke` trims the outer whitespace before publishing
        // the validated result. Apply the same canonicalization here so a
        // model-generated leading/trailing space cannot make completion emit
        // the entire answer a second time.
        let content = if complete {
            content.trim()
        } else {
            content.trim_start()
        };
        let visible_end = if complete {
            content.len()
        } else {
            content
                .char_indices()
                .rev()
                .find_map(|(index, character)| {
                    character
                        .is_whitespace()
                        .then_some(index + character.len_utf8())
                })
                .unwrap_or(0)
        };
        let visible = &content[..visible_end];
        if !visible.starts_with(&candidate.emitted) {
            candidate.quarantined = true;
            return None;
        }
        if output_quarantine::contains_reserved_protocol_marker(visible) {
            candidate.quarantined = true;
            return None;
        }
        let delta = visible[candidate.emitted.len()..].to_string();
        candidate.emitted.push_str(&delta);
        (!delta.is_empty()).then_some(delta)
    }

    pub(super) fn begin(&mut self, tool_call_id: &str, args: &Value) {
        let Some(content) = args.get("content").and_then(Value::as_str).map(str::trim) else {
            self.active.insert(tool_call_id.to_string(), String::new());
            return;
        };
        let best = self
            .candidates
            .iter()
            .filter(|(_, candidate)| {
                candidate.name == FINAL_ANSWER_TOOL && content.starts_with(&candidate.emitted)
            })
            .max_by_key(|(_, candidate)| candidate.emitted.len())
            .map(|(index, candidate)| (*index, candidate.emitted.clone()));
        let emitted = best
            .and_then(|(index, emitted)| self.candidates.remove(&index).map(|_| emitted))
            .unwrap_or_default();
        self.active.insert(tool_call_id.to_string(), emitted);
    }

    /// Return only the validated suffix that was not already rendered from
    /// provider deltas. An empty suffix is the normal successful stream path.
    pub(super) fn finish(&mut self, tool_call_id: &str, answer: &str) -> String {
        let emitted = self.active.remove(tool_call_id).unwrap_or_default();
        answer.strip_prefix(&emitted).unwrap_or(answer).to_string()
    }
}

/// Decode the currently complete prefix of a JSON string field. The input may
/// end anywhere, including inside an escape or UTF-16 surrogate pair.
fn partial_json_string_field(input: &str, field: &str) -> Option<(String, bool)> {
    let needle = format!("\"{field}\"");
    let after_key = input.split_once(&needle)?.1.trim_start();
    let raw = after_key
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    let bytes = raw.as_bytes();
    let mut index = 0;
    let mut safe_end = 0;
    let mut complete = false;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                safe_end = index;
                complete = true;
                break;
            }
            b'\\' => {
                let escape = *bytes.get(index + 1)?;
                if escape == b'u' {
                    let first_end = index + 6;
                    if first_end > bytes.len() {
                        break;
                    }
                    let first = u16::from_str_radix(&raw[index + 2..first_end], 16).ok()?;
                    if (0xD800..=0xDBFF).contains(&first) {
                        let second_end = first_end + 6;
                        if second_end > bytes.len() || &bytes[first_end..first_end + 2] != b"\\u" {
                            break;
                        }
                        let second =
                            u16::from_str_radix(&raw[first_end + 2..second_end], 16).ok()?;
                        if !(0xDC00..=0xDFFF).contains(&second) {
                            return None;
                        }
                        index = second_end;
                    } else if (0xDC00..=0xDFFF).contains(&first) {
                        return None;
                    } else {
                        index = first_end;
                    }
                } else if matches!(
                    escape,
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't'
                ) {
                    index += 2;
                } else {
                    return None;
                }
                safe_end = index;
            }
            _ => {
                let character = raw[index..].chars().next()?;
                index += character.len_utf8();
                safe_end = index;
            }
        }
    }

    let quoted = format!("\"{}\"", &raw[..safe_end]);
    serde_json::from_str::<String>(&quoted)
        .ok()
        .map(|decoded| (decoded, complete))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_completed_words_and_decodes_split_json_escapes() {
        let mut streams = FinalAnswerStreams::default();
        assert_eq!(
            streams.observe_delta(0, Some("final_"), Some("{\"content\":\"Clark ")),
            None,
        );
        assert_eq!(
            streams.observe_delta(0, Some("answer"), Some("streams\\nword")),
            Some("Clark streams\n".to_string()),
        );
        assert_eq!(
            streams.observe_delta(0, None, Some(" by wo")),
            Some("word by ".into())
        );
        assert_eq!(
            streams.observe_delta(0, None, Some("rd \\uD83D")),
            Some("word ".into()),
        );
        assert_eq!(
            streams.observe_delta(0, None, Some("\\uDE80\"}")),
            Some("🚀".into())
        );
    }

    #[test]
    fn validated_result_emits_only_the_unstreamed_suffix() {
        let mut streams = FinalAnswerStreams::default();
        assert_eq!(
            streams.observe_delta(
                0,
                Some(FINAL_ANSWER_TOOL),
                Some("{\"content\":\"Fixed and verified.\"}"),
            ),
            Some("Fixed and verified.".into()),
        );
        streams.begin(
            "answer-1",
            &serde_json::json!({"content": "Fixed and verified."}),
        );
        assert_eq!(streams.finish("answer-1", "Fixed and verified."), "");
    }

    #[test]
    fn incomplete_final_word_is_reconciled_at_tool_completion() {
        let mut streams = FinalAnswerStreams::default();
        assert_eq!(
            streams.observe_delta(
                0,
                Some(FINAL_ANSWER_TOOL),
                Some("{\"content\":\"Visible then pend"),
            ),
            Some("Visible then ".into()),
        );
        streams.begin(
            "answer-1",
            &serde_json::json!({"content": "Visible then pending"}),
        );
        assert_eq!(
            streams.finish("answer-1", "Visible then pending"),
            "pending"
        );
    }

    #[test]
    fn outer_whitespace_matches_the_validated_trimmed_answer() {
        let mut streams = FinalAnswerStreams::default();
        assert_eq!(
            streams.observe_delta(
                0,
                Some(FINAL_ANSWER_TOOL),
                Some("{\"content\":\"  Fixed and verified.  \"}"),
            ),
            Some("Fixed and verified.".into()),
        );
        streams.begin(
            "answer-1",
            &serde_json::json!({"content": "  Fixed and verified.  "}),
        );
        assert_eq!(streams.finish("answer-1", "Fixed and verified."), "");
    }
}
