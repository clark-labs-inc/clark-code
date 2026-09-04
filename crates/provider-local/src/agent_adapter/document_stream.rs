//! Incremental projection of a document being written by a tool call.
//!
//! A `write_file` whose payload is a whole document takes the model the
//! better part of a minute to emit. Those argument fragments already arrive
//! here token by token; without this module they are accumulated, discarded,
//! and the document only reaches the UI once the file is on disk — one large
//! update after a long silence. This parser exposes the completed words of the
//! one payload field so a reader watches the document being written.
//!
//! Two gates decide eligibility, both provider-side because the UI cannot
//! apply them in time:
//!
//! - **Tool**: only tools whose payload *is* a document a person is waiting to
//!   read. Anything carrying credentials or keystrokes (`computer_type_text`
//!   and friends, which `redaction` scrubs before persistence) must never
//!   stream — a partial JSON fragment cannot be redacted reliably.
//! - **Target**: only markdown files. The schemas are authored
//!   locate-before-payload (`path → content`), so the target is decodable
//!   before any payload arrives. The UI only learns the path once arguments
//!   validate — after the whole payload has streamed — which is too late to
//!   stop a source file from rendering as a document, and too late to avoid
//!   shipping every code write over IPC once per token.

use std::collections::BTreeMap;

use serde_json::Value;

use super::partial_json::partial_json_string_field;

/// The payload field to project, per eligible tool. `edit_file` streams its
/// replacement text; `old_string` is search input, not the document.
/// `apply_patch` payloads are code patches with no single target to gate on,
/// so they do not stream.
fn streamed_field(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "write_file" => Some("content"),
        "edit_file" => Some("new_string"),
        _ => None,
    }
}

fn is_markdown_path(path: &str) -> bool {
    path.rsplit('.').next().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
    })
}

/// How a validated payload reconciles with what its stream already rendered.
pub(super) enum SettledDocument {
    /// The stream was a faithful prefix; only the remainder is new.
    Append(String),
    /// The stream cannot be trusted as a prefix (the provider rewound or
    /// reordered its own arguments). Appending would splice two drafts
    /// together, so the rendered text must be replaced wholesale.
    Replace(String),
}

#[derive(Default)]
struct Candidate {
    id: String,
    name: String,
    arguments: String,
    emitted: String,
    quarantined: bool,
}

#[derive(Default)]
pub(super) struct DocumentStreams {
    candidates: BTreeMap<usize, Candidate>,
}

impl DocumentStreams {
    pub(super) fn reset_message(&mut self) {
        self.candidates.clear();
    }

    /// The next run of completed words, or `None` while the payload has not
    /// advanced past a word boundary.
    pub(super) fn observe_delta(
        &mut self,
        index: usize,
        id_delta: Option<&str>,
        name_delta: Option<&str>,
        arguments_delta: Option<&str>,
    ) -> Option<String> {
        let candidate = self.candidates.entry(index).or_default();
        if let Some(id) = id_delta {
            candidate.id.push_str(id);
        }
        if let Some(name) = name_delta {
            candidate.name.push_str(name);
        }
        if let Some(arguments) = arguments_delta {
            candidate.arguments.push_str(arguments);
        }
        if candidate.quarantined {
            return None;
        }
        let field = streamed_field(&candidate.name)?;

        // Target gate. The path precedes the payload in the schema, so waiting
        // for it to close costs nothing; judging a half-streamed path would.
        let (path, path_complete) = partial_json_string_field(&candidate.arguments, "path")?;
        if !path_complete {
            return None;
        }
        if !is_markdown_path(&path) {
            candidate.quarantined = true;
            return None;
        }

        let (text, complete) = partial_json_string_field(&candidate.arguments, field)?;
        // Hold back a partial trailing word so a reader never sees a fragment
        // resolve into a different word. A closed field has no such risk.
        let visible_end = if complete {
            text.len()
        } else {
            text.char_indices()
                .rev()
                .find_map(|(index, character)| {
                    character
                        .is_whitespace()
                        .then_some(index + character.len_utf8())
                })
                .unwrap_or(0)
        };
        let visible = &text[..visible_end];
        if !visible.starts_with(&candidate.emitted) {
            candidate.quarantined = true;
            return None;
        }
        let delta = visible[candidate.emitted.len()..].to_string();
        candidate.emitted.push_str(&delta);
        (!delta.is_empty()).then_some(delta)
    }

