//! Typed host access to session-owned background terminal tasks.

use agent_core::{BackgroundTask, BackgroundTaskState};

use super::*;

impl LocalAgentProvider {
    pub(super) async fn list_background_tasks(
        &self,
        session: &SessionId,
    ) -> Result<Vec<BackgroundTask>> {
        self.ensure_background_session(session)?;
        Ok(self
            .background
            .list()
            .await
            .into_iter()
            .map(|(id, status)| background_task(id, status))
            .collect())
    }

    pub(super) async fn stop_background(
        &mut self,
        session: &SessionId,
        id: &str,
    ) -> Result<BackgroundTask> {
        self.ensure_background_session(session)?;
        let status = self
            .background
            .status(id)
            .await
            .ok_or_else(|| Error::Other(format!("no background task `{id}`")))?;
        let mut task = background_task(id.to_string(), status);
        if task.state == BackgroundTaskState::Running {
            self.background.kill(id).await.map_err(Error::Other)?;
            task.state = BackgroundTaskState::Stopping;
        }
        Ok(task)
    }

    pub(super) async fn clean_background(
        &mut self,
        session: &SessionId,
    ) -> Result<Vec<BackgroundTask>> {
        self.ensure_background_session(session)?;
        Ok(self
            .background
            .clean_finished()
            .await
            .into_iter()
            .map(|(id, status)| background_task(id, status))
            .collect())
    }

    fn ensure_background_session(&self, session: &SessionId) -> Result<()> {
        if self.session_id.as_ref() == Some(session) {
            Ok(())
        } else {
            Err(Error::Unsupported(
                "Clark can only inspect terminals owned by the active session".into(),
            ))
        }
    }
}

fn background_task(id: String, status: crate::background::TaskStatus) -> BackgroundTask {
    let state = if let Some(message) = status.error {
        BackgroundTaskState::Failed { message }
    } else {
        match status.exit_code {
            None => BackgroundTaskState::Running,
            Some(code) => BackgroundTaskState::Exited { code },
        }
    };
    BackgroundTask {
        id,
        command: status.command,
        state,
        output: status.output,
    }
}
