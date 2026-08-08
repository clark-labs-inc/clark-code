use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use computer_use::{
    ActionAuthorization, ActionDisposition, ActionLocation, ComputerAction, MouseButton, Point,
    PrepareActionRequest, PreparedAction,
};
use serde_json::{json, Value};

use super::{
    action_preflight, backend_call, bool_arg, optional_f64, optional_string, parse_key,
    parse_modifiers, required_i64, required_intent, required_observation_id, required_string,
    target, validate_text_length, ComputerBackend,
};
use crate::tools::{
    PermissionScope, ToolCtx, ToolExecutor, ToolOutcome, ToolPermissionClass,
    ToolPermissionDecision,
};

const MAX_PREPARED_CACHE: usize = 64;

pub(super) fn executors(backend: Arc<dyn ComputerBackend>) -> Vec<Arc<dyn ToolExecutor>> {
    let cache = Arc::new(PreparedCache::default());
    let mut tools = ActionFlavor::ALL
        .into_iter()
        .map(|flavor| {
            Arc::new(PrepareAction {
                backend: backend.clone(),
                cache: cache.clone(),
                flavor,
            }) as Arc<dyn ToolExecutor>
        })
        .collect::<Vec<_>>();
    tools.push(Arc::new(CommitAction { backend, cache }));
    tools
}

#[derive(Clone, Copy)]
enum ActionFlavor {
    Click,
    TypeText,
    Keypress,
    Scroll,
    Drag,
    SecondaryAction,
    SelectText,
    SetValue,
}

impl ActionFlavor {
    const ALL: [Self; 8] = [
        Self::Click,
        Self::TypeText,
        Self::Keypress,
        Self::Scroll,
        Self::Drag,
        Self::SecondaryAction,
        Self::SelectText,
        Self::SetValue,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Click => "computer_click",
            Self::TypeText => "computer_type_text",
            Self::Keypress => "computer_keypress",
            Self::Scroll => "computer_scroll",
            Self::Drag => "computer_drag",
            Self::SecondaryAction => "computer_secondary_action",
            Self::SelectText => "computer_select_text",
            Self::SetValue => "computer_set_value",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Click => "Prepare a click against one exact, freshly observed window. This does not send input. It returns a trusted disposition and opaque prepared_action_id; call computer_commit_action to execute an allowed or user-approved action.",
            Self::TypeText => "Prepare redacted text entry into a freshly observed text control. This does not send input or persist the text. Credential fields are classified for mandatory user handoff. Call computer_commit_action only with the returned opaque id.",
            Self::Keypress => "Prepare one bounded keypress against a freshly observed window. The trusted backend classifies activation, deletion, submission, and shortcut semantics before returning an opaque commit id.",
            Self::Scroll => "Prepare a bounded scroll against a freshly observed window or element. The preparation consumes the observation but sends no input.",
            Self::Drag => "Prepare a bounded left-button drag between observed elements or screenshot-local points. Drag effects require action-time review.",
            Self::SecondaryAction => "Prepare an Accessibility action explicitly advertised by an observed element and present in Agent Desktop's bounded allowlist.",
            Self::SelectText => "Prepare a bounded text selection in an observed editable control. Secure text selection requires user handoff.",
            Self::SetValue => "Prepare a numeric slider or incrementor update constrained by the observed minimum, maximum, and step.",
        }
    }

    fn schema(self) -> Value {
        match self {
            Self::Click => super::schemas::click(),
            Self::TypeText => super::schemas::type_text(),
            Self::Keypress => super::schemas::keypress(),
            Self::Scroll => super::schemas::scroll(),
            Self::Drag => super::schemas::drag(),
            Self::SecondaryAction => super::schemas::secondary_action(),
            Self::SelectText => super::schemas::select_text(),
            Self::SetValue => super::schemas::set_value(),
        }
    }
}

struct PrepareAction {
    backend: Arc<dyn ComputerBackend>,
    cache: Arc<PreparedCache>,
    flavor: ActionFlavor,
}

