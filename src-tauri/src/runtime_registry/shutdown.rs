use std::time::Duration;

use super::RuntimeRegistry;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RuntimeShutdownReceipt {
    pub(crate) sessions: usize,
    pub(crate) workers: usize,
    pub(crate) skill_catalogs: usize,
}

impl RuntimeRegistry {
    /// Unpublish every runtime capability before bounded process shutdown.
    /// This is the sole whole-app exit path; no provider, MCP client, remote
    /// worker, SSH master, claim, or coordination entry remains registered.
    pub(crate) async fn shutdown_all(&self) -> RuntimeShutdownReceipt {
        *self.cloud_account_generation_write().await = None;
        self.command_claims.lock().await.clear();
        let sessions = std::mem::take(&mut *self.sessions.lock().await);
        let workers = std::mem::take(&mut *self.workers.lock().await);
        let skill_catalogs = std::mem::take(&mut *self.skill_catalogs.lock().await);
        self.handles_by_key.lock().await.clear();
        self.connect_gates.lock().await.clear();
        self.connect_circuits.lock().await.states.clear();
        let receipt = RuntimeShutdownReceipt {
            sessions: sessions.len(),
            workers: workers.len(),
            skill_catalogs: skill_catalogs.len(),
        };

        for (_, entry) in sessions {
            let mut live = entry.lock().await;
            live.closing = true;
            let session_id = live.session.id.clone();
            match tokio::time::timeout(
                Duration::from_secs(3),
                live.provider.close_session(&session_id),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, session = %session_id, "provider shutdown failed during app exit");
                }
                Err(_) => {
                    tracing::warn!(session = %session_id, "provider shutdown timed out during app exit");
                }
            }
        }
        for (_, runtime) in workers {
            if let Err(error) = runtime.worker.disconnect().await {
                tracing::warn!(%error, "remote worker shutdown failed during app exit");
            }
        }
        receipt
    }
}
