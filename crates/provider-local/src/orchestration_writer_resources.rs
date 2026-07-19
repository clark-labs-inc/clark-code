use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::background::TaskWaitOutcome;
use crate::tools::ToolCtx;
use crate::IntegrationReadinessGate;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResourceArgs {
    pub id: String,
    #[serde(default)]
    pub workdir: Option<String>,
    pub command: String,
    #[serde(default)]
    pub output_contains: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    120_000
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ResourceReceipt {
    pub resource_id: String,
    pub lease_id: String,
    pub requested_ms: u64,
    pub ready_ms: u64,
    pub released_ms: Option<u64>,
    pub waited_ms: u64,
    pub outcome: &'static str,
    pub health_checks: u32,
    pub host_supervised: bool,
}

#[derive(Clone)]
pub(super) struct StartedResource {
    args: ResourceArgs,
    task_id: String,
    requested_ms: u64,
}

struct ReadinessGate {
    receiver: watch::Receiver<Option<Result<(), String>>>,
}

type ReadinessSender = watch::Sender<Option<Result<(), String>>>;
type ReadinessChannel = (
    Option<Arc<dyn IntegrationReadinessGate>>,
    Option<ReadinessSender>,
);

#[async_trait]
impl IntegrationReadinessGate for ReadinessGate {
    async fn wait_ready(&self, cancel: CancellationToken) -> Result<(), String> {
        let mut receiver = self.receiver.clone();
        loop {
            if let Some(result) = receiver.borrow().clone() {
                return result;
            }
            tokio::select! {
                _ = cancel.cancelled() => {
                    return Err("integration cancelled while waiting for environment readiness".into());
                }
                changed = receiver.changed() => {
                    changed.map_err(|_| "environment readiness supervisor stopped unexpectedly".to_string())?;
                }
            }
        }
    }
}

pub(super) fn readiness_channel(enabled: bool) -> ReadinessChannel {
    if !enabled {
        return (None, None);
    }
    let (sender, receiver) = watch::channel(None);
    (Some(Arc::new(ReadinessGate { receiver })), Some(sender))
}

pub(super) async fn start(
    resources: &[ResourceArgs],
    ctx: &ToolCtx,
) -> Result<Vec<StartedResource>, String> {
    let mut ids = std::collections::BTreeSet::new();
    let mut started = Vec::with_capacity(resources.len());
    for resource in resources {
        if resource.id.trim().is_empty()
            || resource.command.trim().is_empty()
            || !ids.insert(resource.id.clone())
        {
            release(&started, ctx, None).await;
            return Err("resource ids and commands must be non-empty and ids unique".into());
        }
        if resource.timeout_ms == 0 || resource.timeout_ms > 600_000 {
            release(&started, ctx, None).await;
            return Err(format!("resource {} timeout is out of range", resource.id));
        }
        let workdir = resource.workdir.as_deref().unwrap_or(".");
        let cwd = match ctx.sandbox.resolve_existing(workdir) {
            Ok(cwd) => cwd,
            Err(error) => {
                release(&started, ctx, None).await;
                return Err(error);
            }
        };
        let task_id = match ctx
            .background
            .spawn(ctx.executor.clone(), resource.command.clone(), &cwd)
            .await
        {
            Ok(task_id) => task_id,
            Err(error) => {
                release(&started, ctx, None).await;
                return Err(error);
            }
        };
        started.push(StartedResource {
            args: resource.clone(),
            task_id,
            requested_ms: now_ms(),
        });
    }
    Ok(started)
}

pub(super) async fn supervise(
    resources: &[StartedResource],
    ctx: &ToolCtx,
    sender: ReadinessSender,
) -> Result<Vec<ResourceReceipt>, String> {
    let mut receipts = Vec::with_capacity(resources.len());
    for resource in resources {
        let waited = ctx
            .background
            .wait(
                &resource.task_id,
                resource.args.output_contains.as_deref(),
                Duration::from_millis(resource.args.timeout_ms),
                Duration::from_millis(100),
                &ctx.cancel,
            )
            .await;
        let waited = match waited {
            Ok(waited) => waited,
            Err(error) => {
                let _ = sender.send(Some(Err(error.clone())));
                return Err(error);
            }
        };
        let successful_exit = waited.status.exit_code == Some(Some(0));
        let ready = waited.outcome == TaskWaitOutcome::Ready
            || (resource.args.output_contains.is_none()
                && waited.outcome == TaskWaitOutcome::Finished
                && successful_exit);
        if !ready || waited.status.error.is_some() {
            let error = format!(
                "resource {} failed readiness after {} ms",
                resource.args.id,
                waited.waited.as_millis()
            );
            let _ = sender.send(Some(Err(error.clone())));
            return Err(error);
        }
        receipts.push(ResourceReceipt {
            resource_id: resource.args.id.clone(),
            lease_id: resource.task_id.clone(),
            requested_ms: resource.requested_ms,
            ready_ms: now_ms(),
            released_ms: None,
            waited_ms: waited.waited.as_millis().try_into().unwrap_or(u64::MAX),
            outcome: "ready",
            health_checks: 1,
            host_supervised: true,
        });
    }
    let _ = sender.send(Some(Ok(())));
    Ok(receipts)
}

pub(super) async fn release(
    resources: &[StartedResource],
    ctx: &ToolCtx,
    mut receipts: Option<&mut Vec<ResourceReceipt>>,
) {
    for resource in resources {
        let _ = ctx.background.kill(&resource.task_id).await;
        if let Some(receipts) = receipts.as_deref_mut() {
            if let Some(receipt) = receipts
                .iter_mut()
                .find(|receipt| receipt.resource_id == resource.args.id)
            {
                receipt.released_ms = Some(now_ms());
            }
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ReadTracker;
    use std::sync::Mutex;

    fn ctx(root: &std::path::Path) -> ToolCtx {
        ToolCtx {
            sandbox: Arc::new(crate::sandbox::Sandbox::new(root).unwrap()),
            executor: Arc::new(crate::exec::LocalExecutor),
            reads: Arc::new(Mutex::new(ReadTracker::default())),
            cancel: CancellationToken::new(),
            background: Arc::new(crate::background::BackgroundTasks::default()),
            session: Arc::new(tokio::sync::Mutex::new(
                crate::loop_state::SessionState::default(),
            )),
            progress: None,
            agent_progress: None,
        }
    }

    #[tokio::test]
    async fn host_supervises_readiness_and_releases_the_resource() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = ctx(temp.path());
        let resources = vec![ResourceArgs {
            id: "test-service".into(),
            command: "printf SERVICE_READY; sleep 30".into(),
            output_contains: Some("SERVICE_READY".into()),
            workdir: None,
            timeout_ms: 2_000,
        }];
        let started = start(&resources, &ctx).await.unwrap();
        let (gate, sender) = readiness_channel(true);
        let sender = sender.unwrap();
        let gate = gate.unwrap();
        let (receipts, gate_result) = tokio::join!(
            supervise(&started, &ctx, sender),
            gate.wait_ready(CancellationToken::new())
        );
        gate_result.unwrap();
        let mut receipts = receipts.unwrap();
        assert_eq!(receipts.len(), 1);
        assert!(receipts[0].host_supervised);
        release(&started, &ctx, Some(&mut receipts)).await;
        assert!(receipts[0].released_ms.is_some());
    }
}