#[async_trait]
impl ToolExecutor for PrepareAction {
    fn name(&self) -> &str {
        self.flavor.name()
    }

    fn description(&self) -> &str {
        self.flavor.description()
    }

    fn parameters(&self) -> Value {
        self.flavor.schema()
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn permission_class(&self) -> ToolPermissionClass {
        ToolPermissionClass::LocalRead
    }

    fn permission_preflight(&self, args: &Value) -> Result<(), String> {
        action_preflight(args)
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let request = match parse_request(self.flavor, &args) {
            Ok(request) => request,
            Err(error) => return ToolOutcome::error(error),
        };
        let backend = self.backend.clone();
        match backend_call(ctx, move || backend.prepare_action(request)).await {
            Ok(prepared) => {
                self.cache.insert(prepared.clone());
                prepared_outcome(&prepared)
            }
            Err(error) => ToolOutcome::error(error),
        }
    }
}

struct CommitAction {
    backend: Arc<dyn ComputerBackend>,
    cache: Arc<PreparedCache>,
}

#[async_trait]
impl ToolExecutor for CommitAction {
    fn name(&self) -> &str {
        "computer_commit_action"
    }

    fn description(&self) -> &str {
        "Commit exactly one opaque prepared_action_id returned by a computer action tool. Agent Desktop derives the permission prompt, durable signer-bound approval eligibility, and redacted preview from trusted native preparation state. Never reconstruct or modify the action payload."
    }

    fn parameters(&self) -> Value {
        super::schemas::commit_action()
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn permission_class(&self) -> ToolPermissionClass {
        ToolPermissionClass::External
    }

    fn permission_preflight(&self, args: &Value) -> Result<(), String> {
        let prepared = self.cached(args)?;
        match prepared.assessment.disposition {
            ActionDisposition::Deny => Err(format!(
                "trusted computer-use policy denied this prepared action: {}",
                prepared.assessment.reason
            )),
            ActionDisposition::MandatoryHandoff => Err(format!(
                "this action must be completed by the user: {}",
                prepared.assessment.reason
            )),
            ActionDisposition::ActionTimeConfirmation
            | ActionDisposition::PreapprovalEligible
            | ActionDisposition::Allow => Ok(()),
        }
    }

    fn permission_scope(&self, args: &Value) -> Option<PermissionScope> {
        let prepared = self.cached(args).ok()?;
        let app = &prepared.preview.app_name;
        let summary = &prepared.preview.summary;
        let reason = Some(prepared.assessment.reason.clone());
        match prepared.assessment.disposition {
            ActionDisposition::Deny | ActionDisposition::MandatoryHandoff => None,
            ActionDisposition::ActionTimeConfirmation => Some(PermissionScope {
                key: format!("computer-action:{}:one-off", prepared.id),
                title: Some(format!("{summary} in {app}?")),
                always_label: None,
                reason,
                risk: Some("confirm".to_string()),
                remember: false,
                preapproved: false,
            }),
            ActionDisposition::PreapprovalEligible => Some(PermissionScope {
                key: format!("computer-action:{}", prepared.id),
                title: Some(format!("{summary} in {app}?")),
                always_label: Some(format!(
                    "Always allow routine actions for this signed copy of {app}"
                )),
                reason,
                risk: Some("confirm".to_string()),
                remember: true,
                preapproved: false,
            }),
            ActionDisposition::Allow => Some(PermissionScope {
                key: format!("computer-action:{}", prepared.id),
                title: Some(format!("{summary} in {app}")),
                always_label: None,
                reason,
                risk: None,
                remember: false,
                preapproved: true,
            }),
        }
    }

    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        let prepared = self.cached(args).ok()?;
        let preview = prepared.preview;
        Some(format!(
            "{}\nApp: {} ({})\nPID: {}\nWindow: {}\nTarget: {}\nPayload: {}",
            preview.summary,
            preview.app_name,
            preview.bundle_id,
            preview.pid,
            preview.window_id,
            preview.element_id.as_deref().unwrap_or("window"),
            preview
                .payload_summary
                .as_deref()
                .unwrap_or("no sensitive payload"),
        ))
    }