    /// Reconcile a call's validated payload with what its stream rendered.
    ///
    /// Called once arguments validate, which is still *before* the tool writes
    /// the file — so completing the document here is the earliest a reader can
    /// have all of it. `None` means nothing was streamed for this call (or the
    /// target is not a markdown document), so there is nothing to reconcile;
    /// the validated `raw_input` carries the payload for anything that wants it.
    ///
    /// Matched by call id, never by tool name: parallel writes in one message
    /// share a name, and two documents opening on the same heading would let the
    /// longer stream be claimed by whichever call settled first — dropping text
    /// from one document and repeating it in the other.
    pub(super) fn settle(
        &mut self,
        tool_call_id: &str,
        tool_name: &str,
        args: &Value,
    ) -> Option<SettledDocument> {
        let payload = streamed_field(tool_name)
            .and_then(|field| args.get(field))
            .and_then(Value::as_str)?;
        if !args
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(is_markdown_path)
        {
            return None;
        }
        let matched = self
            .candidates
            .iter()
            .find(|(_, candidate)| candidate.id == tool_call_id)
            .map(|(index, candidate)| (*index, candidate.quarantined, candidate.emitted.clone()));
        let (index, quarantined, emitted) = matched?;
        self.candidates.remove(&index);
        if quarantined || !payload.starts_with(&emitted) {
            return Some(SettledDocument::Replace(payload.to_string()));
        }
        let suffix = payload[emitted.len()..].to_string();
        (!suffix.is_empty()).then_some(SettledDocument::Append(suffix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append(settled: Option<SettledDocument>) -> Option<String> {
        match settled {
            Some(SettledDocument::Append(text)) => Some(text),
            _ => None,
        }
    }

    fn replace(settled: Option<SettledDocument>) -> Option<String> {
        match settled {
            Some(SettledDocument::Replace(text)) => Some(text),
            _ => None,
        }
    }

    #[test]
    fn streams_the_document_of_a_write_word_by_word() {
        let mut streams = DocumentStreams::default();
        assert_eq!(
            streams.observe_delta(
                0,
                Some("call-1"),
                Some("write_"),
                Some("{\"path\":\"proposal.md\",")
            ),
            None,
        );
        assert_eq!(
            streams.observe_delta(
                0,
                None,
                Some("file"),
                Some("\"content\":\"# Proposal\\n\\n## Recommend")
            ),
            // `## ` is already past a word boundary; `Recommend` is held back
            // so a reader never sees a fragment resolve into another word.
            Some("# Proposal\n\n## ".into()),
        );
        assert_eq!(
            streams.observe_delta(0, None, None, Some("ation\\n\\nStart here.\"}")),
            Some("Recommendation\n\nStart here.".into()),
        );
    }

    #[test]
    fn projects_the_replacement_text_of_an_edit_not_its_search_text() {
        let mut streams = DocumentStreams::default();
        assert_eq!(
            streams.observe_delta(
                0,
                Some("call-1"),
                Some("edit_file"),
                Some("{\"path\":\"a.md\",\"old_string\":\"stale words here\","),
            ),
            None,
        );
        assert_eq!(
            streams.observe_delta(0, None, None, Some("\"new_string\":\"fresh words ")),
            Some("fresh words ".into()),
        );
    }

    #[test]
    fn a_source_file_write_never_streams() {
        // The UI cannot know the target until arguments validate — after the
        // whole payload has streamed — so this gate is the only thing standing
        // between a code file and the document surface.
        let mut streams = DocumentStreams::default();
        assert_eq!(
            streams.observe_delta(
                0,
                Some("call-1"),
                Some("write_file"),
                Some("{\"path\":\"src/main.rs\",\"content\":\"fn main() { "),
            ),
            None,
        );
        assert_eq!(
            streams.observe_delta(0, None, None, Some("println!(\\\"hi\\\"); }\"}")),
            None,
        );
        // Nor at settle: nothing streamed, nothing to reconcile.
        let args = serde_json::json!({"path": "src/main.rs", "content": "fn main() {}"});
        assert!(streams.settle("call-1", "write_file", &args).is_none());
    }

    #[test]
    fn a_half_streamed_path_defers_rather_than_misjudging() {
        let mut streams = DocumentStreams::default();
        // `proposal.m` so far — neither markdown nor not-markdown yet.
        assert_eq!(
            streams.observe_delta(
                0,
                Some("call-1"),
                Some("write_file"),
                Some("{\"path\":\"proposal.m")
            ),
            None,
        );
        assert_eq!(
            streams.observe_delta(0, None, None, Some("d\",\"content\":\"# Proposal ")),
            Some("# Proposal ".into()),
        );
    }

    #[test]
    fn ineligible_tools_never_stream() {
        let mut streams = DocumentStreams::default();
        assert_eq!(
            streams.observe_delta(
                0,
                None,
                Some("computer_type_text"),
                Some("{\"text\":\"hunter2 and more\""),
            ),
            None,
        );
        assert_eq!(
            streams.observe_delta(1, None, Some("read_file"), Some("{\"path\":\"a.md\"}")),
            None,
        );
        // A patch payload is code with no single target to gate on.
        assert_eq!(
            streams.observe_delta(
                2,
                None,
                Some("apply_patch"),
                Some("{\"patch\":\"*** Update File: a.md\\n"),
            ),
            None,
        );
    }

    #[test]
    fn validated_arguments_emit_only_the_unstreamed_suffix() {
        let mut streams = DocumentStreams::default();
        assert_eq!(
            streams.observe_delta(
                0,
                Some("call-1"),
                Some("write_file"),
                Some("{\"path\":\"proposal.md\",\"content\":\"Visible then pend"),
            ),
            Some("Visible then ".into()),
        );
        let args = serde_json::json!({"path": "proposal.md", "content": "Visible then pending"});
        assert_eq!(
            append(streams.settle("call-1", "write_file", &args)),
            Some("pending".into()),
        );
    }

    #[test]
    fn a_stream_that_kept_up_settles_to_nothing_rather_than_an_empty_event() {
        let mut streams = DocumentStreams::default();
        streams.observe_delta(
            0,
            Some("call-1"),
            Some("write_file"),
            Some("{\"path\":\"a.md\",\"content\":\"Whole document here \"}"),
        );
        let args = serde_json::json!({"path": "a.md", "content": "Whole document here "});
        assert!(streams.settle("call-1", "write_file", &args).is_none());
    }

    #[test]
    fn an_untrusted_stream_settles_by_replacement_never_by_appending() {
        let mut streams = DocumentStreams::default();
        assert_eq!(
            streams.observe_delta(
                0,
                Some("call-1"),
                Some("write_file"),
                Some("{\"path\":\"a.md\",\"content\":\"first pass "),
            ),
            Some("first pass ".into()),
        );
        // The provider rewinds its own arguments; the rendered "first pass " is
        // now a lie. The reducer *appends* `append_input`, so reconciling with
        // an append would render "first pass <whole new document>" — a splice
        // of two drafts. Replacement is the only honest patch.
        streams.candidates.get_mut(&0).unwrap().arguments =
            "{\"path\":\"a.md\",\"content\":\"different text".to_string();
        assert_eq!(streams.observe_delta(0, None, None, Some(" more ")), None);
        let args = serde_json::json!({"path": "a.md", "content": "different text more"});
        assert_eq!(
            replace(streams.settle("call-1", "write_file", &args)),
            Some("different text more".into()),
        );
    }

    #[test]
    fn an_ineligible_tool_settles_to_nothing_at_all() {
        let mut streams = DocumentStreams::default();
        assert!(streams
            .settle("call-1", "read_file", &serde_json::json!({"path": "a.md"}))
            .is_none());
    }

    #[test]
    fn a_shared_prefix_does_not_let_one_write_consume_another_stream() {
        let mut streams = DocumentStreams::default();
        // Two writes in one message whose documents open identically — the same
        // heading, which specs routinely do. Name plus prefix cannot tell them
        // apart; only the call id can.
        streams.observe_delta(
            0,
            Some("call-a"),
            Some("write_file"),
            Some("{\"path\":\"a.md\",\"content\":\"## Overview\\n\\nThis is"),
        );
        streams.observe_delta(
            1,
            Some("call-b"),
            Some("write_file"),
            Some("{\"path\":\"b.md\",\"content\":\"## "),
        );

        let second = serde_json::json!({"path": "b.md", "content": "## Overview\n\nThis is the second file."});
        let first =
            serde_json::json!({"path": "a.md", "content": "## Overview\n\nThis is file one."});

        // Call b rendered only "## ", so its suffix must restore everything else.
        assert_eq!(
            append(streams.settle("call-b", "write_file", &second)),
            Some("Overview\n\nThis is the second file.".into()),
        );
        // And call a, which rendered "## Overview\n\nThis ", must not be handed
        // its whole document over again.
        assert_eq!(
            append(streams.settle("call-a", "write_file", &first)),
            Some("is file one.".into()),
        );
    }

    #[test]
    fn two_parallel_writes_settle_against_their_own_streams() {
        let mut streams = DocumentStreams::default();
        streams.observe_delta(
            0,
            Some("call-a"),
            Some("write_file"),
            Some("{\"path\":\"a.md\",\"content\":\"## Alpha section "),
        );
        streams.observe_delta(
            1,
            Some("call-b"),
            Some("write_file"),
            Some("{\"path\":\"b.md\",\"content\":\"## Beta "),
        );

        let alpha = serde_json::json!({"path": "a.md", "content": "## Alpha section one"});
        let beta = serde_json::json!({"path": "b.md", "content": "## Beta two"});

        assert_eq!(
            append(streams.settle("call-a", "write_file", &alpha)),
            Some("one".into()),
        );
        assert_eq!(
            append(streams.settle("call-b", "write_file", &beta)),
            Some("two".into()),
        );
    }
}
