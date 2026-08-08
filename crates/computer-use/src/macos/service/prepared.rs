mod execute;
#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use crate::{
    assess_proposed_action, ActionAuthorization, ActionDisposition, ActionReceipt,
    ComputerUseError, PrepareActionRequest, PreparedAction, ReceiptOutcome, RedactedActionPreview,
};

use super::{
    accessibility, now_ms, LatestObservation, MacServiceBackend, PreparedRecord, OBSERVATION_TTL,
};

const PREPARED_TTL_MS: u64 = 60_000;
const MAX_PREPARED_ACTIONS: usize = 32;
const SETTLE_INTERVAL: Duration = Duration::from_millis(100);
const SETTLE_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) fn settled_walk(
    window: &crate::WindowInfo,
    screenshot_width: u32,
    screenshot_height: u32,
) -> Result<(accessibility::WalkResult, crate::ObservationSettlement), ComputerUseError> {
    let started = Instant::now();
    let mut samples = 1_u32;
    let mut walk = accessibility::walk_window(window, screenshot_width, screenshot_height)?;
    let mut fingerprint = crate::observation::settlement_fingerprint(
        &walk
            .elements
            .iter()
            .map(|element| element.info.clone())
            .collect::<Vec<_>>(),
    );
    let mut stable = false;
    while started.elapsed() < SETTLE_TIMEOUT {
        std::thread::sleep(SETTLE_INTERVAL);
        let next = accessibility::walk_window(window, screenshot_width, screenshot_height)?;
        samples += 1;
        let next_fingerprint = crate::observation::settlement_fingerprint(
            &next
                .elements
                .iter()
                .map(|element| element.info.clone())
                .collect::<Vec<_>>(),
        );
        walk = next;
        if next_fingerprint == fingerprint {
            stable = true;
            break;
        }
        fingerprint = next_fingerprint;
    }
    Ok((
        walk,
        crate::ObservationSettlement {
            stable,
            elapsed_ms: started.elapsed().as_millis() as u64,
            samples,
        },
    ))
}