    async fn permission_decision(
        &self,
        args: &Value,
        decision: ToolPermissionDecision,
        ctx: &ToolCtx,
    ) -> Result<(), String> {
        let prepared = self.cached(args)?;
        let authorization = match decision {
            ToolPermissionDecision::AllowOnce => ActionAuthorization::Once,
            ToolPermissionDecision::AllowAlways => ActionAuthorization::Durable,
            ToolPermissionDecision::Denied => ActionAuthorization::Denied,
        };
        let id = prepared.id.clone();
        let backend = self.backend.clone();
        let result = backend_call(ctx, move || backend.authorize_action(&id, authorization)).await;
        if decision == ToolPermissionDecision::Denied {
            self.cache.remove(&prepared.id);
        }
        result
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let prepared = match self.cached(&args) {
            Ok(prepared) => prepared,
            Err(error) => return ToolOutcome::error(error),
        };
        let id = prepared.id.clone();
        let result = commit_backend_call(ctx, self.backend.clone(), id.clone()).await;
        self.cache.remove(&id);
        match result {
            Ok(receipt) => {
                let details = receipt_json(&receipt);
                ToolOutcome::ok(
                    serde_json::to_string_pretty(&details)
                        .unwrap_or_else(|_| "Computer action completed.".to_string()),
                )
                .with_details(details)
            }
            Err(error) => ToolOutcome::error(error),
        }
    }
}

impl CommitAction {
    fn cached(&self, args: &Value) -> Result<PreparedAction, String> {
        let id = prepared_id(args)?;
        self.cache.get(&id)
    }
}

#[derive(Default)]
struct PreparedCache {
    actions: Mutex<HashMap<String, PreparedAction>>,
}

impl PreparedCache {
    fn insert(&self, prepared: PreparedAction) {
        if let Ok(mut actions) = self.actions.lock() {
            let now = now_ms();
            actions.retain(|_, action| action.expires_at_ms >= now);
            if actions.len() >= MAX_PREPARED_CACHE {
                if let Some(oldest) = actions
                    .iter()
                    .min_by_key(|(_, action)| action.expires_at_ms)
                    .map(|(id, _)| id.clone())
                {
                    actions.remove(&oldest);
                }
            }
            actions.insert(prepared.id.clone(), prepared);
        }
    }

    fn get(&self, id: &str) -> Result<PreparedAction, String> {
        let actions = self
            .actions
            .lock()
            .map_err(|_| "prepared-action cache lock was poisoned".to_string())?;
        let prepared = actions
            .get(id)
            .cloned()
            .ok_or_else(|| "unknown prepared_action_id; prepare the action again".to_string())?;
        if prepared.expires_at_ms < now_ms() {
            return Err("prepared action expired; observe and prepare again".to_string());
        }
        Ok(prepared)
    }

