use super::*;
use crate::{ActionDisposition, ApplicationIdentity, ComputerAction, Rect, WindowTarget};

fn window(bundle_id: &str, app_name: &str, title: &str) -> WindowInfo {
    WindowInfo {
        target: WindowTarget {
            pid: 42,
            window_id: 7,
            bundle_id: bundle_id.to_string(),
        },
        app_name: app_name.to_string(),
        title: title.to_string(),
        frame: Rect::default(),
        layer: 0,
        on_screen: true,
    }
}

fn button(name: Option<&str>) -> ElementInfo {
    ElementInfo {
        id: "ax-1".to_string(),
        role: "AXButton".to_string(),
        name: name.map(str::to_string),
        value: None,
        description: None,
        bounds: Rect::default(),
        enabled: true,
        focused: false,
        actionable: true,
        actions: vec!["AXPress".to_string()],
        sensitive_text: false,
        value_settable: false,
        value_constraints: None,
    }
}

#[test]
fn forbidden_targets_cover_self_terminals_credentials_and_system_security() {
    for bundle_id in [
        "com.agent-desktop.desktop",
        "com.agent-desktop.desktop.dev",
        "com.agent-desktop.desktop.dev.computer-use-helper",
        "com.apple.Terminal",
        "com.googlecode.iterm2",
        "com.termius-dmg.mac",
        "com.vandyke.SecureCRT",
        "com.apple.keychainaccess",
        "com.1password.1password",
        "com.callpod.keepermac",
        "com.apple.systempreferences",
        "com.apple.SecurityAgent",
        "org.example.password-manager",
    ] {
        assert!(
            ensure_bundle_allowed(bundle_id).is_err(),
            "{bundle_id} should be forbidden"
        );
    }
    assert!(ensure_bundle_allowed("com.apple.Safari").is_ok());
    assert!(ensure_bundle_allowed("com.google.Chrome").is_ok());
}

#[test]
fn resolved_app_name_and_security_agent_title_fail_closed() {
    assert!(ensure_window_allowed(&window("org.example.terminal", "Terminal", "shell")).is_err());
    assert!(ensure_window_allowed(&window(
        "com.apple.SomeAgent",
        "Authorization Agent",
        "Administrator Password"
    ))
    .is_err());
    assert!(ensure_window_allowed(&window(
        "com.google.Chrome",
        "Google Chrome",
        "Accessibility documentation"
    ))
    .is_err());
}

#[test]
fn browser_targets_fail_closed_even_for_variants_and_unlisted_bundle_ids() {
    for target in [
        window(
            "com.google.Chrome.canary",
            "Google Chrome Canary",
            "Example",
        ),
        window("com.duckduckgo.macos.browser", "DuckDuckGo", "Example"),
        window("org.mozilla.nightly", "Firefox Nightly", "Example"),
        window("io.example.unlisted", "Orion", "Example"),
        window("io.example.unlisted", "Dia Beta", "Example"),
    ] {
        let error = ensure_window_allowed(&target).unwrap_err();
        assert!(
            error.to_string().contains("origin-aware browser control"),
            "{} should fail closed as a browser",
            target.app_name
        );
    }
}

fn signed_identity(bundle_id: &str) -> ApplicationIdentity {
    ApplicationIdentity {
        bundle_id: bundle_id.to_string(),
        team_identifier: Some("TEAM123".to_string()),
        designated_requirement: format!(
            "identifier {bundle_id} and anchor apple generic and certificate leaf[subject.OU] = TEAM123"
        ),
        identity_key: format!("identity:{bundle_id}"),
        durable_approval_eligible: true,
    }
}

fn routine_intent() -> ActionIntent {
    ActionIntent {
        risk: ActionRisk::Routine,
        reason: "operate the reviewed control".to_string(),
    }
}

