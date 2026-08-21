//! Holds required-tool prose only until a streamed ordinary tool proves the
//! response honored the structured boundary. Contract-violating prose and
//! terminal `final_answer` companion text remain private.

#[derive(Default)]
pub(super) struct RequiredToolText {
    required: bool,
    released: bool,
    buffered: String,
}

impl RequiredToolText {
    pub(super) fn new(required: bool) -> Self {
        Self {
            required,
            ..Self::default()
        }
    }

    pub(super) fn observe(&mut self, delta: &str) -> Option<String> {
        if !self.required || self.released {
            return Some(delta.to_string());
        }
        self.buffered.push_str(delta);
        None
    }

    pub(super) fn release_for_ordinary_tool(&mut self) -> Option<String> {
        if !self.required || self.released {
            return None;
        }
        self.released = true;
        let buffered = std::mem::take(&mut self.buffered);
        (!buffered.is_empty()).then_some(buffered)
    }

    pub(super) fn finish_ordinary_turn(&mut self, complete_text: &str) -> Option<String> {
        if !self.required || self.released || complete_text.is_empty() {
            return None;
        }
        self.released = true;
        self.buffered.clear();
        Some(complete_text.to_string())
    }

    pub(super) fn reset_attempt(&mut self) {
        self.released = false;
        self.buffered.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_text_releases_once_an_ordinary_tool_is_known() {
        let mut text = RequiredToolText::new(true);
        assert_eq!(text.observe("I will "), None);
        assert_eq!(text.release_for_ordinary_tool().as_deref(), Some("I will "));
        assert_eq!(text.observe("continue."), Some("continue.".into()));
        assert_eq!(text.finish_ordinary_turn("I will continue."), None);
    }

    #[test]
    fn required_text_stays_private_without_an_ordinary_tool() {
        let mut text = RequiredToolText::new(true);
        assert_eq!(text.observe("discard me"), None);
        assert_eq!(text.finish_ordinary_turn(""), None);
    }
}
