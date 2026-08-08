use std::time::Duration;

use crate::{
    assess_proposed_action, ActionAuthorization, ActionDisposition, ActionReceipt,
    ApplicationIdentity, ComputerAction, ComputerUseError, PrepareActionRequest, PreparedAction,
    ReceiptOutcome, RedactedActionPreview,
};

use super::{now_ms, SimPreparedRecord, SimulatedComputerBackend, HEIGHT, WIDTH};

const PREPARED_TTL_MS: u64 = 60_000;

impl SimulatedComputerBackend {
    pub(super) fn prepare_impl(
        &self,
        request: PrepareActionRequest,
    ) -> Result<PreparedAction, ComputerUseError> {
        Self::validate_window(&request.window)?;
        let mut state = self.state.lock().expect("simulator lock");
        Self::require_latest(&state, &request.observation_id)?;
        validate_points(&request.action)?;
        let application = application_identity();
        let approved = state
            .approved_identities
            .contains(&application.identity_key);
        let assessment = assess_proposed_action(
            &Self::window(),
            &application,
            &Self::elements(&state),
            &request.intent,
            &request.action,
            request.dry_run,
            approved,
        )?;
        let id = format!("sim-prepared-{}", state.prepared_sequence);
        state.prepared_sequence += 1;
        let public = PreparedAction {
            id: id.clone(),
            window: request.window.clone(),
            application,
            kind: request.action.kind(),
            assessment,
            preview: preview(&request.action),
            approval_revision: state.approval_revision,
            expires_at_ms: now_ms().saturating_add(PREPARED_TTL_MS),
            dry_run: request.dry_run,
        };
        state.latest_observation_id = None;
        state.prepared.insert(
            id,
            SimPreparedRecord {
                public: public.clone(),
                request,
                authorization: None,
            },
        );
        Ok(public)
    }

    pub(super) fn prepared_impl(&self, id: &str) -> Result<PreparedAction, ComputerUseError> {
        let state = self.state.lock().expect("simulator lock");
        let prepared = state
            .prepared
            .get(id)
            .ok_or_else(|| ComputerUseError::PreparedActionNotFound(id.to_string()))?;
        if prepared.public.expires_at_ms < now_ms() {
            return Err(ComputerUseError::PreparedActionExpired);
        }
        Ok(prepared.public.clone())
    }

    pub(super) fn authorize_impl(
        &self,
        id: &str,
        authorization: ActionAuthorization,
    ) -> Result<(), ComputerUseError> {
        let mut state = self.state.lock().expect("simulator lock");
        if authorization == ActionAuthorization::Denied {
            state.prepared.remove(id);
            return Ok(());
        }
        let (disposition, identity, app_name) = {
            let prepared = state
                .prepared
                .get(id)
                .ok_or_else(|| ComputerUseError::PreparedActionNotFound(id.to_string()))?;
            (
                prepared.public.assessment.disposition,
                prepared.public.application.identity_key.clone(),
                prepared.public.preview.app_name.clone(),
            )
        };
        match disposition {
            ActionDisposition::Deny => {
                return Err(ComputerUseError::ActionDenied(
                    "the prepared action is denied".to_string(),
                ))
            }
            ActionDisposition::MandatoryHandoff => {
                return Err(ComputerUseError::HumanHandoffRequired(
                    "the prepared action cannot be delegated".to_string(),
                ))
            }
            ActionDisposition::ActionTimeConfirmation
            | ActionDisposition::PreapprovalEligible
            | ActionDisposition::Allow => {}
        }
        if authorization == ActionAuthorization::Durable {
            if disposition != ActionDisposition::PreapprovalEligible {
                return Err(ComputerUseError::ApprovalRequired);
            }
            let _ = app_name;
            state.approved_identities.insert(identity);
            state.approval_revision = state.approval_revision.saturating_add(1);
            let revision = state.approval_revision;
            if let Some(prepared) = state.prepared.get_mut(id) {
                prepared.public.approval_revision = revision;
            }
        }
        state
            .prepared
            .get_mut(id)
            .expect("prepared action checked above")
            .authorization = Some(authorization);
        Ok(())
    }

