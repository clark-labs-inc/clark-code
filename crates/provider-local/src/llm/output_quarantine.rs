use super::{AssistantTurn, ChatContent, ChatMessage, ContentPart};

/// A provider response crossed a boundary that must never reach tools, history,
/// or the UI. Codes are deliberately content-free so diagnostics can be
/// persisted without repeating the quarantined material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OutputViolation {
    ReservedProtocolMarker,
    UnpromptedIdentityResidue,
}

impl OutputViolation {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::ReservedProtocolMarker => "reserved_protocol_marker",
            Self::UnpromptedIdentityResidue => "unprompted_identity_residue",
        }
    }
}

pub(super) fn inspect(
    turn: &AssistantTurn,
    request_messages: &[ChatMessage],
) -> Option<OutputViolation> {
    let reserved_marker = std::iter::once(turn.text.as_str())
        .chain(std::iter::once(turn.reasoning.as_str()))
        .chain(turn.tool_calls.iter().flat_map(|call| {
            [
                call.function.name.as_str(),
                call.function.arguments.as_str(),
            ]
        }))
        .any(contains_reserved_protocol_marker)
        || turn
            .reasoning_details
            .iter()
            .filter_map(|detail| serde_json::to_string(detail).ok())
            .any(|detail| contains_reserved_protocol_marker(&detail));
    if reserved_marker {
        return Some(OutputViolation::ReservedProtocolMarker);
    }

    let candidate = turn.text.trim();
    if turn.tool_calls.is_empty()
        && looks_like_unprompted_identity_residue(candidate)
        && !request_contains(request_messages, candidate)
    {
        return Some(OutputViolation::UnpromptedIdentityResidue);
    }
    None
}

pub(crate) fn contains_reserved_protocol_marker(text: &str) -> bool {
    let normalized = collapse_underscores(text);
    [
        "begin_of_sentence",
        "require_escalated_model",
        "expiration_placeholder",
        "skillconstraint_hard",
    ]
    .into_iter()
    .any(|marker| normalized.contains(marker))
}

/// Holds the initial token and then only the currently-generating word while
/// publishing completed words.
///
/// Reserved provider-control markers are identifier-like and never contain
/// whitespace. Keeping the open word private preserves marker quarantine
/// across arbitrary SSE chunk boundaries. Retaining the first token until a
/// second begins also preserves the complete-turn check for unprompted
/// identity residue, which is structurally a single identifier-like token.
pub(super) struct StreamingGuard<'a, Sink> {
    pending: String,
    sink: &'a mut Sink,
    violation: Option<OutputViolation>,
    published: bool,
}

impl<'a, Sink> StreamingGuard<'a, Sink>
where
    Sink: FnMut(&str),
{
    pub(super) fn new(sink: &'a mut Sink) -> Self {
        Self {
            pending: String::new(),
            sink,
            violation: None,
            published: false,
        }
    }

    pub(super) fn push(&mut self, delta: &str) {
        if delta.is_empty() || self.violation.is_some() {
            return;
        }
        self.pending.push_str(delta);
        if !self.published && self.pending.split_whitespace().nth(1).is_none() {
            return;
        }
        let Some(boundary) = self
            .pending
            .char_indices()
            .rev()
            .find_map(|(index, character)| {
                character
                    .is_whitespace()
                    .then_some(index + character.len_utf8())
            })
        else {
            return;
        };
        let completed = self.pending[..boundary].to_string();
        if contains_reserved_protocol_marker(&completed) {
            self.violation = Some(OutputViolation::ReservedProtocolMarker);
            self.pending.clear();
            return;
        }
        self.pending.drain(..boundary);
        if !completed.is_empty() {
            (self.sink)(&completed);
            self.published = true;
        }
    }

    pub(super) fn flush(&mut self) {
        if self.violation.is_some() || self.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        (self.sink)(&pending);
        self.published = true;
    }

    pub(super) fn published(&self) -> bool {
        self.published
    }
}

fn collapse_underscores(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut previous_underscore = false;
    for character in text.chars() {
        for character in character.to_lowercase() {
            // Some tokenizers expose their word-boundary sentinel as U+2581
            // rather than an ASCII underscore. Fullwidth low line appears in
            // the same provider-residue family after copy/paste.
            if matches!(character, '_' | '\u{2581}' | '\u{ff3f}') {
                if !previous_underscore {
                    normalized.push('_');
                }
                previous_underscore = true;
            } else {
                normalized.push(character);
                previous_underscore = false;
            }
        }
    }
    normalized
}

/// The incident included a complete assistant turn containing only an invalid
/// identifier-like host token that had never appeared in the request. Ordinary
/// domains remain valid output; the underscore or model-style `@` is the
/// structural signal that this is provider/session residue rather than prose.
fn looks_like_unprompted_identity_residue(text: &str) -> bool {
    (4..=256).contains(&text.len())
        && text.contains('.')
        && (text.contains('_') || text.contains('@'))
        && text.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '@')
        })
}

