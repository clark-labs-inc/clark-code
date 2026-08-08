use std::path::PathBuf;
use std::time::{Duration, Instant};

use exec_core::{Executor, LocalExecutor, ProcessSpec};
use exec_sandbox::{BackendKind, SandboxManager, SandboxPolicy, SandboxStatus, SandboxedExecutor};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

#[derive(Serialize)]
struct CompilerResult {
    backend: String,
    iterations: usize,
    total_micros: u128,
    average_nanos: u128,
}

#[derive(Serialize)]
struct NativeResult {
    inside_write_exit: Option<i32>,
    outside_write_exit: Option<i32>,
    outside_file_created: bool,
    launch_iterations: usize,
    host_launch: LatencyResult,
    sandbox_launch: LatencyResult,
}

#[derive(Serialize)]
struct LatencyResult {
    average_micros: u128,
    p50_micros: u128,
    p95_micros: u128,
}

#[derive(Serialize)]
struct Report {
    compiler: Vec<CompilerResult>,
    native: Option<NativeResult>,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("sandbox benchmark failed: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let iterations = std::env::args()
        .skip_while(|arg| arg != "--iterations")
        .nth(1)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1_000)
        .max(1);
    let launch_iterations = argument("--launch-iterations")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(20)
        .max(1);
    let workspace = tempfile::tempdir().map_err(|error| error.to_string())?;
    let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
    let policy = SandboxPolicy::workspace_write(workspace.path().to_path_buf(), Vec::new());
    let original = ProcessSpec::argv("/bin/sh", workspace.path()).args(["-c", "printf benchmark"]);
    let mut compiler = Vec::new();
    for (backend, helper) in [
        (BackendKind::MacosSeatbelt, "/usr/bin/sandbox-exec"),
        (BackendKind::LinuxBubblewrap, "/usr/bin/bwrap"),
        (
            BackendKind::WindowsRestrictedToken,
            "agent-command-runner.exe",
        ),
    ] {
        let manager = SandboxManager::simulate(policy.clone(), backend, PathBuf::from(helper));
        let started = Instant::now();
        for _ in 0..iterations {
            std::hint::black_box(manager.prepare_process(original.clone())?);
        }
        let elapsed = started.elapsed();
        compiler.push(CompilerResult {
            backend: format!("{backend:?}"),
            iterations,
            total_micros: elapsed.as_micros(),
            average_nanos: elapsed.as_nanos() / iterations as u128,
        });
    }

    let native = match SandboxManager::current(policy) {
        Ok(manager) if matches!(manager.status(), SandboxStatus::Enforced { .. }) => {
            let executor = SandboxedExecutor::with_manager(manager)?;
            let inside = shell_write_command(&workspace.path().join("inside.txt"));
            let outside_command = shell_write_command(&outside.path().join("outside.txt"));
            let cancel = CancellationToken::new();
            let inside = executor
                .exec(&inside, workspace.path(), Duration::from_secs(10), &cancel)
                .await?;
            let outside_output = executor
                .exec(
                    &outside_command,
                    workspace.path(),
                    Duration::from_secs(10),
                    &cancel,
                )
                .await?;
            let host_launch =
                measure_launch(&LocalExecutor, workspace.path(), launch_iterations).await?;
            let sandbox_launch =
                measure_launch(&executor, workspace.path(), launch_iterations).await?;
            Some(NativeResult {
                inside_write_exit: inside.code,
                outside_write_exit: outside_output.code,
                outside_file_created: outside.path().join("outside.txt").exists(),
                launch_iterations,
                host_launch,
                sandbox_launch,
            })
        }
        _ => None,
    };
    if native.as_ref().is_some_and(|result| {
        result.inside_write_exit != Some(0)
            || result.outside_write_exit == Some(0)
            || result.outside_file_created
    }) {
        return Err("native containment invariant failed".to_string());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&Report { compiler, native })
            .map_err(|error| error.to_string())?
    );
    Ok(())
}

fn argument(name: &str) -> Option<String> {
    std::env::args()
        .skip_while(|argument| argument != name)
        .nth(1)
}

async fn measure_launch(
    executor: &dyn Executor,
    cwd: &std::path::Path,
    iterations: usize,
) -> Result<LatencyResult, String> {
    let cancel = CancellationToken::new();
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let output = executor
            .exec(noop_command(), cwd, Duration::from_secs(10), &cancel)
            .await?;
        if output.code != Some(0) {
            return Err(format!("no-op command failed: {:?}", output.stderr));
        }
        samples.push(started.elapsed().as_micros());
    }
    samples.sort_unstable();
    let average_micros = samples.iter().sum::<u128>() / samples.len() as u128;
    let percentile = |numerator: usize| {
        let index = (samples.len() * numerator)
            .div_ceil(100)
            .saturating_sub(1)
            .min(samples.len() - 1);
        samples[index]
    };
    Ok(LatencyResult {
        average_micros,
        p50_micros: percentile(50),
        p95_micros: percentile(95),
    })
}

#[cfg(unix)]
fn noop_command() -> &'static str {
    ":"
}

#[cfg(windows)]
fn noop_command() -> &'static str {
    "exit 0"
}

#[cfg(unix)]
fn shell_write_command(path: &std::path::Path) -> String {
    format!(
        "printf test > {}",
        shell_quote(path.to_string_lossy().as_ref())
    )
}

#[cfg(windows)]
fn shell_write_command(path: &std::path::Path) -> String {
    format!(
        "Set-Content -LiteralPath '{}' -Value test",
        path.to_string_lossy().replace('\'', "''")
    )
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