    pub(super) fn commit_impl(&self, id: &str) -> Result<ActionReceipt, ComputerUseError> {
        let record = {
            let mut state = self.state.lock().expect("simulator lock");
            state
                .prepared
                .remove(id)
                .ok_or_else(|| ComputerUseError::PreparedActionNotFound(id.to_string()))?
        };
        if record.public.expires_at_ms < now_ms() {
            return Err(ComputerUseError::PreparedActionExpired);
        }
        Self::validate_window(&record.request.window)?;
        authorize_commit(&record, self)?;

        let outcome = if record.request.dry_run {
            ReceiptOutcome::DryRun
        } else {
            let lease = self.leases.begin()?;
            apply_action(self, &record.request.action, &lease)?;
            drop(lease);
            ReceiptOutcome::Succeeded
        };
        let mut state = self.state.lock().expect("simulator lock");
        if !record.request.dry_run {
            Self::invalidate(&mut state);
        }
        let receipt_id = format!("sim-receipt-{}", state.receipt_sequence);
        state.receipt_sequence += 1;
        Ok(ActionReceipt {
            receipt_id,
            prepared_action_id: record.public.id,
            application_identity_key: record.public.application.identity_key,
            bundle_id: record.public.window.bundle_id.clone(),
            pid: record.public.window.pid,
            window_id: record.public.window.window_id,
            action_kind: record.public.kind,
            disposition: record.public.assessment.disposition,
            outcome,
            payload_summary: record
                .public
                .preview
                .payload_summary
                .unwrap_or_else(|| "no sensitive payload".to_string()),
            completed_at_ms: now_ms(),
            persisted: true,
        })
    }

    #[cfg(test)]
    pub(crate) fn simulate_user_takeover(&self) {
        self.leases.mark_user_takeover();
    }
}

fn authorize_commit(
    record: &SimPreparedRecord,
    backend: &SimulatedComputerBackend,
) -> Result<(), ComputerUseError> {
    match record.public.assessment.disposition {
        ActionDisposition::Deny => Err(ComputerUseError::ActionDenied(
            record.public.assessment.reason.clone(),
        )),
        ActionDisposition::MandatoryHandoff => Err(ComputerUseError::HumanHandoffRequired(
            record.public.assessment.reason.clone(),
        )),
        ActionDisposition::ActionTimeConfirmation => record
            .authorization
            .is_some_and(|authorization| authorization == ActionAuthorization::Once)
            .then_some(())
            .ok_or(ComputerUseError::ApprovalRequired),
        ActionDisposition::PreapprovalEligible => {
            let state = backend.state.lock().expect("simulator lock");
            (record.authorization.is_some()
                || state
                    .approved_identities
                    .contains(&record.public.application.identity_key))
            .then_some(())
            .ok_or(ComputerUseError::ApprovalRequired)
        }
        ActionDisposition::Allow => Ok(()),
    }
}

