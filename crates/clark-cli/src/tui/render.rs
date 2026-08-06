#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TranscriptKind {
    User,
    Clark,
    System,
    Tool,
    Artifact,
    Diff,
    Error,
}

impl TranscriptKind {
    fn label(self) -> &'static str {
        match self {
            Self::User => "You",
            Self::Clark => "Clark",
            Self::System => "System",
            Self::Tool => "Tool",
            Self::Artifact => "Artifact",
            Self::Diff => "Diff",
            Self::Error => "Error",
        }
    }

    fn label_style(self) -> RenderStyle {
        match self {
            Self::User => RenderStyle::UserLabel,
            Self::Clark => RenderStyle::ClarkLabel,
            Self::System => RenderStyle::SystemLabel,
            Self::Tool => RenderStyle::Tool,
            Self::Artifact => RenderStyle::Artifact,
            Self::Diff => RenderStyle::DiffHeader,
            Self::Error => RenderStyle::Error,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RenderStyle {
    UserLabel,
    ClarkLabel,
    SystemLabel,
    Body,
    Tool,
    Artifact,
    DiffHeader,
    DiffHunk,
    DiffAdd,
    DiffRemove,
    Heading,
    Code,
    Error,
    Spacer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RenderLine {
    pub(crate) text: String,
    pub(crate) style: RenderStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TranscriptEntry {
    kind: TranscriptKind,
    text: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Transcript {
    entries: Vec<TranscriptEntry>,
}

impl Transcript {
    pub(crate) fn with_system(text: impl Into<String>) -> Self {
        let mut transcript = Self::default();
        transcript.push(TranscriptKind::System, text);
        transcript
    }

    pub(crate) fn push(&mut self, kind: TranscriptKind, text: impl Into<String>) {
        self.entries.push(TranscriptEntry {
            kind,
            text: text.into(),
        });
    }

    pub(crate) fn append(&mut self, kind: TranscriptKind, text: &str) {
        if let Some(last) = self.entries.last_mut() {
            if last.kind == kind {
                last.text.push_str(text);
                return;
            }
        }
        self.push(kind, text);
    }

    pub(crate) fn clear_with_notice(&mut self, notice: impl Into<String>) {
        self.entries.clear();
        self.push(TranscriptKind::System, notice);
    }

    pub(crate) fn last_text(&self, kind: TranscriptKind) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.kind == kind)
            .map(|entry| entry.text.as_str())
    }

    pub(crate) fn render(&self, raw: bool) -> Vec<RenderLine> {
        let mut lines = Vec::new();
        for entry in &self.entries {
            if !raw {
                lines.push(RenderLine {
                    text: entry.kind.label().into(),
                    style: entry.kind.label_style(),
                });
            }
            let mut in_code = false;
            for text in entry.text.split('\n') {
                let fence = text.trim_start().starts_with("```");
                lines.push(RenderLine {
                    text: text.into(),
                    style: content_style(entry.kind, text, raw, in_code || fence),
                });
                if fence {
                    in_code = !in_code;
                }
            }
            lines.push(RenderLine {
                text: String::new(),
                style: RenderStyle::Spacer,
            });
        }
        lines
    }
}

fn content_style(kind: TranscriptKind, text: &str, raw: bool, in_code: bool) -> RenderStyle {
    if raw {
        return RenderStyle::Body;
    }
    match kind {
        TranscriptKind::Tool => RenderStyle::Tool,
        TranscriptKind::Artifact => RenderStyle::Artifact,
        TranscriptKind::Error => RenderStyle::Error,
        TranscriptKind::Diff if text.starts_with("@@") => RenderStyle::DiffHunk,
        TranscriptKind::Diff if text.starts_with('+') && !text.starts_with("+++") => {
            RenderStyle::DiffAdd
        }
        TranscriptKind::Diff if text.starts_with('-') && !text.starts_with("---") => {
            RenderStyle::DiffRemove
        }
        TranscriptKind::Diff => RenderStyle::Body,
        _ if in_code => RenderStyle::Code,
        _ if text.trim_start().starts_with('#') => RenderStyle::Heading,
        _ => RenderStyle::Body,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Selection {
    anchor: usize,
    focus: usize,
}

#[derive(Debug, Default)]
pub(crate) struct TranscriptViewport {
    offset_from_bottom: usize,
    selection: Option<Selection>,
}

impl TranscriptViewport {
    pub(crate) fn follow_bottom(&mut self) {
        self.offset_from_bottom = 0;
    }

    pub(crate) fn scroll_up(&mut self, amount: usize) {
        self.offset_from_bottom = self.offset_from_bottom.saturating_add(amount);
    }

    pub(crate) fn scroll_down(&mut self, amount: usize) {
        self.offset_from_bottom = self.offset_from_bottom.saturating_sub(amount);
    }

    pub(crate) fn visible_range(&self, total: usize, height: usize) -> (usize, usize) {
        if height == 0 || total == 0 {
            return (0, 0);
        }
        let max_offset = total.saturating_sub(height);
        let offset = self.offset_from_bottom.min(max_offset);
        let end = total.saturating_sub(offset);
        (end.saturating_sub(height), end)
    }

    #[cfg(test)]
    #[cfg(test)]
    pub(crate) fn select_visible_row(
        &mut self,
        row: usize,
        total: usize,
        height: usize,
        extend: bool,
    ) -> bool {
        let (start, end) = self.visible_range(total, height);
        let index = start.saturating_add(row);
        if index >= end {
            return false;
        }
        if extend {
            match &mut self.selection {
                Some(selection) => selection.focus = index,
                None => {
                    self.selection = Some(Selection {
                        anchor: index,
                        focus: index,
                    });
                }
            }
        } else {
            self.selection = Some(Selection {
                anchor: index,
                focus: index,
            });
        }
        true
    }

    pub(crate) fn select_source(&mut self, source: usize, total: usize, extend: bool) -> bool {
        if source >= total {
            return false;
        }
        if extend {
            match &mut self.selection {
                Some(selection) => selection.focus = source,
                None => {
                    self.selection = Some(Selection {
                        anchor: source,
                        focus: source,
                    });
                }
            }
        } else {
            self.selection = Some(Selection {
                anchor: source,
                focus: source,
            });
        }
        true
    }

    pub(crate) fn extend_selection_from(
        &mut self,
        direction: i8,
        total: usize,
        default_source: usize,
    ) -> bool {
        if total == 0 || default_source >= total {
            return false;
        }
        let selection = self.selection.get_or_insert(Selection {
            anchor: default_source,
            focus: default_source,
        });
        let next = if direction < 0 {
            selection.focus.saturating_sub(1)
        } else {
            selection.focus.saturating_add(1).min(total - 1)
        };
        if next == selection.focus {
            return false;
        }
        selection.focus = next;
        if direction < 0 {
            self.scroll_up(1);
        } else {
            self.scroll_down(1);
        }
        true
    }

    #[cfg(test)]
    #[cfg(test)]
    pub(crate) fn extend_selection(&mut self, direction: i8, total: usize, height: usize) -> bool {
        if total == 0 {
            return false;
        }
        let (visible_start, visible_end) = self.visible_range(total, height);
        let selection = self.selection.get_or_insert_with(|| {
            let index = visible_end.saturating_sub(1);
            Selection {
                anchor: index,
                focus: index,
            }
        });
        let next = if direction < 0 {
            selection.focus.saturating_sub(1)
        } else {
            selection.focus.saturating_add(1).min(total - 1)
        };
        if next == selection.focus {
            return false;
        }
        selection.focus = next;
        if next < visible_start {
            self.scroll_up(1);
        } else if next >= visible_end {
            self.scroll_down(1);
        }
        true
    }

    pub(crate) fn is_selected(&self, index: usize) -> bool {
        self.selection.is_some_and(|selection| {
            let start = selection.anchor.min(selection.focus);
            let end = selection.anchor.max(selection.focus);
            (start..=end).contains(&index)
        })
    }

    pub(crate) fn selected_text(&self, lines: &[RenderLine]) -> Option<String> {
        let selection = self.selection?;
        let start = selection.anchor.min(selection.focus);
        let end = selection
            .anchor
            .max(selection.focus)
            .min(lines.len().saturating_sub(1));
        (start < lines.len()).then(|| {
            lines[start..=end]
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    pub(crate) fn clear_selection(&mut self) {
        self.selection = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_entries_have_stable_distinct_render_roles() {
        let mut transcript = Transcript::default();
        transcript.push(TranscriptKind::User, "question");
        transcript.append(
            TranscriptKind::Clark,
            "# Answer\n```rust\nlet value = 1;\n``` ",
        );
        transcript.append(TranscriptKind::Clark, "continued");
        transcript.push(TranscriptKind::Tool, "cargo test");
        transcript.push(TranscriptKind::Artifact, "report.pdf");
        transcript.push(TranscriptKind::Error, "failed");
        let lines = transcript.render(false);
        assert!(lines
            .iter()
            .any(|line| line.style == RenderStyle::UserLabel));
        assert!(lines
            .iter()
            .any(|line| line.style == RenderStyle::ClarkLabel));
        assert!(lines.iter().any(|line| line.style == RenderStyle::Tool));
        assert!(lines.iter().any(|line| line.style == RenderStyle::Artifact));
        assert!(lines.iter().any(|line| line.style == RenderStyle::Error));
        assert!(lines.iter().any(|line| line.style == RenderStyle::Heading));
        assert!(lines.iter().any(|line| line.style == RenderStyle::Code));
        assert_eq!(
            transcript.last_text(TranscriptKind::Clark),
            Some("# Answer\n```rust\nlet value = 1;\n``` continued")
        );
    }

    #[test]
    fn diff_lines_are_classified_without_changing_content() {
        let mut transcript = Transcript::default();
        transcript.push(TranscriptKind::Diff, "@@ -1 +1 @@\n-old\n+new\n context");
        let lines = transcript.render(false);
        assert!(lines.iter().any(|line| line.style == RenderStyle::DiffHunk));
        assert!(lines
            .iter()
            .any(|line| line.style == RenderStyle::DiffRemove));
        assert!(lines.iter().any(|line| line.style == RenderStyle::DiffAdd));
        assert!(lines.iter().any(|line| line.text == " context"));
    }

    #[test]
    fn viewport_is_bottom_anchored_and_scrolls_without_overflow() {
        let lines = (0..10)
            .map(|index| RenderLine {
                text: index.to_string(),
                style: RenderStyle::Body,
            })
            .collect::<Vec<_>>();
        let mut viewport = TranscriptViewport::default();
        assert_eq!(visible(&viewport, &lines, 3)[0].text, "7");
        viewport.scroll_up(4);
        assert_eq!(visible(&viewport, &lines, 3)[0].text, "3");
        viewport.scroll_up(usize::MAX);
        assert_eq!(visible(&viewport, &lines, 3)[0].text, "0");
        viewport.scroll_down(usize::MAX);
        assert_eq!(visible(&viewport, &lines, 3)[0].text, "7");
        viewport.scroll_up(2);
        viewport.follow_bottom();
        assert_eq!(visible(&viewport, &lines, 3)[0].text, "7");
    }

    #[test]
    fn raw_rendering_omits_labels_but_preserves_records() {
        let mut transcript = Transcript::with_system("connected");
        transcript.push(TranscriptKind::Clark, "hello");
        let lines = transcript.render(true);
        assert!(!lines.iter().any(|line| line.text == "System"));
        assert!(!lines.iter().any(|line| line.text == "Clark"));
        assert!(lines.iter().any(|line| line.text == "connected"));
        assert!(lines.iter().any(|line| line.text == "hello"));
        transcript.clear_with_notice("cleared");
        assert_eq!(
            transcript.last_text(TranscriptKind::System),
            Some("cleared")
        );
    }

    #[test]
    fn keyboard_and_mouse_selection_copy_exact_lines() {
        let lines = (0..8)
            .map(|index| RenderLine {
                text: format!("line {index}"),
                style: RenderStyle::Body,
            })
            .collect::<Vec<_>>();
        let mut viewport = TranscriptViewport::default();
        assert!(viewport.select_visible_row(1, lines.len(), 4, false));
        assert!(viewport.select_visible_row(3, lines.len(), 4, true));
        assert!(viewport.is_selected(5));
        assert!(viewport.is_selected(7));
        assert!(!viewport.is_selected(4));
        assert_eq!(
            viewport.selected_text(&lines).as_deref(),
            Some("line 5\nline 6\nline 7")
        );
        assert!(viewport.extend_selection(-1, lines.len(), 4));
        assert_eq!(
            viewport.selected_text(&lines).as_deref(),
            Some("line 5\nline 6")
        );
        viewport.clear_selection();
        assert_eq!(viewport.selected_text(&lines), None);
    }

    fn visible<'a>(
        viewport: &TranscriptViewport,
        lines: &'a [RenderLine],
        height: usize,
    ) -> &'a [RenderLine] {
        let (start, end) = viewport.visible_range(lines.len(), height);
        &lines[start..end]
    }
}
