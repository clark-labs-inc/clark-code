use std::mem;

use super::terminal_layout::{display_width, wrap_line};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ComposerViewport {
    pub(crate) text: String,
    pub(crate) cursor_column: u16,
    pub(crate) cursor_row: u16,
    pub(crate) height: u16,
}

#[derive(Debug, Default)]
pub(crate) struct Composer {
    text: String,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    draft_before_history: Option<String>,
    preferred_column: Option<usize>,
}

impl Composer {
    pub(crate) fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    pub(crate) fn insert_char(&mut self, character: char) {
        self.begin_edit();
        self.text.insert(self.cursor, character);
        self.cursor += character.len_utf8();
    }

    pub(crate) fn insert_text(&mut self, text: &str) {
        self.begin_edit();
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        self.text.insert_str(self.cursor, &normalized);
        self.cursor += normalized.len();
    }

    pub(crate) fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    pub(crate) fn replace_text(&mut self, text: String) {
        self.begin_edit();
        self.text = text;
        self.cursor = self.text.len();
    }

    pub(crate) fn backspace(&mut self) -> bool {
        let Some(previous) = self.previous_boundary() else {
            return false;
        };
        self.begin_edit();
        self.text.drain(previous..self.cursor);
        self.cursor = previous;
        true
    }

    pub(crate) fn delete(&mut self) -> bool {
        let Some(next) = self.next_boundary() else {
            return false;
        };
        self.begin_edit();
        self.text.drain(self.cursor..next);
        true
    }

    pub(crate) fn move_left(&mut self) -> bool {
        let Some(previous) = self.previous_boundary() else {
            return false;
        };
        self.cursor = previous;
        self.preferred_column = None;
        true
    }

    pub(crate) fn move_right(&mut self) -> bool {
        let Some(next) = self.next_boundary() else {
            return false;
        };
        self.cursor = next;
        self.preferred_column = None;
        true
    }

    pub(crate) fn move_home(&mut self) {
        self.cursor = self.line_start(self.cursor);
        self.preferred_column = None;
    }

    pub(crate) fn move_end(&mut self) {
        self.cursor = self.line_end(self.cursor);
        self.preferred_column = None;
    }

    pub(crate) fn move_up_or_history(&mut self) -> bool {
        if self.move_vertical(-1) {
            true
        } else {
            self.history_previous()
        }
    }

    pub(crate) fn move_down_or_history(&mut self) -> bool {
        if self.move_vertical(1) {
            true
        } else {
            self.history_next()
        }
    }

    pub(crate) fn submit(&mut self) -> Option<String> {
        if self.is_blank() {
            return None;
        }
        let submitted = mem::take(&mut self.text);
        if self.history.last() != Some(&submitted) {
            self.history.push(submitted.clone());
        }
        self.cursor = 0;
        self.history_index = None;
        self.draft_before_history = None;
        self.preferred_column = None;
        Some(submitted)
    }

    pub(crate) fn slash_query(&self) -> Option<&str> {
        let prefix = self.text.get(..self.cursor)?;
        let query = prefix.strip_prefix('/')?;
        (!query.contains(char::is_whitespace)).then_some(query)
    }

    pub(crate) fn viewport(&self, max_lines: u16, max_columns: u16) -> ComposerViewport {
        let max_lines = usize::from(max_lines.max(1));
        let content_width = usize::from(max_columns.saturating_sub(2).max(1));
        let mut rows = Vec::new();
        let mut cursor_row = 0usize;
        let mut cursor_column = 2usize;
        let mut line_start = 0usize;
        for (source_line, line) in self.text.split('\n').enumerate() {
            let line_end = line_start + line.len();
            let segments = wrap_line(source_line, line, content_width);
            let cursor_in_line = self.cursor >= line_start && self.cursor <= line_end;
            let local_cursor = self.cursor.saturating_sub(line_start);
            for (segment_index, segment) in segments.iter().enumerate() {
                if cursor_in_line
                    && local_cursor >= segment.start_byte
                    && (local_cursor < segment.end_byte
                        || (segment_index + 1 == segments.len()
                            && local_cursor <= segment.end_byte))
                {
                    cursor_row = rows.len();
                    cursor_column = 2 + display_width(&line[segment.start_byte..local_cursor]);
                }
                rows.push(format!(
                    "{}{}",
                    if rows.is_empty() { "> " } else { "  " },
                    segment.text
                ));
            }
            line_start = line_end.saturating_add(1);
        }
        let first_visible = cursor_row.saturating_add(1).saturating_sub(max_lines);
        let last_visible = (first_visible + max_lines).min(rows.len());
        let text = rows[first_visible..last_visible]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        ComposerViewport {
            text,
            cursor_column: u16::try_from(cursor_column)
                .unwrap_or(u16::MAX)
                .min(max_columns.saturating_sub(1)),
            cursor_row: u16::try_from(cursor_row - first_visible).unwrap_or(u16::MAX),
            height: u16::try_from(last_visible - first_visible).unwrap_or(u16::MAX),
        }
    }

    fn begin_edit(&mut self) {
        self.history_index = None;
        self.draft_before_history = None;
        self.preferred_column = None;
    }