fn apply_action(
    backend: &SimulatedComputerBackend,
    action: &ComputerAction,
    lease: &crate::lease::InputLease,
) -> Result<(), ComputerUseError> {
    lease.check()?;
    if let ComputerAction::Drag { duration_ms, .. } = action {
        let steps = (*duration_ms / 10).clamp(1, 200);
        for _ in 0..steps {
            lease.check()?;
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    lease.check()?;
    let mut state = backend.state.lock().expect("simulator lock");
    match action {
        ComputerAction::Click { element_id, .. } => match element_id.as_deref() {
            Some("ax-2") => state.status = "Opened example".to_string(),
            Some("ax-4") => state.status = "Deleted record".to_string(),
            _ => state.status = "Clicked".to_string(),
        },
        ComputerAction::TypeText { text, replace, .. } => {
            if *replace {
                state.input = text.clone();
            } else {
                state.input.push_str(text);
            }
            state.status = "Text entered".to_string();
        }
        ComputerAction::Keypress { key, .. } => {
            state.status = format!("Pressed {key:?}");
        }
        ComputerAction::Scroll {
            delta_x, delta_y, ..
        } => {
            state.status = format!("Scrolled {delta_x},{delta_y}");
        }
        ComputerAction::Drag { .. } => state.status = "Dragged".to_string(),
        ComputerAction::SecondaryAction { element_id, action } => {
            state.status = format!("Performed {action} on {element_id}");
        }
        ComputerAction::SelectText { start, end, .. } => {
            state.status = format!("Selected {start}..{end}");
        }
        ComputerAction::SetValue { value, .. } => {
            state.slider_value = *value;
            state.status = "Value set".to_string();
        }
    }
    Ok(())
}

fn validate_points(action: &ComputerAction) -> Result<(), ComputerUseError> {
    let bounds = crate::Rect {
        x: 0.0,
        y: 0.0,
        width: WIDTH as f64,
        height: HEIGHT as f64,
    };
    let points = match action {
        ComputerAction::Click { point, .. } => point.iter().collect::<Vec<_>>(),
        ComputerAction::Drag { start, end, .. } => {
            start.point.iter().chain(end.point.iter()).collect()
        }
        _ => Vec::new(),
    };
    for point in points {
        if !bounds.contains(*point) {
            return Err(ComputerUseError::PointOutOfBounds {
                x: point.x,
                y: point.y,
            });
        }
    }
    Ok(())
}

fn application_identity() -> ApplicationIdentity {
    ApplicationIdentity {
        bundle_id: SimulatedComputerBackend::BUNDLE_ID.to_string(),
        team_identifier: Some("AGENT-SIMULATOR".to_string()),
        designated_requirement:
            "identifier com.agent-desktop.computer-use-simulator and anchor simulator".to_string(),
        identity_key: "simulator-signer-v1".to_string(),
        durable_approval_eligible: true,
    }
}

fn preview(action: &ComputerAction) -> RedactedActionPreview {
    let (summary, element_id, payload_summary) = match action {
        ComputerAction::Click { element_id, .. } => (
            "Click in the observed window".to_string(),
            element_id.clone(),
            None,
        ),
        ComputerAction::TypeText {
            element_id, text, ..
        } => (
            "Enter text in an observed control".to_string(),
            Some(element_id.clone()),
            Some(format!(
                "text redacted ({} characters)",
                text.chars().count()
            )),
        ),
        ComputerAction::Keypress { key, modifiers } => (
            "Send a bounded keypress".to_string(),
            None,
            Some(format!(
                "{} modifier(s), {}",
                modifiers.len(),
                if matches!(key, crate::Key::Character(_)) {
                    "character redacted"
                } else {
                    "named key"
                }
            )),
        ),
        ComputerAction::Scroll {
            element_id,
            delta_x,
            delta_y,
        } => (
            "Scroll the observed window".to_string(),
            element_id.clone(),
            Some(format!("bounded delta {delta_x},{delta_y}")),
        ),
        ComputerAction::Drag { duration_ms, .. } => (
            "Drag within the observed window".to_string(),
            None,
            Some(format!("bounded duration {duration_ms} ms")),
        ),
        ComputerAction::SecondaryAction { element_id, action } => (
            format!("Perform advertised Accessibility action {action}"),
            Some(element_id.clone()),
            None,
        ),
        ComputerAction::SelectText {
            element_id,
            start,
            end,
        } => (
            "Select text in an observed control".to_string(),
            Some(element_id.clone()),
            Some(format!("range {start}..{end}")),
        ),
        ComputerAction::SetValue { element_id, .. } => (
            "Set a constrained numeric value".to_string(),
            Some(element_id.clone()),
            Some("numeric value redacted".to_string()),
        ),
    };
    RedactedActionPreview {
        summary,
        app_name: "Agent Computer Use Simulator".to_string(),
        bundle_id: SimulatedComputerBackend::BUNDLE_ID.to_string(),
        pid: 42_424,
        window_id: 7,
        element_id,
        payload_summary,
    }
}
