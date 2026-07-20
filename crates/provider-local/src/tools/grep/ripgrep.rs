use std::path::Path;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use super::{ToolCtx, ToolOutcome, MAX_MATCHES};

/// Run the pinned ripgrep sidecar for local searches. `None` means the command
/// is unavailable (normal in source builds) or the executor is remote, so the
/// caller should use the portable library implementation.
pub(super) async fn search(
    pattern: &str,
    base: &Path,
    name_filter: Option<&glob::Pattern>,
    mode: &str,
    ctx: &ToolCtx,
) -> Option<ToolOutcome> {
    if !ctx.executor.is_local() {
        return None;
    }

    let root = ctx.sandbox.root();
    let scope = base
        .strip_prefix(root)
        .ok()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut args = vec![
        "--color=never".to_string(),
        "--no-heading".to_string(),
        "--hidden".to_string(),
        "--no-require-git".to_string(),
        "--glob=!.git/**".to_string(),
        "--max-filesize=2M".to_string(),
        "--with-filename".to_string(),
    ];
    match mode {
        "files_with_matches" => {
            args.push("--files-with-matches".to_string());
        }
        "count" => {
            args.push("--count".to_string());
        }
        _ => {
            args.extend(
                [
                    "--line-number",
                    "--max-columns=400",
                    "--max-columns-preview",
                ]
                .into_iter()
                .map(str::to_string),
            );
        }
    }
    if let Some(filter) = name_filter {
        args.push("--glob".to_string());
        args.push(filter.as_str().to_string());
    }
    args.push("--".to_string());
    args.push(pattern.to_string());
    args.push(scope.to_string_lossy().into_owned());

    let process = match ctx.executor.prepare_process(
        exec_core::ProcessSpec::argv(clark_install_context::rg_command(), root).args(args),
    ) {
        Ok(process) => process,
        Err(error) => return Some(ToolOutcome::error(error)),
    };
    let mut child =
        match exec_core::spawn_process(&process, Stdio::null(), Stdio::piped(), Stdio::piped()) {
            Ok(child) => child,
            Err(error) if error.contains("No such file") || error.contains("not found") => {
                return None
            }
            Err(error) => {
                return Some(ToolOutcome::error(format!(
                    "failed to start bundled ripgrep: {error}"
                )))
            }
        };
    let stdout = child.stdout.take().expect("piped ripgrep stdout");
    let mut stderr = child.stderr.take().expect("piped ripgrep stderr");
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes).await;
        bytes
    });
    let mut reader = BufReader::new(stdout);
    let mut output = Vec::new();
    let mut match_count = 0usize;
    let mut truncated = false;

    loop {
        let mut line_bytes = Vec::new();
        let read = tokio::select! {
            _ = ctx.cancel.cancelled() => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Some(ToolOutcome::error("search cancelled"));
            }
            read = reader.read_until(b'\n', &mut line_bytes) => read,
        };
        match read {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Some(ToolOutcome::error(format!(
                    "failed reading ripgrep output: {error}"
                )));
            }
        }
        while matches!(line_bytes.last(), Some(b'\n' | b'\r')) {
            line_bytes.pop();
        }
        let line = normalize_line(&String::from_utf8_lossy(&line_bytes), mode);

        let contribution = if mode == "count" {
            line.rsplit_once(':')
                .and_then(|(_, count)| count.trim().parse::<usize>().ok())
                .unwrap_or(1)
        } else {
            1
        };
        match_count = match_count.saturating_add(contribution);
        output.push(line);
        if output.len() % 64 == 0 {
            ctx.report(format!("ripgrep found {match_count} matches\n"));
        }
        if match_count >= MAX_MATCHES || output.len() >= MAX_MATCHES {
            truncated = true;
            let _ = child.kill().await;
            break;
        }
    }

    let status = match child.wait().await {
        Ok(status) => status,
        Err(error) => {
            return Some(ToolOutcome::error(format!(
                "failed waiting for ripgrep: {error}"
            )))
        }
    };
    let stderr = stderr_task.await.unwrap_or_default();
    if !truncated && !status.success() && status.code() != Some(1) {
        let message = String::from_utf8_lossy(&stderr).trim().to_string();
        return Some(ToolOutcome::error(if message.is_empty() {
            format!("ripgrep exited with {status}")
        } else {
            format!("ripgrep failed: {message}")
        }));
    }
    if output.is_empty() {
        return Some(ToolOutcome::ok(format!("(no matches for `{pattern}`)")));
    }

    let mut body = output.join("\n");
    if truncated {
        body.push_str(&format!("\n… [truncated at {MAX_MATCHES} matches]"));
    }
    Some(ToolOutcome::ok(body))
}

fn normalize_line(line: &str, mode: &str) -> String {
    let line = line.strip_prefix("./").unwrap_or(line);
    if mode == "count" {
        return line
            .rsplit_once(':')
            .map(|(path, count)| format!("{path}: {count}"))
            .unwrap_or_else(|| line.to_string());
    }
    if mode != "count" && mode != "files_with_matches" {
        let mut fields = line.splitn(3, ':');
        if let (Some(path), Some(line_number), Some(content)) =
            (fields.next(), fields.next(), fields.next())
        {
            return format!("{path}:{line_number}: {content}");
        }
    }
    line.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_ripgrep_rows_to_the_tool_contract() {
        assert_eq!(
            normalize_line("./src/lib.rs:4:fn main() {}", "content"),
            "src/lib.rs:4: fn main() {}"
        );
        assert_eq!(normalize_line("./src/lib.rs:3", "count"), "src/lib.rs: 3");
        assert_eq!(
            normalize_line("./src/lib.rs", "files_with_matches"),
            "src/lib.rs"
        );
    }
}
