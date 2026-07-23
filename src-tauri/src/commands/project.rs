use super::RemoteArg;

pub(super) async fn project_executor(
    remote: Option<RemoteArg>,
) -> Result<Box<dyn provider_local::Executor>, String> {
    match remote {
        Some(remote) => Ok(Box::new(
            provider_local::RemoteExecutor::connect(&remote.ws_url, &remote.token).await?,
        )),
        None => Ok(Box::new(provider_local::LocalExecutor)),
    }
}
