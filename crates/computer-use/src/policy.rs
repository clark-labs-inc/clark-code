use crate::{
    ActionIntent, ActionRisk, ComputerUseError, ElementInfo, Key, Modifier, MouseButton,
    RiskAssessment, WindowInfo,
};

mod action;
mod catalog;

pub use action::assess_proposed_action;
use catalog::{
    BROWSER_APP_NAMES, BROWSER_BUNDLE_IDS, FORBIDDEN_APP_NAMES, FORBIDDEN_BUNDLE_FRAGMENTS,
    FORBIDDEN_BUNDLE_IDS,
};

const MAX_INTENT_REASON_CHARS: usize = 500;

pub fn ensure_bundle_allowed(bundle_id: &str) -> Result<(), ComputerUseError> {
    let bundle_id = bundle_id.trim();
    if bundle_id.is_empty() {
        return Err(forbidden(
            bundle_id,
            "the target has no verifiable bundle identity",
        ));
    }
    let normalized = bundle_id.to_ascii_lowercase();
    let product_bundle_ids = [
        env!("DESKTOP_COMPUTER_USE_PROD_APP_ID"),
        env!("DESKTOP_COMPUTER_USE_DEV_APP_ID"),
    ];
    if product_bundle_ids
        .iter()
        .any(|product| normalized == product.to_ascii_lowercase())
    {
        return Err(forbidden(
            bundle_id,
            "Clark Code and its privileged helpers cannot control their own UI",
        ));
    }
    if let Some((_, reason)) = FORBIDDEN_BUNDLE_IDS
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(bundle_id))
    {
        return Err(forbidden(bundle_id, *reason));
    }
    if let Some((_, reason)) = FORBIDDEN_BUNDLE_FRAGMENTS
        .iter()
        .find(|(fragment, _)| normalized.contains(fragment))
    {
        return Err(forbidden(bundle_id, *reason));
    }
    Ok(())
}

pub fn ensure_window_allowed(window: &WindowInfo) -> Result<(), ComputerUseError> {
    ensure_bundle_allowed(&window.target.bundle_id)?;
    if is_browser_target(window) {
        return Err(forbidden(
            &window.target.bundle_id,
            "browser windows require origin-aware browser control and fail closed through generic Accessibility",
        ));
    }
    let app_name = window.app_name.trim().to_ascii_lowercase();
    if FORBIDDEN_APP_NAMES
        .iter()
        .any(|forbidden| app_name == *forbidden || app_name.starts_with(&format!("{forbidden} ")))
    {
        return Err(forbidden(
            &window.target.bundle_id,
            "the resolved application is a forbidden terminal, credential, or system-security surface",
        ));
    }

    // Authentication agents occasionally use host-dependent bundle ids. Keep
    // this title fallback narrow to Apple/system agents so a browser tab about
    // Accessibility documentation is not mistaken for System Settings.
    let is_system_agent = window.target.bundle_id.starts_with("com.apple.")
        && ["agent", "settings", "preferences", "login"]
            .iter()
            .any(|marker| app_name.contains(marker));
    let title = window.title.to_ascii_lowercase();
    if is_system_agent
        && [
            "authentication required",
            "administrator password",
            "privacy & security",
            "security & privacy",
            "screen recording",
            "accessibility",
        ]
        .iter()
        .any(|marker| title.contains(marker))
    {
        return Err(forbidden(
            &window.target.bundle_id,
            "macOS authentication, privacy, and security dialogs are never controllable",
        ));
    }
    Ok(())
}

fn is_browser_target(window: &WindowInfo) -> bool {
    let bundle_id = window.target.bundle_id.trim().to_ascii_lowercase();
    let known_bundle = BROWSER_BUNDLE_IDS
        .iter()
        .any(|browser| bundle_id == *browser || bundle_id.starts_with(&format!("{browser}.")));
    let app_name = window.app_name.trim().to_ascii_lowercase();
    known_bundle
        || BROWSER_APP_NAMES
            .iter()
            .any(|browser| app_name == *browser || app_name.starts_with(&format!("{browser} ")))
}