impl MacServiceBackend {
    pub(super) fn prepare_impl(
        &self,
        request: PrepareActionRequest,
    ) -> Result<PreparedAction, ComputerUseError> {
        let observation = self.consume_observation(&request.window, &request.observation_id)?;
        execute::validate_action_geometry(&observation, &request.action)?;
        let application = super::super::auth::resolve_application_identity(
            request.window.pid,
            &request.window.bundle_id,
        )
        .map_err(ComputerUseError::HelperRejected)?;
        let (durable_grant, approval_revision) = self.approvals.is_granted(&application)?;
        let assessment = assess_proposed_action(
            &observation.window,
            &application,
            &observation.element_list,
            &request.intent,
            &request.action,
            request.dry_run,
            durable_grant,
        )?;
        let id = uuid::Uuid::new_v4().to_string();
        let public = PreparedAction {
            id: id.clone(),
            window: request.window.clone(),
            application,
            kind: request.action.kind(),
            assessment,
            preview: redacted_preview(&observation.window, &request.action),
            approval_revision,
            expires_at_ms: now_ms().saturating_add(PREPARED_TTL_MS),
            dry_run: request.dry_run,
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?;
        state
            .prepared
            .retain(|_, prepared| prepared.public.expires_at_ms >= now_ms());
        if state.prepared.len() >= MAX_PREPARED_ACTIONS {
            let oldest = state
                .prepared
                .iter()
                .min_by_key(|(_, prepared)| prepared.public.expires_at_ms)
                .map(|(id, _)| id.clone());
            if let Some(oldest) = oldest {
                state.prepared.remove(&oldest);
            }
        }
        state.prepared.insert(
            id,
            PreparedRecord {
                public: public.clone(),
                request,
                observation,
                authorization: None,
            },
        );
        Ok(public)
    }

    pub(super) fn prepared_impl(&self, id: &str) -> Result<PreparedAction, ComputerUseError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?;
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
        if authorization == ActionAuthorization::Denied {
            self.state
                .lock()
                .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?
                .prepared
                .remove(id);
            return Ok(());
        }
        let prepared = self.prepared_impl(id)?;
        match prepared.assessment.disposition {
            ActionDisposition::Deny => {
                return Err(ComputerUseError::ActionDenied(prepared.assessment.reason))
            }
            ActionDisposition::MandatoryHandoff => {
                return Err(ComputerUseError::HumanHandoffRequired(
                    prepared.assessment.reason,
                ))
            }
            ActionDisposition::ActionTimeConfirmation
            | ActionDisposition::PreapprovalEligible
            | ActionDisposition::Allow => {}
        }
        let revision = if authorization == ActionAuthorization::Durable {
            if prepared.assessment.disposition != ActionDisposition::PreapprovalEligible {
                return Err(ComputerUseError::ApprovalRequired);
            }
            self.approvals.grant(
                prepared.application.clone(),
                prepared.preview.app_name.clone(),
            )?
        } else {
            prepared.approval_revision
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?;
        let record = state
            .prepared
            .get_mut(id)
            .ok_or_else(|| ComputerUseError::PreparedActionNotFound(id.to_string()))?;
        record.authorization = Some(authorization);
        record.public.approval_revision = revision;
        Ok(())
    }

    pub(super) fn commit_impl(&self, id: &str) -> Result<ActionReceipt, ComputerUseError> {
        let record = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?
            .prepared
            .remove(id)
            .ok_or_else(|| ComputerUseError::PreparedActionNotFound(id.to_string()))?;
        if record.public.expires_at_ms < now_ms() {
            return Err(ComputerUseError::PreparedActionExpired);
        }
        // Register the cancellable lease before any target revalidation. The
        // control channel can therefore stop a commit that has been received
        // but has not yet reached its first synthesized input.
        let lease = if record.request.dry_run {
            None
        } else {
            Some(self.leases.begin()?)
        };
        let current_window = self.validate_target(&record.public.window)?;
        if super::frame_changed(record.observation.window.frame, current_window.frame) {
            return Err(ComputerUseError::ObservationStale);
        }
        let current_identity = super::super::auth::resolve_application_identity(
            record.public.window.pid,
            &record.public.window.bundle_id,
        )
        .map_err(ComputerUseError::HelperRejected)?;
        if current_identity != record.public.application {
            return Err(ComputerUseError::TargetChanged(
                "the target application's code-signing identity changed".to_string(),
            ));
        }
        let (durable_grant, current_revision) =
            self.approvals.is_granted(&record.public.application)?;
        let requires_durable_grant = authorize_commit(&record, durable_grant, current_revision)?;

        if record.request.dry_run {
            let mut receipt = receipt(&record, ReceiptOutcome::DryRun);
            receipt.persisted = self.approvals.record_receipt(receipt.clone()).is_ok();
            return Ok(receipt);
        }

        self.input_monitor.ensure_ready()?;
        self.reserve_input(&record.public.window)?;
        let approval_guard = self.approvals.begin_action(
            &record.public.application,
            record.public.approval_revision,
            requires_durable_grant,
        )?;
        debug_assert_eq!(approval_guard.revision, record.public.approval_revision);
        let result = execute::execute_action(
            self,
            &record,
            lease.as_ref().expect("non-dry-run lease created above"),
        );
        drop(lease);
        drop(approval_guard);
        let outcome = match &result {
            Ok(()) => ReceiptOutcome::Succeeded,
            Err(ComputerUseError::InputCancelled) => ReceiptOutcome::Cancelled,
            Err(ComputerUseError::UserTakeover) => ReceiptOutcome::UserTakeover,
            Err(_) => ReceiptOutcome::Failed,
        };
        let mut receipt = receipt(&record, outcome);
        receipt.persisted = self.approvals.record_receipt(receipt.clone()).is_ok();
        result?;
        Ok(receipt)
    }

    pub(super) fn commit_legacy(
        &self,
        prepared: PreparedAction,
        declared: crate::ActionRisk,
    ) -> Result<(), ComputerUseError> {
        if prepared.assessment.model_underclassified {
            self.authorize_impl(&prepared.id, ActionAuthorization::Denied)?;
            return Err(ComputerUseError::RiskDeclarationMismatch {
                declared,
                required: prepared.assessment.risk,
                reason: prepared.assessment.reason,
            });
        }
        self.authorize_impl(&prepared.id, ActionAuthorization::Once)?;
        self.commit_impl(&prepared.id).map(|_| ())
    }

    fn consume_observation(
        &self,
        target: &crate::WindowTarget,
        observation_id: &str,
    ) -> Result<LatestObservation, ComputerUseError> {
        let current = self.validate_target(target)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?;
        let latest = state
            .latest
            .get(target)
            .ok_or(ComputerUseError::ObservationRequired)?;
        if latest.id != observation_id
            || latest.observed_at.elapsed() > OBSERVATION_TTL
            || super::frame_changed(latest.window.frame, current.frame)
        {
            return Err(ComputerUseError::ObservationStale);
        }
        state
            .latest
            .remove(target)
            .ok_or(ComputerUseError::ObservationRequired)
    }

    fn reserve_input(&self, target: &crate::WindowTarget) -> Result<(), ComputerUseError> {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?;
        let entries = state.input_times.entry(target.clone()).or_default();
        while entries
            .front()
            .is_some_and(|timestamp| now.duration_since(*timestamp) >= super::INPUT_WINDOW)
        {
            entries.pop_front();
        }
        if entries.len() >= super::MAX_INPUTS_PER_WINDOW {
            return Err(ComputerUseError::RateLimited);
        }
        entries.push_back(now);
        Ok(())
    }
}

fn authorize_commit(
    record: &PreparedRecord,
    durable_grant: bool,
    current_revision: u64,
) -> Result<bool, ComputerUseError> {
    if record.public.approval_revision != current_revision {
        return Err(ComputerUseError::ApprovalRequired);
    }
    match record.public.assessment.disposition {
        ActionDisposition::Deny => Err(ComputerUseError::ActionDenied(
            record.public.assessment.reason.clone(),
        )),
        ActionDisposition::MandatoryHandoff => Err(ComputerUseError::HumanHandoffRequired(
            record.public.assessment.reason.clone(),
        )),
        ActionDisposition::ActionTimeConfirmation => (record.authorization
            == Some(ActionAuthorization::Once))
        .then_some(false)
        .ok_or(ComputerUseError::ApprovalRequired),
        ActionDisposition::PreapprovalEligible => {
            if durable_grant {
                Ok(true)
            } else if record.authorization == Some(ActionAuthorization::Once) {
                Ok(false)
            } else {
                Err(ComputerUseError::ApprovalRequired)
            }
        }
        ActionDisposition::Allow if record.request.dry_run => Ok(false),
        ActionDisposition::Allow => durable_grant
            .then_some(true)
            .ok_or(ComputerUseError::ApprovalRequired),
    }
}

fn receipt(record: &PreparedRecord, outcome: ReceiptOutcome) -> ActionReceipt {
    ActionReceipt {
        receipt_id: uuid::Uuid::new_v4().to_string(),
        prepared_action_id: record.public.id.clone(),
        application_identity_key: record.public.application.identity_key.clone(),
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
            .clone()
            .unwrap_or_else(|| "no sensitive payload".to_string()),
        completed_at_ms: now_ms(),
        persisted: false,
    }
}

fn redacted_preview(
    window: &crate::WindowInfo,
    action: &crate::ComputerAction,
) -> RedactedActionPreview {
    let (summary, element_id, payload_summary) = match action {
        crate::ComputerAction::Click { element_id, .. } => (
            "Click in the observed window".to_string(),
            element_id.clone(),
            None,
        ),
        crate::ComputerAction::TypeText {
            element_id, text, ..
        } => (
            "Enter text in an observed control".to_string(),
            Some(element_id.clone()),
            Some(format!(
                "text redacted ({} characters)",
                text.chars().count()
            )),
        ),
        crate::ComputerAction::Keypress { key, modifiers } => (
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
        crate::ComputerAction::Scroll {
            element_id,
            delta_x,
            delta_y,
        } => (
            "Scroll the observed window".to_string(),
            element_id.clone(),
            Some(format!("bounded delta {delta_x},{delta_y}")),
        ),
        crate::ComputerAction::Drag { duration_ms, .. } => (
            "Drag within the observed window".to_string(),
            None,
            Some(format!("bounded duration {duration_ms} ms")),
        ),
        crate::ComputerAction::SecondaryAction { element_id, action } => (
            format!("Perform advertised Accessibility action {action}"),
            Some(element_id.clone()),
            None,
        ),
        crate::ComputerAction::SelectText {
            element_id,
            start,
            end,
        } => (
            "Select text in an observed control".to_string(),
            Some(element_id.clone()),
            Some(format!("range {start}..{end}")),
        ),
        crate::ComputerAction::SetValue { element_id, .. } => (
            "Set a constrained numeric value".to_string(),
            Some(element_id.clone()),
            Some("numeric value redacted".to_string()),
        ),
    };
    RedactedActionPreview {
        summary,
        app_name: window.app_name.clone(),
        bundle_id: window.target.bundle_id.clone(),
        pid: window.target.pid,
        window_id: window.target.window_id,
        element_id,
        payload_summary,
    }
}
