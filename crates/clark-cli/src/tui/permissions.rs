use agent_core::{PermissionOptionKind, PermissionRequest};

#[derive(Clone, Debug, PartialEq, Eq)]
struct PermissionChoice {
    id: String,
    label: String,
    kind: PermissionOptionKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PermissionRow {
    pub(crate) label: String,
    pub(crate) consequence: &'static str,
    pub(crate) selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PermissionPicker {
    pub(crate) title: String,
    pub(crate) detail: Option<String>,
    pub(crate) risk: Option<String>,
    pub(crate) reason: Option<String>,
    choices: Vec<PermissionChoice>,
    selected: usize,
}

impl PermissionPicker {
    pub(crate) fn from_request(request: &PermissionRequest) -> Result<Self, String> {
        if request.options.is_empty() {
            return Err("Clark returned a permission request with no choices".into());
        }
        Ok(Self {
            title: request.title.clone(),
            detail: request.detail.clone(),
            risk: request.risk.clone(),
            reason: request.reason.clone(),
            choices: request
                .options
                .iter()
                .map(|option| PermissionChoice {
                    id: option.id.clone(),
                    label: option.label.clone(),
                    kind: option.kind,
                })
                .collect(),
            selected: 0,
        })
    }

    pub(crate) fn select_previous(&mut self) -> bool {
        if self.selected == 0 {
            return false;
        }
        self.selected -= 1;
        true
    }

    pub(crate) fn select_next(&mut self) -> bool {
        if self.selected + 1 >= self.choices.len() {
            return false;
        }
        self.selected += 1;
        true
    }

    pub(crate) fn selected_id(&self) -> String {
        self.choices[self.selected].id.clone()
    }

    pub(crate) fn allow_once_id(&self) -> Option<String> {
        self.id_for_kind(PermissionOptionKind::AllowOnce)
            .or_else(|| self.id_for_kind(PermissionOptionKind::AllowAlways))
    }

    pub(crate) fn reject_once_id(&self) -> Option<String> {
        self.id_for_kind(PermissionOptionKind::RejectOnce)
            .or_else(|| self.id_for_kind(PermissionOptionKind::RejectAlways))
    }

    pub(crate) fn rows(&self) -> Vec<PermissionRow> {
        self.choices
            .iter()
            .enumerate()
            .map(|(index, choice)| PermissionRow {
                label: choice.label.clone(),
                consequence: consequence(choice.kind),
                selected: index == self.selected,
            })
            .collect()
    }

    pub(crate) fn desired_height(&self) -> u16 {
        let context_lines = usize::from(self.detail.is_some())
            + usize::from(self.risk.is_some())
            + usize::from(self.reason.is_some());
        u16::try_from(self.choices.len() + context_lines + 2)
            .unwrap_or(12)
            .min(12)
    }

    fn id_for_kind(&self, kind: PermissionOptionKind) -> Option<String> {
        self.choices
            .iter()
            .find(|choice| choice.kind == kind)
            .map(|choice| choice.id.clone())
    }
}

fn consequence(kind: PermissionOptionKind) -> &'static str {
    match kind {
        PermissionOptionKind::AllowOnce => "allow this action once",
        PermissionOptionKind::AllowAlways => "allow matching actions for this session",
        PermissionOptionKind::RejectOnce => "deny this action once",
        PermissionOptionKind::RejectAlways => "deny matching actions for this session",
    }
}

#[cfg(test)]
mod tests {
    use agent_core::{PermissionOption, PermissionRequestId, SessionId};

    use super::*;

    fn request(options: Vec<PermissionOption>) -> PermissionRequest {
        PermissionRequest {
            id: PermissionRequestId::new("permission-1"),
            session: SessionId::new("session-1"),
            tool_call: None,
            title: "Run destructive command?".into(),
            options,
            detail: Some("rm output.tmp".into()),
            risk: Some("danger".into()),
            reason: Some("deletes a file".into()),
        }
    }

    fn option(id: &str, label: &str, kind: PermissionOptionKind) -> PermissionOption {
        PermissionOption {
            id: id.into(),
            label: label.into(),
            kind,
        }
    }

    #[test]
    fn picker_preserves_exact_context_and_all_provider_choices() {
        let picker = PermissionPicker::from_request(&request(vec![
            option("allow", "Allow once", PermissionOptionKind::AllowOnce),
            option("always", "Always allow", PermissionOptionKind::AllowAlways),
            option("deny", "Deny", PermissionOptionKind::RejectOnce),
        ]))
        .expect("valid picker");
        assert_eq!(picker.title, "Run destructive command?");
        assert_eq!(picker.detail.as_deref(), Some("rm output.tmp"));
        assert_eq!(picker.risk.as_deref(), Some("danger"));
        assert_eq!(picker.reason.as_deref(), Some("deletes a file"));
        assert_eq!(picker.rows().len(), 3);
        assert_eq!(picker.selected_id(), "allow");
    }

    #[test]
    fn keyboard_selection_stops_at_real_choice_boundaries() {
        let mut picker = PermissionPicker::from_request(&request(vec![
            option("allow", "Allow", PermissionOptionKind::AllowOnce),
            option("deny", "Deny", PermissionOptionKind::RejectOnce),
        ]))
        .expect("valid picker");
        assert!(!picker.select_previous());
        assert!(picker.select_next());
        assert_eq!(picker.selected_id(), "deny");
        assert!(!picker.select_next());
        assert!(picker.select_previous());
        assert_eq!(picker.selected_id(), "allow");
    }

    #[test]
    fn shortcuts_choose_once_before_session_wide_options() {
        let picker = PermissionPicker::from_request(&request(vec![
            option("always", "Always allow", PermissionOptionKind::AllowAlways),
            option("allow", "Allow once", PermissionOptionKind::AllowOnce),
            option("never", "Always deny", PermissionOptionKind::RejectAlways),
            option("deny", "Deny once", PermissionOptionKind::RejectOnce),
        ]))
        .expect("valid picker");
        assert_eq!(picker.allow_once_id().as_deref(), Some("allow"));
        assert_eq!(picker.reject_once_id().as_deref(), Some("deny"));
        assert!(picker
            .rows()
            .iter()
            .any(|row| row.consequence.contains("session")));
    }

    #[test]
    fn request_without_choices_fails_closed() {
        assert_eq!(
            PermissionPicker::from_request(&request(Vec::new())).unwrap_err(),
            "Clark returned a permission request with no choices"
        );
    }
}