    fn previous_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
    }

    fn next_boundary(&self) -> Option<usize> {
        if self.cursor == self.text.len() {
            return None;
        }
        self.text[self.cursor..]
            .chars()
            .next()
            .map(|character| self.cursor + character.len_utf8())
    }

    fn line_start(&self, cursor: usize) -> usize {
        self.text[..cursor].rfind('\n').map_or(0, |index| index + 1)
    }

    fn line_end(&self, cursor: usize) -> usize {
        self.text[cursor..]
            .find('\n')
            .map_or(self.text.len(), |offset| cursor + offset)
    }

    fn move_vertical(&mut self, direction: i8) -> bool {
        let start = self.line_start(self.cursor);
        let desired_column = self
            .preferred_column
            .unwrap_or_else(|| self.text[start..self.cursor].chars().count());
        let (target_start, target_end) = if direction < 0 {
            if start == 0 {
                return false;
            }
            let end = start - 1;
            (self.line_start(end), end)
        } else {
            let end = self.line_end(self.cursor);
            if end == self.text.len() {
                return false;
            }
            let target_start = end + 1;
            (target_start, self.line_end(target_start))
        };
        self.cursor = self.byte_at_column(target_start, target_end, desired_column);
        self.preferred_column = Some(desired_column);
        true
    }

    fn byte_at_column(&self, start: usize, end: usize, column: usize) -> usize {
        self.text[start..end]
            .char_indices()
            .nth(column)
            .map_or(end, |(offset, _)| start + offset)
    }

    fn history_previous(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        let index = match self.history_index {
            Some(0) => return false,
            Some(index) => index - 1,
            None => {
                self.draft_before_history = Some(self.text.clone());
                self.history.len() - 1
            }
        };
        self.history_index = Some(index);
        self.text.clone_from(&self.history[index]);
        self.cursor = self.text.len();
        self.preferred_column = None;
        true
    }

    fn history_next(&mut self) -> bool {
        let Some(index) = self.history_index else {
            return false;
        };
        if index + 1 < self.history.len() {
            self.history_index = Some(index + 1);
            self.text.clone_from(&self.history[index + 1]);
        } else {
            self.history_index = None;
            self.text = self.draft_before_history.take().unwrap_or_default();
        }
        self.cursor = self.text.len();
        self.preferred_column = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_at_unicode_character_boundaries() {
        let mut composer = Composer::default();
        composer.insert_text("a🙂b");
        assert!(composer.move_left());
        assert!(composer.backspace());
        assert_eq!(composer.text, "ab");
        assert!(composer.delete());
        assert_eq!(composer.text, "a");
    }

    #[test]
    fn horizontal_and_line_boundary_motion_edit_in_place() {
        let mut composer = Composer::default();
        composer.insert_text("abc\ndef");
        composer.move_home();
        assert!(composer.move_right());
        composer.insert_char('!');
        composer.move_end();
        composer.insert_char('?');
        assert_eq!(composer.text, "abc\nd!ef?");
    }

    #[test]
    fn multiline_paste_is_preserved_on_submit() {
        let mut composer = Composer::default();
        composer.insert_text("first\r\nsecond\rthird");
        composer.insert_newline();
        composer.insert_text("fourth");
        assert_eq!(
            composer.submit().as_deref(),
            Some("first\nsecond\nthird\nfourth")
        );
        assert!(composer.is_blank());
    }

    #[test]
    fn history_navigation_restores_the_unsent_draft() {
        let mut composer = Composer::default();
        composer.insert_text("one");
        assert_eq!(composer.submit().as_deref(), Some("one"));
        composer.insert_text("two");
        assert_eq!(composer.submit().as_deref(), Some("two"));
        composer.insert_text("draft");
        assert!(composer.move_up_or_history());
        assert_eq!(composer.text, "two");
        assert!(composer.move_up_or_history());
        assert_eq!(composer.text, "one");
        assert!(composer.move_down_or_history());
        assert_eq!(composer.text, "two");
        assert!(composer.move_down_or_history());
        assert_eq!(composer.text, "draft");
    }

    #[test]
    fn vertical_cursor_motion_retains_the_preferred_column() {
        let mut composer = Composer::default();
        composer.insert_text("12345\nx\nabcde");
        assert!(composer.move_left());
        assert!(composer.move_up_or_history());
        assert!(composer.move_up_or_history());
        composer.insert_char('!');
        assert_eq!(composer.text, "1234!5\nx\nabcde");
    }

    #[test]
    fn viewport_tracks_cursor_and_slash_query() {
        let mut composer = Composer::default();
        composer.insert_text("/sta");
        assert_eq!(composer.slash_query(), Some("sta"));
        composer.insert_text("\none\ntwo\nthree");
        assert_eq!(composer.slash_query(), None);
        let viewport = composer.viewport(2, 80);
        assert_eq!(viewport.text, "  two\n  three");
        assert_eq!(viewport.cursor_row, 1);
        assert_eq!(viewport.cursor_column, 7);
        assert_eq!(viewport.height, 2);
    }

    #[test]
    fn viewport_wraps_wide_characters_and_tracks_terminal_cursor_cells() {
        let mut composer = Composer::default();
        composer.insert_text("a界🙂z");
        let viewport = composer.viewport(4, 6);
        assert_eq!(viewport.text, "> a界\n  🙂z");
        assert_eq!(viewport.cursor_row, 1);
        assert_eq!(viewport.cursor_column, 5);
    }
}
