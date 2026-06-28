use agent_core::error::{Error, Result};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct CommandClient {
    client: reqwest::Client,
    api_base: String,
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CommandResponse {
    pub conversation_id: Option<String>,
    pub job_id: Option<String>,
    #[serde(default)]
    pub accepted: Option<bool>,
}

#[derive(Clone, Copy)]
pub enum UserMessageCommandType {
    StartRun,
    SendMessage,
}

impl UserMessageCommandType {
    fn as_wire_type(self) -> &'static str {
        match self {
            Self::StartRun => "start_run",
            Self::SendMessage => "send_message",
        }
    }
}

pub struct UserMessageCommand {
    pub command_type: UserMessageCommandType,
    pub command_id: String,
    pub conversation_id: String,
    pub text: String,
    pub attachments: Vec<Value>,
    pub tier_id: String,
}

pub struct CancelRunCommand {
    pub command_id: String,
    pub conversation_id: String,
    pub job_id: Option<String>,
}

pub struct ConfirmCommand {
    pub command_id: String,
    pub conversation_id: String,
    pub action_id: String,
    pub approved: bool,
    pub job_id: Option<String>,
    pub tier_id: String,
}

impl CommandClient {
    pub fn new(api_base: String, token: Option<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .build()
            .map_err(|error| Error::Transport(format!("command client build failed: {error}")))?;
        Ok(Self {
            client,
            api_base,
            token,
        })
    }

    pub async fn send_user_message(&self, command: UserMessageCommand) -> Result<CommandResponse> {
        self.post(user_message_body(&command)).await
    }

    pub async fn cancel_run(&self, command: CancelRunCommand) -> Result<CommandResponse> {
        self.post(cancel_run_body(&command)).await
    }

    pub async fn confirm(&self, command: ConfirmCommand) -> Result<CommandResponse> {
        self.post(confirm_body(&command)).await
    }

    async fn post(&self, body: Value) -> Result<CommandResponse> {
        let url = format!(
            "{}/api/conversation-sync/commands",
            self.api_base.trim_end_matches('/')
        );
        let mut request = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .body(body.to_string());
        if let Some(token) = self.token.as_deref() {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = request
            .send()
            .await
            .map_err(|error| Error::Transport(format!("command request failed: {error}")))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| Error::Transport(format!("command response read failed: {error}")))?;
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "conversation command failed ({status}): {text}"
            )));
        }
        serde_json::from_str(&text)
            .map_err(|error| Error::Protocol(format!("invalid command response: {error}")))
    }
}

fn user_message_body(command: &UserMessageCommand) -> Value {
    json!({
        "command_id": command.command_id,
        "type": command.command_type.as_wire_type(),
        "conversation_id": command.conversation_id,
        "text": command.text,
        "attachments": command.attachments,
        "tier_id": command.tier_id,
    })
}

fn cancel_run_body(command: &CancelRunCommand) -> Value {
    json!({
        "command_id": command.command_id,
        "type": "cancel_run",
        "conversation_id": command.conversation_id,
        "job_id": command.job_id,
    })
}

fn confirm_body(command: &ConfirmCommand) -> Value {
    json!({
        "command_id": command.command_id,
        "type": "confirm",
        "conversation_id": command.conversation_id,
        "action_id": command.action_id,
        "approved": command.approved,
        "job_id": command.job_id,
        "tier_id": command.tier_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_run_body_uses_canonical_command_envelope() {
        let body = user_message_body(&UserMessageCommand {
            command_type: UserMessageCommandType::StartRun,
            command_id: "request-1".into(),
            conversation_id: "conv-1".into(),
            text: "hello".into(),
            attachments: vec![json!({"filename": "note.txt"})],
            tier_id: "clark".into(),
        });

        assert_eq!(body["command_id"], "request-1");
        assert_eq!(body["type"], "start_run");
        assert_eq!(body["conversation_id"], "conv-1");
        assert_eq!(body["text"], "hello");
        assert_eq!(body["tier_id"], "clark");
        assert!(body.get("client_request_id").is_none());
        assert!(body.get("source").is_none());
    }

    #[test]
    fn follow_up_body_uses_send_message_command_type() {
        let body = user_message_body(&UserMessageCommand {
            command_type: UserMessageCommandType::SendMessage,
            command_id: "request-2".into(),
            conversation_id: "conv-1".into(),
            text: "follow up".into(),
            attachments: Vec::new(),
            tier_id: "clark".into(),
        });

        assert_eq!(body["command_id"], "request-2");
        assert_eq!(body["type"], "send_message");
        assert_eq!(body["conversation_id"], "conv-1");
        assert_eq!(body["text"], "follow up");
    }

    #[test]
    fn cancel_and_confirm_bodies_use_command_types() {
        let cancel = cancel_run_body(&CancelRunCommand {
            command_id: "cancel-1".into(),
            conversation_id: "conv-1".into(),
            job_id: Some("job-1".into()),
        });
        assert_eq!(cancel["type"], "cancel_run");
        assert_eq!(cancel["job_id"], "job-1");

        let confirm = confirm_body(&ConfirmCommand {
            command_id: "confirm-1".into(),
            conversation_id: "conv-1".into(),
            action_id: "action-1".into(),
            approved: true,
            job_id: None,
            tier_id: "clark".into(),
        });
        assert_eq!(confirm["type"], "confirm");
        assert_eq!(confirm["action_id"], "action-1");
        assert_eq!(confirm["approved"], true);
        assert_eq!(confirm["tier_id"], "clark");
    }
}
