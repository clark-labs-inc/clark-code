use super::AppendRequest;
use crate::commands::ProductCloudOutcome;
use crate::state::AppState;
use tauri::{AppHandle, Emitter};

#[async_trait::async_trait]
pub(super) trait TrajectoryCloudBoundary: Send + Sync {
    async fn append(
        &self,
        conversation_id: &str,
        request: &AppendRequest,
    ) -> Result<ProductCloudOutcome, String>;

    fn emit_auth_expired(&self);
    fn emit_sync_warning(&self, message: &str);
    fn emit_conversation_deleted(&self, conversation_id: &str);
}

pub(super) struct ProductTrajectoryCloudBoundary {
    app: AppHandle,
    state: AppState,
}

impl ProductTrajectoryCloudBoundary {
    pub(super) fn new(app: AppHandle, state: AppState) -> Self {
        Self { app, state }
    }
}

#[async_trait::async_trait]
impl TrajectoryCloudBoundary for ProductTrajectoryCloudBoundary {
    async fn append(
        &self,
        conversation_id: &str,
        request: &AppendRequest,
    ) -> Result<ProductCloudOutcome, String> {
        crate::commands::product_cloud_request(
            "conversation.append_trajectory",
            serde_json::json!({
                "conversationId": conversation_id,
                "request": request,
            }),
            &self.app,
            &self.state,
        )
        .await
    }

    fn emit_auth_expired(&self) {
        let _ = self.app.emit("cloud-auth-expired", ());
    }

    fn emit_sync_warning(&self, message: &str) {
        let _ = self.app.emit("cloud-sync-warning", message);
    }

    fn emit_conversation_deleted(&self, conversation_id: &str) {
        let _ = self.app.emit("cloud-conversation-deleted", conversation_id);
    }
}
