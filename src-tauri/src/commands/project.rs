use super::RemoteArg;
use crate::AppState;

pub(crate) async fn project_executor(
    remote: Option<RemoteArg>,
    state: &AppState,
) -> Result<Box<dyn provider_local::Executor>, String> {
    match remote {
        Some(remote) => {
            let owner = state
                .runtime_registry
                .cloud_account()
                .await
                .map(|account| account.account.as_str().to_string())
                .ok_or("Clark Code must be signed in before using a remote project")?;
            let account = crate::runtime_registry::AccountKey::new(owner)?;
            let handle = crate::runtime_registry::WorkerHandle::parse(&remote.id)?;
            let runtime = state.runtime_registry.resolve(&account, &handle).await?;
            Ok(Box::new(
                crate::remote_worker_executor::RemoteWorkerExecutor::new(
                    runtime.worker(),
                    runtime.project_id().as_str().to_string(),
                ),
            ))
        }
        None => Ok(Box::new(provider_local::LocalExecutor)),
    }
}