    fn remove(&self, id: &str) {
        if let Ok(mut actions) = self.actions.lock() {
            actions.remove(id);
        }
    }
}

fn parse_request(flavor: ActionFlavor, args: &Value) -> Result<PrepareActionRequest, String> {
    let intent = required_intent(args)?;
    let window = target(args)?;
    let observation_id = required_observation_id(args)?;
    let action = match flavor {
        ActionFlavor::Click => ComputerAction::Click {
            element_id: optional_string(args, "element_id"),
            point: optional_point(args, "x", "y")?,
            button: mouse_button(args, "button", MouseButton::Left)?,
        },
        ActionFlavor::TypeText => {
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| "`text` must be a string".to_string())?
                .to_string();
            validate_text_length(&text)?;
            ComputerAction::TypeText {
                element_id: required_string(args, "element_id")?,
                text,
                replace: bool_arg(args, "replace", false)?,
            }
        }
        ActionFlavor::Keypress => ComputerAction::Keypress {
            key: parse_key(&required_string(args, "key")?)?,
            modifiers: parse_modifiers(args)?,
        },
        ActionFlavor::Scroll => ComputerAction::Scroll {
            element_id: optional_string(args, "element_id"),
            delta_x: bounded_i32(args, "delta_x")?,
            delta_y: bounded_i32(args, "delta_y")?,
        },
        ActionFlavor::Drag => ComputerAction::Drag {
            start: location(args, "start")?,
            end: location(args, "end")?,
            button: MouseButton::Left,
            duration_ms: positive_u32(args, "duration_ms")?,
        },
        ActionFlavor::SecondaryAction => ComputerAction::SecondaryAction {
            element_id: required_string(args, "element_id")?,
            action: required_string(args, "action")?,
        },
        ActionFlavor::SelectText => ComputerAction::SelectText {
            element_id: required_string(args, "element_id")?,
            start: nonnegative_u32(args, "start")?,
            end: nonnegative_u32(args, "end")?,
        },
        ActionFlavor::SetValue => ComputerAction::SetValue {
            element_id: required_string(args, "element_id")?,
            value: optional_f64(args, "value")?
                .ok_or_else(|| "`value` must be a finite number".to_string())?,
        },
    };
    validate_action_shape(&action)?;
    Ok(PrepareActionRequest {
        intent,
        window,
        observation_id,
        action,
        dry_run: bool_arg(args, "dry_run", false)?,
    })
}

fn validate_action_shape(action: &ComputerAction) -> Result<(), String> {
    match action {
        ComputerAction::Click {
            element_id, point, ..
        } if element_id.is_some() == point.is_some() => {
            Err("provide exactly one of `element_id` or screenshot-local `x` and `y`".to_string())
        }
        ComputerAction::Scroll {
            delta_x, delta_y, ..
        } if *delta_x == 0 && *delta_y == 0 => {
            Err("at least one scroll delta must be non-zero".to_string())
        }
        ComputerAction::SelectText { start, end, .. } if start > end => {
            Err("`start` must not exceed `end`".to_string())
        }
        _ => Ok(()),
    }
}

fn location(args: &Value, prefix: &str) -> Result<ActionLocation, String> {
    let element_id = optional_string(args, &format!("{prefix}_element_id"));
    let point = optional_point(args, &format!("{prefix}_x"), &format!("{prefix}_y"))?;
    if element_id.is_some() == point.is_some() {
        return Err(format!(
            "provide exactly one `{prefix}_element_id` or `{prefix}_x` and `{prefix}_y` pair"
        ));
    }
    Ok(ActionLocation { element_id, point })
}

fn optional_point(args: &Value, x_key: &str, y_key: &str) -> Result<Option<Point>, String> {
    match (optional_f64(args, x_key)?, optional_f64(args, y_key)?) {
        (Some(x), Some(y)) => Ok(Some(Point { x, y })),
        (None, None) => Ok(None),
        _ => Err(format!("`{x_key}` and `{y_key}` must be provided together")),
    }
}

fn mouse_button(args: &Value, key: &str, default: MouseButton) -> Result<MouseButton, String> {
    match args.get(key).and_then(Value::as_str) {
        None => Ok(default),
        Some("left") => Ok(MouseButton::Left),
        Some("right") => Ok(MouseButton::Right),
        Some(value) => Err(format!("unknown mouse button `{value}`")),
    }
}

fn bounded_i32(args: &Value, key: &str) -> Result<i32, String> {
    i32::try_from(required_i64(args, key)?).map_err(|_| format!("`{key}` is outside the i32 range"))
}

fn positive_u32(args: &Value, key: &str) -> Result<u32, String> {
    let value = nonnegative_u32(args, key)?;
    (value > 0)
        .then_some(value)
        .ok_or_else(|| format!("`{key}` must be positive"))
}

fn nonnegative_u32(args: &Value, key: &str) -> Result<u32, String> {
    u32::try_from(required_i64(args, key)?)
        .map_err(|_| format!("`{key}` must be a non-negative u32"))
}