pub fn assess_click(
    window: &WindowInfo,
    element: Option<&ElementInfo>,
    coordinate: bool,
    button: MouseButton,
) -> RiskAssessment {
    if coordinate {
        return assessment(
            ActionRisk::Ambiguous,
            "coordinate clicks have no Accessibility element identity",
        );
    }
    let Some(element) = element else {
        return assessment(
            ActionRisk::Ambiguous,
            "the click has no Accessibility element semantics",
        );
    };
    if button == MouseButton::Right {
        return assessment(
            ActionRisk::Ambiguous,
            "secondary clicks open context-dependent actions",
        );
    }
    assess_element(window, element)
}

pub fn assess_type_text(window: &WindowInfo, element: &ElementInfo) -> RiskAssessment {
    if element.sensitive_text || element.role == "AXSecureTextField" {
        return assessment(
            ActionRisk::Credential,
            "the destination is a secure or protected text field",
        );
    }
    assess_semantics(window, element, true)
}

pub fn assess_keypress(
    window: &WindowInfo,
    elements: impl Iterator<Item = ElementInfo>,
    key: Key,
    modifiers: &[Modifier],
) -> RiskAssessment {
    let focused = elements.into_iter().find(|element| element.focused);
    if focused.as_ref().is_some_and(|element| {
        (element.sensitive_text || element.role == "AXSecureTextField")
            && matches!(
                key,
                Key::Character(_) | Key::Backspace | Key::Delete | Key::Return
            )
    }) {
        return assessment(
            ActionRisk::Credential,
            "the keypress targets a secure or protected text field",
        );
    }
    if modifiers.contains(&Modifier::Command)
        && matches!(
            key,
            Key::Character('q' | 'Q' | 'w' | 'W') | Key::Delete | Key::Backspace
        )
    {
        return assessment(
            ActionRisk::Destructive,
            "the keyboard shortcut can close, quit, or delete content",
        );
    }
    if modifiers.contains(&Modifier::Command) && matches!(key, Key::Return | Key::Character('\n')) {
        return assessment(
            ActionRisk::ExternalCommunication,
            "Command-Return commonly sends or submits content",
        );
    }
    if matches!(key, Key::Delete | Key::Backspace) {
        return assessment(
            ActionRisk::Destructive,
            "Delete and Backspace can remove content or selected records",
        );
    }
    if matches!(key, Key::Return | Key::Space) {
        if let Some(focused) = focused.as_ref() {
            let focused_risk = assess_element(window, focused);
            if focused_risk.risk != ActionRisk::Routine {
                return focused_risk;
            }
        }
        return assessment(
            ActionRisk::Ambiguous,
            "Return and Space can activate a focused or default control",
        );
    }
    if modifiers.iter().any(|modifier| {
        matches!(
            modifier,
            Modifier::Command | Modifier::Control | Modifier::Option
        )
    }) {
        return assessment(
            ActionRisk::Ambiguous,
            "modified shortcuts have application-specific effects",
        );
    }
    if matches!(key, Key::Character(_)) {
        if let Some(focused) = focused
            .as_ref()
            .filter(|element| is_text_role(&element.role))
        {
            return assess_type_text(window, focused);
        }
        return assessment(
            ActionRisk::Ambiguous,
            "a character key outside a focused text control can invoke an application shortcut",
        );
    }
    if key == Key::Escape {
        return assessment(
            ActionRisk::Ambiguous,
            "Escape can cancel a modal action or discard in-progress state",
        );
    }
    assessment(
        ActionRisk::Routine,
        "the keypress is limited to focus or cursor navigation",
    )
}

pub fn validate_intent(
    intent: &ActionIntent,
    required: &RiskAssessment,
) -> Result<(), ComputerUseError> {
    validate_intent_shape(intent)?;
    if required.risk != ActionRisk::Routine && intent.risk != required.risk {
        return Err(ComputerUseError::RiskDeclarationMismatch {
            declared: intent.risk,
            required: required.risk,
            reason: required.reason.clone(),
        });
    }
    Ok(())
}

pub fn validate_intent_shape(intent: &ActionIntent) -> Result<(), ComputerUseError> {
    let reason = intent.reason.trim();
    if reason.is_empty() {
        return Err(ComputerUseError::InvalidActionIntent(
            "reason must be non-empty".to_string(),
        ));
    }
    if reason.chars().count() > MAX_INTENT_REASON_CHARS {
        return Err(ComputerUseError::InvalidActionIntent(format!(
            "reason exceeds {MAX_INTENT_REASON_CHARS} characters"
        )));
    }
    Ok(())
}

