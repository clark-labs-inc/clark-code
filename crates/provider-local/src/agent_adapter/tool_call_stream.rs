//! Provider tool-call delta routing.
//!
//! Ordinary tools are safe to expose as activity while their arguments are
//! generated. `final_answer` is different: its arguments contain user-facing
//! prose and may still be rejected by the effect-completion gate. Buffer only
//! enough of a fragmented name to distinguish that terminal tool.

use std::collections::BTreeMap;

use crate::llm::WireToolCallDelta;
use crate::tools::final_answer::FINAL_ANSWER_TOOL;

#[derive(Default)]
struct Candidate {
    name: String,
    buffered: Vec<WireToolCallDelta>,
    ordinary: bool,
    suppressed: bool,
}

pub(super) struct ToolCallStreamGate {
    stream_terminal_tool: bool,
    candidates: BTreeMap<usize, Candidate>,
}

#[derive(Default)]
pub(super) struct VisibleToolCallDeltas {
    pub(super) deltas: Vec<WireToolCallDelta>,
    pub(super) ordinary_started: bool,
}

impl ToolCallStreamGate {
    pub(super) fn new(stream_terminal_tool: bool) -> Self {
        Self {
            stream_terminal_tool,
            candidates: BTreeMap::new(),
        }
    }

    pub(super) fn observe(&mut self, delta: WireToolCallDelta) -> VisibleToolCallDeltas {
        let candidate = self.candidates.entry(delta.index).or_default();
        if candidate.suppressed {
            return VisibleToolCallDeltas::default();
        }
        if candidate.ordinary {
            return VisibleToolCallDeltas {
                deltas: vec![delta],
                ordinary_started: false,
            };
        }

        if let Some(name) = &delta.name_delta {
            candidate.name.push_str(name);
        }
        candidate.buffered.push(delta);

        if candidate.name == FINAL_ANSWER_TOOL {
            if self.stream_terminal_tool {
                return VisibleToolCallDeltas {
                    deltas: std::mem::take(&mut candidate.buffered),
                    ordinary_started: false,
                };
            }
            candidate.suppressed = true;
            candidate.buffered.clear();
            return VisibleToolCallDeltas::default();
        }
        if FINAL_ANSWER_TOOL.starts_with(&candidate.name) {
            return if self.stream_terminal_tool {
                VisibleToolCallDeltas {
                    deltas: std::mem::take(&mut candidate.buffered),
                    ordinary_started: false,
                }
            } else {
                VisibleToolCallDeltas::default()
            };
        }

        candidate.ordinary = true;
        VisibleToolCallDeltas {
            deltas: std::mem::take(&mut candidate.buffered),
            ordinary_started: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(index: usize, name: Option<&str>, arguments: Option<&str>) -> WireToolCallDelta {
        WireToolCallDelta {
            index,
            id_delta: (name.is_some()).then(|| format!("call-{index}")),
            name_delta: name.map(str::to_string),
            arguments_delta: arguments.map(str::to_string),
        }
    }

    #[test]
    fn ordinary_tool_flushes_after_fragmented_name_is_distinguishable() {
        let mut gate = ToolCallStreamGate::new(false);
        assert!(gate.observe(delta(0, Some("fi"), None)).deltas.is_empty());
        let flushed = gate.observe(delta(0, Some("le_read"), Some("{\"path\":")));
        assert!(flushed.ordinary_started);
        assert_eq!(flushed.deltas.len(), 2);
        assert_eq!(flushed.deltas[0].name_delta.as_deref(), Some("fi"));
        assert_eq!(flushed.deltas[1].name_delta.as_deref(), Some("le_read"));
        assert_eq!(
            gate.observe(delta(0, None, Some("\"README.md\"}")))
                .deltas
                .len(),
            1
        );
    }

    #[test]
    fn rejected_terminal_tool_arguments_stay_staged() {
        let mut gate = ToolCallStreamGate::new(false);
        assert!(gate
            .observe(delta(0, Some("final_"), None))
            .deltas
            .is_empty());
        assert!(gate
            .observe(delta(0, Some("answer"), Some("{\"content\":\"secret")))
            .deltas
            .is_empty());
        assert!(gate
            .observe(delta(0, None, Some(" answer\"}")))
            .deltas
            .is_empty());
    }

    #[test]
    fn approved_terminal_tool_streams_without_buffering() {
        let mut gate = ToolCallStreamGate::new(true);
        let input = delta(0, Some(FINAL_ANSWER_TOOL), Some("{\"content\":\"hello"));
        let visible = gate.observe(input.clone());
        assert!(!visible.ordinary_started);
        assert_eq!(visible.deltas, vec![input]);
    }
}
