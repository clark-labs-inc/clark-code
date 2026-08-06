//! One account-scoped Clark Cloud conversation contract for native hosts.
//!
//! Desktop authenticates its route with a session JWT; the CLI and headless
//! workers use a Clark Platform API key. Those are two authentication front
//! doors onto the same server-side conversation rows, not separate history
//! systems. Both surfaces persist the same [`agent_core::Snapshot`].

use std::collections::HashMap;
use std::time::Duration;

use agent_core::Snapshot;
use reqwest::{Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialSurface {
    /// Clark Desktop's Better Auth session JWT routes.
    DesktopSession,
    /// Clark CLI/headless routes authenticated by a Platform API key.
    CliApiKey,
}

impl CredentialSurface {
    fn conversation_root(self) -> &'static str {
        match self {
            Self::DesktopSession => "/api/desktop/conversations",
            Self::CliApiKey => "/v1/cli/conversations",
        }
    }

    fn specialist_root(self) -> &'static str {
        match self {
            Self::DesktopSession => "/api/desktop/specialist-conversations",
            Self::CliApiKey => "/v1/cli/specialist-conversations",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecialistContext {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub study_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_fingerprint: Option<String>,
    pub rev: i64,
    #[serde(default)]
    pub change_rev: i64,
    #[serde(default)]
    pub archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default)]
    pub title_locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub created_at: Value,
    pub updated_at: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specialist_context: Option<SpecialistContext>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationDetail {
    #[serde(flatten)]
    pub summary: ConversationSummary,
    pub snapshot: Snapshot,
}

#[derive(Clone, Debug)]
pub struct ConversationWrite {
    pub id: String,
    pub title: String,
    pub provider: String,
    pub project: Option<String>,
    pub repository_fingerprint: Option<String>,
    pub remote_host: Option<String>,
    pub mode: Option<String>,
    pub title_locked: bool,
    pub specialist_context: Option<SpecialistContext>,
    pub rev: i64,
    pub snapshot: Snapshot,
    pub status: Option<String>,
    pub base_rev: Option<i64>,
    pub mutation_id: Option<Uuid>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid Clark Cloud endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("Clark Cloud transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("Clark Cloud rejected the conversation request ({status}): {body}")]
    Http { status: u16, body: String },
    #[error("Clark Cloud returned an unreadable conversation: {0}")]
    InvalidResponse(String),
}

impl Error {
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Http { status, .. } => Some(*status),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub struct ConversationClient {
    http: reqwest::Client,
    base: String,
    credential: Zeroizing<String>,
    surface: CredentialSurface,
}

impl ConversationClient {
    pub fn new(
        base: &str,
        credential: impl Into<String>,
        surface: CredentialSurface,
        user_agent: &str,
    ) -> Result<Self> {
        let base = validated_base(base)?;
        let http = clark_http::build_client(clark_http::ClientOptions {
            request_timeout: Some(REQUEST_TIMEOUT),
            user_agent: Some(user_agent),
            ..Default::default()
        })?;
        Ok(Self {
            http,
            base,
            credential: Zeroizing::new(credential.into()),
            surface,
        })
    }

    pub async fn list(&self) -> Result<Vec<ConversationSummary>> {
        let response = self
            .authorized(self.http.get(self.url(self.surface.conversation_root())))
            .send()
            .await?;
        let mut conversations: Vec<ConversationSummary> = read_json(response).await?;
        let specialists = self.list_specialists().await?;
        for conversation in &mut conversations {
            conversation.specialist_context = specialists.get(&conversation.id).cloned();
        }
        Ok(conversations)
    }

    pub async fn get(&self, id: &str) -> Result<ConversationDetail> {
        let response = self
            .authorized(
                self.http
                    .get(self.item_url(self.surface.conversation_root(), id)),
            )
            .send()
            .await?;
        let value: Value = read_json(response).await?;
        let normalized = normalize_detail_snapshot(value);
        let mut detail: ConversationDetail = serde_json::from_value(normalized)
            .map_err(|error| Error::InvalidResponse(error.to_string()))?;
        detail.summary.specialist_context = self.get_specialist(id).await?;
        Ok(detail)
    }

    pub async fn put(&self, write: &ConversationWrite) -> Result<ConversationSummary> {
        let response = self
            .authorized(
                self.http
                    .put(self.item_url(self.surface.conversation_root(), &write.id)),
            )
            .json(&ConversationWriteBody::from(write))
            .send()
            .await?;
        let mut summary: ConversationSummary = read_json(response).await?;
        if let Some(context) = &write.specialist_context {
            self.put_specialist(&write.id, context).await?;
            summary.specialist_context = Some(context.clone());
        }
        Ok(summary)
    }

    pub async fn set_archived(&self, id: &str, archived: bool) -> Result<ConversationSummary> {
        let response = self
            .authorized(
                self.http
                    .patch(self.item_url(self.surface.conversation_root(), id)),
            )
            .json(&serde_json::json!({ "archived": archived }))
            .send()
            .await?;
        read_json(response).await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let response = self
            .authorized(
                self.http
                    .delete(self.item_url(self.surface.conversation_root(), id)),
            )
            .send()
            .await?;
        ensure_success(response).await.map(|_| ())
    }

    fn authorized(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request.bearer_auth(self.credential.as_str())
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn item_url(&self, root: &str, id: &str) -> String {
        format!("{}{root}/{}", self.base, urlencoding::encode(id))
    }

    async fn list_specialists(&self) -> Result<HashMap<String, SpecialistContext>> {
        let response = self
            .authorized(self.http.get(self.url(self.surface.specialist_root())))
            .send()
            .await?;
        let rows: Vec<SpecialistRow> = read_json(response).await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.conversation_id, row.context))
            .collect())
    }

    async fn get_specialist(&self, id: &str) -> Result<Option<SpecialistContext>> {
        let response = self
            .authorized(
                self.http
                    .get(self.item_url(self.surface.specialist_root(), id)),
            )
            .send()
            .await?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let row: SpecialistRow = read_json(response).await?;
        Ok(Some(row.context))
    }

    async fn put_specialist(&self, id: &str, context: &SpecialistContext) -> Result<()> {
        let response = self
            .authorized(
                self.http
                    .put(self.item_url(self.surface.specialist_root(), id)),
            )
            .json(&serde_json::json!({ "context": context }))
            .send()
            .await?;
        ensure_success(response).await.map(|_| ())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversationWriteBody<'a> {
    title: &'a str,
    provider: &'a str,
    project: &'a Option<String>,
    repository_fingerprint: &'a Option<String>,
    remote_host: &'a Option<String>,
    mode: &'a Option<String>,
    title_locked: bool,
    rev: i64,
    snapshot: &'a Snapshot,
    status: &'a Option<String>,
    base_rev: Option<i64>,
    mutation_id: Option<Uuid>,
}

impl<'a> From<&'a ConversationWrite> for ConversationWriteBody<'a> {
    fn from(write: &'a ConversationWrite) -> Self {
        Self {
            title: &write.title,
            provider: &write.provider,
            project: &write.project,
            repository_fingerprint: &write.repository_fingerprint,
            remote_host: &write.remote_host,
            mode: &write.mode,
            title_locked: write.title_locked,
            rev: write.rev,
            snapshot: &write.snapshot,
            status: &write.status,
            base_rev: write.base_rev,
            mutation_id: write.mutation_id,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpecialistRow {
    conversation_id: String,
    context: SpecialistContext,
}

fn normalize_detail_snapshot(mut detail: Value) -> Value {
    if let Some(snapshot) = detail.get_mut("snapshot") {
        *snapshot = agent_core::normalize_snapshot_value(std::mem::take(snapshot));
    }
    detail
}

fn validated_base(raw: &str) -> Result<String> {
    let mut url = Url::parse(raw).map_err(|error| Error::InvalidEndpoint(error.to_string()))?;
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() {
        return Err(Error::InvalidEndpoint(
            "credentials and query strings are not allowed".into(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| Error::InvalidEndpoint("host is missing".into()))?;
    let loopback = host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1";
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(Error::InvalidEndpoint(
            "HTTPS is required except for loopback development".into(),
        ));
    }
    url.set_fragment(None);
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

async fn read_json<T: for<'de> Deserialize<'de>>(response: Response) -> Result<T> {
    let response = ensure_success(response).await?;
    response
        .json::<T>()
        .await
        .map_err(|error| Error::InvalidResponse(error.to_string()))
}

async fn ensure_success(response: Response) -> Result<Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    Err(Error::Http { status, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_surfaces_share_shapes_but_not_credential_routes() {
        assert_eq!(
            CredentialSurface::DesktopSession.conversation_root(),
            "/api/desktop/conversations"
        );
        assert_eq!(
            CredentialSurface::CliApiKey.conversation_root(),
            "/v1/cli/conversations"
        );
        assert_eq!(
            CredentialSurface::DesktopSession.specialist_root(),
            "/api/desktop/specialist-conversations"
        );
        assert_eq!(
            CredentialSurface::CliApiKey.specialist_root(),
            "/v1/cli/specialist-conversations"
        );
    }

    #[test]
    fn cloud_base_rejects_remote_plaintext_and_embedded_credentials() {
        assert!(validated_base("http://api.example.com").is_err());
        assert!(validated_base("https://user:secret@api.example.com").is_err());
        assert_eq!(
            validated_base("http://127.0.0.1:8787/").unwrap(),
            "http://127.0.0.1:8787"
        );
    }

    #[test]
    fn legacy_snapshot_is_normalized_before_typed_cloud_decode() {
        let value = serde_json::json!({
            "id": "conversation-1",
            "title": "Experiment",
            "provider": "specialist",
            "rev": 2,
            "changeRev": 3,
            "archived": false,
            "titleLocked": false,
            "createdAt": "2026-08-01T00:00:00Z",
            "updatedAt": "2026-08-01T00:01:00Z",
            "snapshot": {
                "runs": {},
                "timeline": [{"item": "plan", "plan": {}}],
                "tool_calls": {},
                "artifacts": []
            }
        });
        let detail: ConversationDetail =
            serde_json::from_value(normalize_detail_snapshot(value)).unwrap();
        assert!(matches!(
            detail.snapshot.timeline.first(),
            Some(agent_core::TimelineItem::ExecutionChecklist { .. })
        ));
    }
}
