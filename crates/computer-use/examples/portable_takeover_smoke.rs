#[cfg(any(target_os = "linux", target_os = "windows"))]
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn main() {
    eprintln!("portable_takeover_smoke is only available on Windows and Linux");
    std::process::exit(2);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    use computer_use::{
        ActionAuthorization, ActionIntent, ActionLocation, ActionRisk, ComputerAction, MouseButton,
        Point, PrepareActionRequest, WindowFilter,
    };

    let title = std::env::args()
        .nth(1)
        .ok_or("usage: portable_takeover_smoke <title>")?;
    let backend = computer_use::native_backend()?;
    let window = backend
        .list_windows(WindowFilter {
            bundle_id: None,
            title_contains: Some(title),
        })?
        .into_iter()
        .next()
        .ok_or("takeover fixture window was not found")?;
    let observation = backend.observe(&window.target)?;
    let width = observation.screenshot.width as f64;
    let height = observation.screenshot.height as f64;
    let prepared = backend.prepare_action(PrepareActionRequest {
        intent: ActionIntent {
            risk: ActionRisk::Ambiguous,
            reason: "QA drag used to verify physical user takeover".to_string(),
        },
        window: window.target,
        observation_id: observation.observation_id,
        action: ComputerAction::Drag {
            start: ActionLocation {
                element_id: None,
                point: Some(Point {
                    x: width * 0.25,
                    y: height * 0.5,
                }),
            },
            end: ActionLocation {
                element_id: None,
                point: Some(Point {
                    x: width * 0.75,
                    y: height * 0.5,
                }),
            },
            button: MouseButton::Left,
            duration_ms: 2_000,
        },
        dry_run: false,
    })?;
    backend.authorize_action(&prepared.id, ActionAuthorization::Once)?;
    println!("READY_FOR_PHYSICAL_INPUT");
    std::io::stdout().flush()?;
    match backend.commit_action(&prepared.id) {
        Err(computer_use::ComputerUseError::UserTakeover) => {
            println!(
                "{}",
                serde_json::json!({
                    "platform": std::env::consts::OS,
                    "takeover": "detected",
                    "input": "cancelled",
                })
            );
            Ok(())
        }
        Err(error) => Err(format!("takeover smoke failed with a different error: {error}").into()),
        Ok(receipt) => Err(format!(
            "physical takeover was not detected; action completed as {:?}",
            receipt.outcome
        )
        .into()),
    }
}
