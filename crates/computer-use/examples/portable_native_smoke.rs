#[cfg(any(target_os = "linux", target_os = "windows"))]
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn main() {
    eprintln!("portable_native_smoke is only available on Windows and Linux");
    std::process::exit(2);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use computer_use::{
        ActionAuthorization, ActionIntent, ActionRisk, ComputerAction, MouseButton, Point,
        PrepareActionRequest, WindowFilter,
    };
    use sha2::{Digest, Sha256};

    let mut arguments = std::env::args().skip(1);
    let title = arguments
        .next()
        .ok_or("usage: portable_native_smoke <title>")?;
    if arguments.next().is_some() {
        return Err("usage: portable_native_smoke <title>".into());
    }
    let backend = computer_use::native_backend()?;
    let permissions = backend.permissions()?;
    let mut windows = backend.list_windows(WindowFilter {
        bundle_id: None,
        title_contains: Some(title.clone()),
    })?;
    let window = windows
        .drain(..)
        .next()
        .ok_or_else(|| format!("no allowed window contains title {title:?}"))?;
    let before = backend.observe(&window.target)?;
    let point = Point {
        x: before.screenshot.width as f64 / 2.0,
        y: before.screenshot.height as f64 / 2.0,
    };
    let prepared = backend.prepare_action(PrepareActionRequest {
        intent: ActionIntent {
            risk: ActionRisk::Ambiguous,
            reason: "QA fixture button changes a local label only".to_string(),
        },
        window: window.target.clone(),
        observation_id: before.observation_id.clone(),
        action: ComputerAction::Click {
            element_id: None,
            point: Some(point),
            button: MouseButton::Left,
        },
        dry_run: false,
    })?;
    backend.authorize_action(&prepared.id, ActionAuthorization::Once)?;
    let receipt = backend.commit_action(&prepared.id)?;
    std::thread::sleep(std::time::Duration::from_millis(250));
    let after = backend.observe(&window.target)?;
    let before_hash = format!("{:x}", Sha256::digest(&before.screenshot.png));
    let after_hash = format!("{:x}", Sha256::digest(&after.screenshot.png));
    println!(
        "{}",
        serde_json::json!({
            "platform": std::env::consts::OS,
            "service_permissions": permissions,
            "window": {
                "pid": window.target.pid,
                "window_id": window.target.window_id,
                "application_identity": window.target.bundle_id,
                "title": window.title,
            },
            "observation": {
                "width": before.screenshot.width,
                "height": before.screenshot.height,
                "before_sha256": before_hash,
                "after_sha256": after_hash,
                "changed": before_hash != after_hash,
                "accessibility_truncated": before.accessibility_truncated,
            },
            "action": {
                "prepared_id": prepared.id,
                "disposition": prepared.assessment.disposition,
                "outcome": receipt.outcome,
                "persisted": receipt.persisted,
            }
        })
    );
    Ok(())
}
