#[derive(Default)]
pub(super) struct ProposedPlanStreamFilter {
    pending: String,
    in_plan: bool,
}

impl ProposedPlanStreamFilter {
    pub(super) fn feed(&mut self, delta: &str) -> String {
        self.pending.push_str(delta);
        self.drain(false)
    }

    pub(super) fn finish(&mut self) -> String {
        self.drain(true)
    }

    fn drain(&mut self, final_chunk: bool) -> String {
        let mut visible = String::new();
        loop {
            if self.in_plan {
                if let Some(end) = self.pending.find("</proposed_plan>") {
                    self.pending = self.pending[end + "</proposed_plan>".len()..].to_string();
                    self.in_plan = false;
                    continue;
                }
                if final_chunk {
                    visible.push_str("<proposed_plan>");
                    visible.push_str(&self.pending);
                    self.pending.clear();
                    self.in_plan = false;
                }
                break;
            }
            if let Some(start) = self.pending.find("<proposed_plan>") {
                visible.push_str(&self.pending[..start]);
                self.pending = self.pending[start + "<proposed_plan>".len()..].to_string();
                self.in_plan = true;
                continue;
            }
            if final_chunk {
                visible.push_str(&self.pending);
                self.pending.clear();
            } else {
                let keep = (1.."<proposed_plan>".len())
                    .rev()
                    .find(|length| self.pending.ends_with(&"<proposed_plan>"[..*length]))
                    .unwrap_or(0);
                let emit_len = self.pending.len().saturating_sub(keep);
                visible.push_str(&self.pending[..emit_len]);
                self.pending = self.pending[emit_len..].to_string();
            }
            break;
        }
        visible
    }
}