#[test]
fn trusted_dispositions_distinguish_grants_confirmations_handoff_and_denial() {
    let textedit = window("com.apple.TextEdit", "TextEdit", "Untitled");
    let app = signed_identity("com.apple.TextEdit");
    let open = button(Some("Open example"));
    let routine = ComputerAction::Click {
        element_id: Some(open.id.clone()),
        point: None,
        button: MouseButton::Left,
    };

    let without_grant = assess_proposed_action(
        &textedit,
        &app,
        std::slice::from_ref(&open),
        &routine_intent(),
        &routine,
        false,
        false,
    )
    .unwrap();
    assert_eq!(
        without_grant.disposition,
        ActionDisposition::PreapprovalEligible
    );
    let with_grant = assess_proposed_action(
        &textedit,
        &app,
        std::slice::from_ref(&open),
        &routine_intent(),
        &routine,
        false,
        true,
    )
    .unwrap();
    assert_eq!(with_grant.disposition, ActionDisposition::Allow);

    let destructive = button(Some("Delete record"));
    let assessed = assess_proposed_action(
        &textedit,
        &app,
        std::slice::from_ref(&destructive),
        &routine_intent(),
        &ComputerAction::Click {
            element_id: Some(destructive.id.clone()),
            point: None,
            button: MouseButton::Left,
        },
        false,
        true,
    )
    .unwrap();
    assert_eq!(assessed.risk, ActionRisk::Destructive);
    assert_eq!(
        assessed.disposition,
        ActionDisposition::ActionTimeConfirmation
    );
    assert!(assessed.model_underclassified);

    let mut password = button(Some("Password"));
    password.role = "AXSecureTextField".to_string();
    password.sensitive_text = true;
    let assessed = assess_proposed_action(
        &textedit,
        &app,
        std::slice::from_ref(&password),
        &routine_intent(),
        &ComputerAction::TypeText {
            element_id: password.id.clone(),
            text: "never persisted".to_string(),
            replace: true,
        },
        false,
        true,
    )
    .unwrap();
    assert_eq!(assessed.risk, ActionRisk::Credential);
    assert_eq!(assessed.disposition, ActionDisposition::MandatoryHandoff);

    let browser = window("com.google.Chrome", "Google Chrome", "Example");
    let denied = assess_proposed_action(
        &browser,
        &signed_identity("com.google.Chrome"),
        std::slice::from_ref(&open),
        &routine_intent(),
        &routine,
        false,
        true,
    )
    .unwrap();
    assert_eq!(denied.disposition, ActionDisposition::Deny);
    assert_eq!(denied.reason_code, "target_forbidden");
}

#[test]
fn secondary_actions_and_numeric_values_are_constrained_by_observation() {
    let target = window("com.apple.TextEdit", "TextEdit", "Untitled");
    let app = signed_identity("com.apple.TextEdit");
    let mut slider = button(Some("Level"));
    slider.role = "AXSlider".to_string();
    slider.actions = vec!["AXIncrement".to_string()];
    slider.value_settable = true;
    slider.value_constraints = Some(crate::ValueConstraints {
        minimum: 0.0,
        maximum: 10.0,
        step: Some(0.5),
    });

    let allowed_value = assess_proposed_action(
        &target,
        &app,
        std::slice::from_ref(&slider),
        &routine_intent(),
        &ComputerAction::SetValue {
            element_id: slider.id.clone(),
            value: 7.5,
        },
        false,
        false,
    )
    .unwrap();
    assert_eq!(
        allowed_value.disposition,
        ActionDisposition::PreapprovalEligible
    );

    let denied_value = assess_proposed_action(
        &target,
        &app,
        std::slice::from_ref(&slider),
        &routine_intent(),
        &ComputerAction::SetValue {
            element_id: slider.id.clone(),
            value: 7.25,
        },
        false,
        false,
    )
    .unwrap();
    assert_eq!(denied_value.disposition, ActionDisposition::Deny);

    let denied_action = assess_proposed_action(
        &target,
        &app,
        std::slice::from_ref(&slider),
        &routine_intent(),
        &ComputerAction::SecondaryAction {
            element_id: slider.id.clone(),
            action: "AXDelete".to_string(),
        },
        false,
        false,
    )
    .unwrap();
    assert_eq!(denied_action.disposition, ActionDisposition::Deny);
}