fn prepared_id(args: &Value) -> Result<String, String> {
    let id = required_string(args, "prepared_action_id")?;
    if id.len() > 128 {
        return Err("`prepared_action_id` is too long".to_string());
    }
    Ok(id)
}

fn prepared_outcome(prepared: &PreparedAction) -> ToolOutcome {
    let details = json!({
        "prepared_action_id": prepared.id,
        "disposition": prepared.assessment.disposition.to_string(),
        "trusted_risk": prepared.assessment.risk.to_string(),
        "reason_code": prepared.assessment.reason_code,
        "reason": prepared.assessment.reason,
        "model_underclassified": prepared.assessment.model_underclassified,
        "expires_at_ms": prepared.expires_at_ms,
        "dry_run": prepared.dry_run,
        "preview": {
            "summary": prepared.preview.summary,
            "app_name": prepared.preview.app_name,
            "bundle_id": prepared.preview.bundle_id,
            "pid": prepared.preview.pid,
            "window_id": prepared.preview.window_id,
            "element_id": prepared.preview.element_id,
            "payload_summary": prepared.preview.payload_summary,
        },
        "next_step": match prepared.assessment.disposition {
            ActionDisposition::Deny => "Do not commit; observe or choose a safer action.",
            ActionDisposition::MandatoryHandoff => "Ask the user to complete this action directly.",
            ActionDisposition::ActionTimeConfirmation
            | ActionDisposition::PreapprovalEligible
            | ActionDisposition::Allow => "Call computer_commit_action with only prepared_action_id.",
        }
    });
    ToolOutcome::ok(
        serde_json::to_string_pretty(&details)
            .unwrap_or_else(|_| "Computer action prepared.".to_string()),
    )
    .with_details(details)
}

fn receipt_json(receipt: &computer_use::ActionReceipt) -> Value {
    json!({
        "receipt_id": receipt.receipt_id,
        "prepared_action_id": receipt.prepared_action_id,
        "bundle_id": receipt.bundle_id,
        "pid": receipt.pid,
        "window_id": receipt.window_id,
        "action_kind": format!("{:?}", receipt.action_kind).to_ascii_lowercase(),
        "disposition": receipt.disposition.to_string(),
        "outcome": format!("{:?}", receipt.outcome).to_ascii_lowercase(),
        "payload_summary": receipt.payload_summary,
        "completed_at_ms": receipt.completed_at_ms,
        "persisted": receipt.persisted,
        "observation_invalidated": true,
    })
}

async fn commit_backend_call(
    ctx: &ToolCtx,
    backend: Arc<dyn ComputerBackend>,
    id: String,
) -> Result<computer_use::ActionReceipt, String> {
    if ctx.cancel.is_cancelled() {
        return Err("cancelled before the prepared action was committed".to_string());
    }
    let commit_backend = backend.clone();
    let mut task = tokio::task::spawn_blocking(move || commit_backend.commit_action(&id));
    tokio::select! {
        biased;
        _ = ctx.cancel.cancelled() => {
            let cancel_backend = backend.clone();
            let cancellation = tokio::task::spawn_blocking(move || cancel_backend.cancel_active())
                .await
                .map_err(|error| format!("computer-use cancellation worker failed: {error}"))?
                .map_err(|error| error.to_string());
            let completion = (&mut task).await;
            match cancellation {
                Ok(ack) if ack.quiesced => Err(format!(
                    "cancelled; synthesized input is quiesced{}",
                    if ack.helper_terminated { " after helper termination" } else { "" }
                )),
                Ok(_) => {
                    let _ = completion;
                    Err("computer-use cancellation did not reach a quiesced state".to_string())
                }
                Err(error) => {
                    let _ = completion;
                    Err(format!("computer-use cancellation failed closed after the action worker stopped: {error}"))
                }
            }
        }
        result = &mut task => result
            .map_err(|error| format!("computer-use worker failed: {error}"))?
            .map_err(|error| error.to_string()),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
