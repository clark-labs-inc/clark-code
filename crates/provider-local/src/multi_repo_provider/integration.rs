use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_orchestration::{IntegrationCheck, IntegrationCheckReceipt, MultiRepoPlan};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::{Executor, FreshIntegrationWorkspace};

pub(super) async fn run_integration_checks(
    executor: &dyn Executor,
    plan: &MultiRepoPlan,
    workspace: &FreshIntegrationWorkspace,
    cancel: &CancellationToken,
) -> Vec<IntegrationCheckReceipt> {
    let mut receipts = Vec::with_capacity(plan.integration_checks.len());
    for check in &plan.integration_checks {
        receipts.push(run_check(executor, workspace, check, cancel).await);
    }
    receipts
}

async fn run_check(
    executor: &dyn Executor,
    workspace: &FreshIntegrationWorkspace,
    check: &IntegrationCheck,
    cancel: &CancellationToken,
) -> IntegrationCheckReceipt {
    let started_ms = now_ms();
    let root = workspace
        .repository_roots
        .get(&check.repository_id)
        .expect("validated integration check repository");
    let command = check
        .argv
        .iter()
        .map(|argument| crate::git_metadata::shell_word(argument))
        .collect::<Vec<_>>()
        .join(" ");
    let result = executor
        .exec(
            &command,
            root,
            Duration::from_millis(check.timeout_ms),
            cancel,
        )
        .await;
    let finished_ms = now_ms().max(started_ms);
    match result {
        Ok(output) => IntegrationCheckReceipt {
            id: check.id.clone(),
            repository_id: check.repository_id.clone(),
            argv: check.argv.clone(),
            started_ms,
            finished_ms,
            exit_code: output.code,
            stdout_sha256: digest(&output.stdout),
            stderr_sha256: digest(&output.stderr),
            passed: output.code == Some(0),
            error: (output.code != Some(0)).then(|| {
                format!(
                    "integration check exited with {:?}: {}",
                    output.code,
                    truncate(&output.stderr)
                )
            }),
        },
        Err(error) => IntegrationCheckReceipt {
            id: check.id.clone(),
            repository_id: check.repository_id.clone(),
            argv: check.argv.clone(),
            started_ms,
            finished_ms,
            exit_code: None,
            stdout_sha256: digest(&[]),
            stderr_sha256: digest(error.as_bytes()),
            passed: false,
            error: Some(error),
        },
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn truncate(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.chars().take(500).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_receipts_use_stable_empty_output_digest() {
        assert_eq!(
            digest(&[]),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn repository_ids_remain_typed_in_receipts() {
        assert_eq!(
            agent_orchestration::RepositoryId::new("api")
                .unwrap()
                .as_str(),
            "api"
        );
    }
}
