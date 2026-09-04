//! Incremental decoding of a single JSON string field out of a tool-call
//! argument stream.
//!
//! OpenAI-compatible providers stream tool arguments as JSON fragments, so a
//! payload that matters to the UI — a final answer, a document being written —
//! is only readable by decoding the prefix that has arrived so far. Shared by
//! `final_answer_stream` and `document_stream`.

/// Decode the currently complete prefix of a JSON string field. The input may
/// end anywhere, including inside an escape or UTF-16 surrogate pair.
pub(super) fn partial_json_string_field(input: &str, field: &str) -> Option<(String, bool)> {
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
    fn reports_the_complete_prefix_and_whether_the_field_closed() {
        assert_eq!(
            partial_json_string_field("{\"content\":\"Half of a sen", "content"),
            Some(("Half of a sen".into(), false)),
        );
        assert_eq!(
            partial_json_string_field("{\"content\":\"All of it.\"}", "content"),
            Some(("All of it.".into(), true)),
        );
    }

    #[test]
    fn stops_at_the_last_safe_boundary_inside_an_escape() {
        // A split escape must never decode into a replacement character. A
        // dangling backslash yields nothing at all rather than the safe prefix:
        // the caller re-reads the whole accumulated buffer on the next delta, so
        // one skipped tick costs nothing and keeps the escape table simple.
        assert_eq!(
            partial_json_string_field("{\"content\":\"line\\", "content"),
            None,
        );
        assert_eq!(
            partial_json_string_field("{\"content\":\"go \\uD83D", "content"),
            Some(("go ".into(), false)),
        );
        assert_eq!(
            partial_json_string_field("{\"content\":\"go \\uD83D\\uDE80", "content"),
            Some(("go \u{1F680}".into(), false)),
        );
    }

    #[test]
    fn decodes_any_named_field_and_skips_what_precedes_it() {
        assert_eq!(
            partial_json_string_field(
                "{\"path\":\"proposal.md\",\"new_string\":\"## Recommend",
                "new_string",
            ),
            Some(("## Recommend".into(), false)),
        );
    }

    #[test]
    fn absent_field_yields_nothing_rather_than_an_empty_string() {
        assert_eq!(
            partial_json_string_field("{\"path\":\"a.md\"", "content"),
            None
        );
        assert_eq!(partial_json_string_field("", "content"), None);
    }
}