#[test]
fn semantic_risks_cover_all_consequential_categories_and_icons() {
    let browser = window("com.google.Chrome", "Google Chrome", "Example");
    assert_eq!(
        assess_click(
            &browser,
            Some(&button(Some("Delete record"))),
            false,
            MouseButton::Left
        )
        .risk,
        ActionRisk::Destructive
    );
    assert_eq!(
        assess_click(
            &browser,
            Some(&button(Some("Pay now"))),
            false,
            MouseButton::Left
        )
        .risk,
        ActionRisk::Financial
    );
    assert_eq!(
        assess_click(
            &browser,
            Some(&button(Some("Send message"))),
            false,
            MouseButton::Left
        )
        .risk,
        ActionRisk::ExternalCommunication
    );
    assert_eq!(
        assess_click(
            &browser,
            Some(&button(Some("Sign in"))),
            false,
            MouseButton::Left
        )
        .risk,
        ActionRisk::Credential
    );
    assert_eq!(
        assess_click(
            &browser,
            Some(&button(Some("Grant access"))),
            false,
            MouseButton::Left
        )
        .risk,
        ActionRisk::SecuritySensitive
    );
    assert_eq!(
        assess_click(&browser, Some(&button(None)), false, MouseButton::Left).risk,
        ActionRisk::Ambiguous
    );
    assert_eq!(
        assess_click(
            &browser,
            Some(&button(Some("Confirm"))),
            false,
            MouseButton::Left
        )
        .risk,
        ActionRisk::Ambiguous
    );
    assert_eq!(
        assess_click(&browser, None, true, MouseButton::Left).risk,
        ActionRisk::Ambiguous
    );
}

#[test]
fn inferred_risk_rejects_underclassification_and_accepts_explicit_review() {
    let required = assessment(ActionRisk::Destructive, "deletes data");
    let error = validate_intent(
        &ActionIntent {
            risk: ActionRisk::Routine,
            reason: "click the icon".to_string(),
        },
        &required,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ComputerUseError::RiskDeclarationMismatch {
            required: ActionRisk::Destructive,
            ..
        }
    ));
    validate_intent(
        &ActionIntent {
            risk: ActionRisk::Destructive,
            reason: "delete the selected record".to_string(),
        },
        &required,
    )
    .unwrap();
}

#[test]
fn keypress_risk_covers_deletion_focused_activation_and_unknown_shortcuts() {
    let browser = window("com.google.Chrome", "Google Chrome", "Example");
    assert_eq!(
        assess_keypress(&browser, std::iter::empty(), Key::Delete, &[]).risk,
        ActionRisk::Destructive
    );

    let mut send = button(Some("Send message"));
    send.focused = true;
    assert_eq!(
        assess_keypress(&browser, std::iter::once(send), Key::Space, &[]).risk,
        ActionRisk::ExternalCommunication
    );

    let mut generic_text = button(Some("Search"));
    generic_text.role = "AXTextField".to_string();
    generic_text.focused = true;
    generic_text.actions = vec!["AXSetValue".to_string()];
    assert_eq!(
        assess_keypress(&browser, std::iter::once(generic_text), Key::Return, &[]).risk,
        ActionRisk::Ambiguous
    );

    assert_eq!(
        assess_keypress(
            &browser,
            std::iter::empty(),
            Key::Character('k'),
            &[Modifier::Command]
        )
        .risk,
        ActionRisk::Ambiguous
    );
    assert_eq!(
        assess_keypress(&browser, std::iter::empty(), Key::ArrowDown, &[]).risk,
        ActionRisk::Routine
    );
}

#[test]
fn keypress_into_a_secure_field_is_credential_sensitive() {
    let browser = window("com.google.Chrome", "Google Chrome", "Sign in");
    let mut secure = button(Some("Password"));
    secure.role = "AXSecureTextField".to_string();
    secure.focused = true;
    secure.sensitive_text = true;
    secure.actions = vec!["AXSetValue".to_string()];

    assert_eq!(
        assess_keypress(&browser, std::iter::once(secure), Key::Character('x'), &[]).risk,
        ActionRisk::Credential
    );
}
