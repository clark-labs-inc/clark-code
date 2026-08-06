use unicode_width::UnicodeWidthChar;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WrappedLine {
    pub(crate) source_line: usize,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) text: String,
    pub(crate) columns: usize,
}

pub(crate) fn display_width(text: &str) -> usize {
    text.chars()
        .fold((0usize, 0usize), |(total, column), character| {
            let width = character_width(character, column);
            (total.saturating_add(width), column.saturating_add(width))
        })
        .0
}

pub(crate) fn wrap_line(source_line: usize, text: &str, width: usize) -> Vec<WrappedLine> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![WrappedLine {
            source_line,
            start_byte: 0,
            end_byte: 0,
            text: String::new(),
            columns: 0,
        }];
    }
    let mut wrapped = Vec::new();
    let mut start = 0usize;
    let mut rendered = String::new();
    let mut columns = 0usize;
    for (index, character) in text.char_indices() {
        let character_columns = character_width(character, columns);
        if character_columns > 0 && columns > 0 && columns + character_columns > width {
            wrapped.push(WrappedLine {
                source_line,
                start_byte: start,
                end_byte: index,
                text: std::mem::take(&mut rendered),
                columns,
            });
            start = index;
            columns = 0;
        }
        if character == '\t' {
            let spaces = character_width(character, columns);
            rendered.extend(std::iter::repeat_n(' ', spaces));
            columns += spaces;
        } else if !character.is_control() {
            rendered.push(character);
            columns += character_width(character, columns);
        }
    }
    wrapped.push(WrappedLine {
        source_line,
        start_byte: start,
        end_byte: text.len(),
        text: rendered,
        columns,
    });
    wrapped
}

fn character_width(character: char, column: usize) -> usize {
    if character == '\t' {
        return 4 - (column % 4);
    }
    UnicodeWidthChar::width(character).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_and_combining_characters_wrap_by_terminal_columns() {
        let lines = wrap_line(7, "A界e\u{301}🙂Z", 4);
        assert_eq!(
            lines
                .iter()
                .map(|line| (line.source_line, line.text.as_str(), line.columns))
                .collect::<Vec<_>>(),
            vec![(7, "A界e\u{301}", 4), (7, "🙂Z", 3)]
        );
        assert_eq!(display_width("A界e\u{301}🙂Z"), 7);
    }

    #[test]
    fn tabs_expand_to_stable_stops_without_changing_source_offsets() {
        let lines = wrap_line(2, "a\t界", 4);
        assert_eq!(lines[0].text, "a   ");
        assert_eq!((lines[0].start_byte, lines[0].end_byte), (0, 2));
        assert_eq!(lines[1].text, "界");
    }
}