fn assess_element(window: &WindowInfo, element: &ElementInfo) -> RiskAssessment {
    if element.sensitive_text || element.role == "AXSecureTextField" {
        return assessment(
            ActionRisk::Credential,
            "the target is a secure or protected text control",
        );
    }
    if element.semantic_label().is_none()
        && element.actionable
        && matches!(
            element.role.as_str(),
            "AXButton" | "AXLink" | "AXMenuItem" | "AXPopUpButton"
        )
    {
        return assessment(
            ActionRisk::Ambiguous,
            "the actionable element has no label or description",
        );
    }
    if element.semantic_label().as_deref().is_some_and(|label| {
        matches!(
            label.trim().to_ascii_lowercase().as_str(),
            "approve" | "accept" | "confirm" | "continue" | "yes"
        )
    }) {
        return assessment(
            ActionRisk::Ambiguous,
            "a generic confirmation control needs contextual human review",
        );
    }
    assess_semantics(window, element, false)
}

fn assess_semantics(
    window: &WindowInfo,
    element: &ElementInfo,
    text_destination: bool,
) -> RiskAssessment {
    let element_text = [
        element.name.as_deref(),
        element.description.as_deref(),
        element.value.as_deref(),
    ]
    .into_iter()
    .flatten()
    .chain(element.actions.iter().map(String::as_str))
    .collect::<Vec<_>>()
    .join(" ");
    let context =
        format!("{} {} {}", window.app_name, window.title, element_text).to_ascii_lowercase();

    if contains_any(
        &context,
        &[
            "privacy & security",
            "security & privacy",
            "screen recording",
            "accessibility permission",
            "administrator",
            "authorize",
            "grant access",
            "allow access",
            "install",
            "uninstall",
            "security setting",
            "full disk access",
            "system extension",
            "enable extension",
            "configuration profile",
            "developer mode",
            "firewall",
        ],
    ) {
        return assessment(
            ActionRisk::SecuritySensitive,
            "the target can change authorization, installation, or security state",
        );
    }
    if contains_any(
        &context,
        &[
            "checkout",
            "buy now",
            "apple pay",
            "google pay",
            "paypal",
            "venmo",
            "purchase",
            "pay now",
            "payment",
            "place order",
            "confirm order",
            "transfer money",
            "send money",
            "wire transfer",
            "subscribe",
            "donate",
            "credit card",
            "card number",
            "cvv",
        ],
    ) {
        return assessment(
            ActionRisk::Financial,
            "the target is part of a purchase, payment, subscription, or transfer",
        );
    }
    if contains_any(
        &context,
        &[
            "password",
            "passcode",
            "one-time code",
            "verification code",
            "two-factor",
            "2fa",
            "multi-factor",
            "mfa",
            "api key",
            "access token",
            "secret key",
            "authenticate",
            "unlock",
            "continue with google",
            "continue with apple",
            "sign in",
            "log in",
            "login",
        ],
    ) {
        return assessment(
            ActionRisk::Credential,
            "the target handles credentials or authentication",
        );
    }
    if contains_any(
        &context,
        &[
            "delete",
            "discard",
            "erase",
            "remove",
            "trash",
            "wipe",
            "reset",
            "revoke",
            "quit",
            "close without saving",
            "permanently",
        ],
    ) {
        return assessment(
            ActionRisk::Destructive,
            "the target can delete, discard, revoke, or permanently remove state",
        );
    }
    if contains_any(
        &context,
        &[
            "send", "submit", "publish", "post", "reply", "comment", "share", "invite", "upload",
            "message", "tweet",
        ],
    ) {
        return assessment(
            ActionRisk::ExternalCommunication,
            if text_destination {
                "the destination contains externally communicated content"
            } else {
                "the target can send, submit, publish, or share content"
            },
        );
    }
    assessment(
        ActionRisk::Routine,
        "no consequential semantic effect was detected",
    )
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn is_text_role(role: &str) -> bool {
    matches!(
        role,
        "AXTextField" | "AXTextArea" | "AXSearchField" | "AXSecureTextField" | "AXComboBox"
    )
}

fn assessment(risk: ActionRisk, reason: impl Into<String>) -> RiskAssessment {
    RiskAssessment {
        risk,
        reason: reason.into(),
    }
}

fn forbidden(bundle_id: &str, reason: impl Into<String>) -> ComputerUseError {
    ComputerUseError::TargetForbidden {
        bundle_id: bundle_id.to_string(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests;