fn request_contains(messages: &[ChatMessage], candidate: &str) -> bool {
    messages.iter().any(|message| {
        message
            .content
            .as_ref()
            .is_some_and(|content| match content {
                ChatContent::Text(text) => contains_case_insensitive(text, candidate),
                ChatContent::Parts(parts) => parts.iter().any(|part| match part {
                    ContentPart::Text { text } => contains_case_insensitive(text, candidate),
                    ContentPart::ImageUrl { .. } => false,
                }),
            })
            || message
                .tool_calls
                .iter()
                .any(|call| contains_case_insensitive(&call.function.arguments, candidate))
    })
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::llm::{WireFunction, WireToolCall};

    fn turn(text: &str) -> AssistantTurn {
        AssistantTurn {
            text: text.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn rejects_reserved_protocol_markers_and_chunk_join_variants() {
        for text in [
            "<|begin_of_sentence|>",
            "<|begin__of__sentence|>",
            "<｜begin▁of▁sentence｜>",
            "<|begin＿of＿sentence|>",
            "prefix require_escalated_model:gpt suffix",
            "expiration_placeholder",
            "SKILLconstraint_hard",
        ] {
            assert_eq!(
                inspect(&turn(text), &[ChatMessage::user("ordinary request")]),
                Some(OutputViolation::ReservedProtocolMarker),
                "marker must be quarantined: {text}",
            );
        }
    }

    #[test]
    fn scans_hidden_reasoning_tool_arguments_and_reasoning_details() {
        let mut reasoning = turn("safe");
        reasoning.reasoning = "<|begin__of__sentence|>".into();
        assert_eq!(
            inspect(&reasoning, &[]),
            Some(OutputViolation::ReservedProtocolMarker)
        );

        let mut tool = turn("");
        tool.tool_calls.push(WireToolCall {
            id: "call-1".into(),
            kind: "function".into(),
            function: WireFunction {
                name: "shell".into(),
                arguments: r#"{"value":"expiration_placeholder"}"#.into(),
            },
        });
        assert_eq!(
            inspect(&tool, &[]),
            Some(OutputViolation::ReservedProtocolMarker)
        );

        let mut details = turn("safe");
        details.reasoning_details = vec![json!({"opaque": "SKILLconstraint_hard"})];
        assert_eq!(
            inspect(&details, &[]),
            Some(OutputViolation::ReservedProtocolMarker)
        );
    }

    #[test]
    fn rejects_unprompted_identity_residue_but_allows_requested_echoes() {
        let leaked = turn("foreign_identity.example.com");
        assert_eq!(
            inspect(&leaked, &[ChatMessage::user("check the branch")]),
            Some(OutputViolation::UnpromptedIdentityResidue)
        );
        assert_eq!(
            inspect(
                &leaked,
                &[ChatMessage::user(
                    "Return foreign_identity.example.com exactly"
                )]
            ),
            None
        );
    }

    #[test]
    fn allows_normal_prose_domains_and_identifier_discussion() {
        for text in [
            "Use example.com for the callback.",
            "example.com",
            "The identifier foreign_identity.example.com is malformed.",
            "Branch cleanup is complete.",
        ] {
            assert_eq!(
                inspect(&turn(text), &[]),
                None,
                "safe text rejected: {text}"
            );
        }
    }

    #[test]
    fn streaming_guard_publishes_completed_words_one_delta_behind() {
        let mut published = Vec::new();
        {
            let mut publish = |text: &str| published.push(text.to_string());
            let mut guard = StreamingGuard::new(&mut publish);
            guard.push("Clark");
            guard.push(" streams");
            guard.push(" smoothly");
            guard.flush();
            assert!(guard.published());
        }
        assert_eq!(published, ["Clark ", "streams ", "smoothly"]);
    }

    #[test]
    fn streaming_guard_withholds_reserved_marker_split_across_deltas() {
        let mut published = Vec::new();
        {
            let mut publish = |text: &str| published.push(text.to_string());
            let mut guard = StreamingGuard::new(&mut publish);
            guard.push("apparently safe ");
            guard.push("<|begin__of");
            guard.push("__sentence|> ");
            guard.push("must stay hidden");
            guard.flush();
        }
        assert_eq!(published, ["apparently safe "]);
    }

    #[test]
    fn streaming_guard_keeps_a_single_identity_token_private() {
        let mut published = Vec::new();
        {
            let mut publish = |text: &str| published.push(text.to_string());
            let mut guard = StreamingGuard::new(&mut publish);
            guard.push("foreign_identity.example.com");
            guard.push("\n");
            assert!(!guard.published());
        }
        assert!(published.is_empty());
    }
}
