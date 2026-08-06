//! Prompt admission owns the ordering between provider validation and the
//! append-only conversation journal. Keeping both operations behind one helper
//! makes "rejected prompts are never durable" a directly testable contract.

use std::future::Future;
use std::pin::Pin;

use agent_core::{AgentEvent, PromptInput, Provider, SessionId};

use crate::trajectory::CloudTrajectoryClient;

pub(super) trait PromptJournal {
    fn append_prompt<'a>(
        &'a self,
        events: &'a [AgentEvent],
    ) -> Pin<Box<dyn Future<Output = Result<i64, String>> + Send + 'a>>;
}

impl PromptJournal for CloudTrajectoryClient {
    fn append_prompt<'a>(
        &'a self,
        events: &'a [AgentEvent],
    ) -> Pin<Box<dyn Future<Output = Result<i64, String>> + Send + 'a>> {
        Box::pin(async move { self.append(events).await })
    }
}

pub(super) async fn admit_and_append_prompt<J: PromptJournal + Sync>(
    provider: &dyn Provider,
    session: &SessionId,
    input: &PromptInput,
    journal: &J,
    durable_prompt: &[AgentEvent],
) -> Result<i64, String> {
    provider
        .validate_prompt(session, input)
        .await
        .map_err(|error| error.to_string())?;
    journal.append_prompt(durable_prompt).await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use agent_core::provider::EventStream;
    use agent_core::{
        ClientResponse, Error, ProviderCapabilities, ProviderConfig, ProviderId, RunId, Session,
        SessionOptions,
    };
    use async_trait::async_trait;
    use futures::stream::{self, StreamExt};

    use super::*;

    struct AdmissionProvider {
        rejection: Option<&'static str>,
    }

    #[async_trait]
    impl Provider for AdmissionProvider {
        fn id(&self) -> ProviderId {
            ProviderId::new("prompt-admission-test")
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::default()
        }

        async fn validate_prompt(
            &self,
            _session: &SessionId,
            _input: &PromptInput,
        ) -> agent_core::Result<()> {
            match self.rejection {
                Some(message) => Err(Error::Other(message.into())),
                None => Ok(()),
            }
        }

        async fn connect(&mut self, _config: ProviderConfig) -> agent_core::Result<()> {
            Ok(())
        }

        async fn new_session(&mut self, _options: SessionOptions) -> agent_core::Result<Session> {
            Err(Error::Unsupported("not used by this test".into()))
        }

        async fn load_session(&mut self, _id: SessionId) -> agent_core::Result<Session> {
            Err(Error::Unsupported("not used by this test".into()))
        }

        async fn prompt(
            &mut self,
            _session: &SessionId,
            _input: PromptInput,
        ) -> agent_core::Result<EventStream> {
            Ok(stream::empty().boxed())
        }

        async fn cancel(&mut self, _session: &SessionId, _run: &RunId) -> agent_core::Result<()> {
            Ok(())
        }

        async fn respond(
            &mut self,
            _session: &SessionId,
            _response: ClientResponse,
        ) -> agent_core::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingJournal {
        appends: AtomicUsize,
        events: Mutex<Vec<AgentEvent>>,
    }

    impl PromptJournal for RecordingJournal {
        fn append_prompt<'a>(
            &'a self,
            events: &'a [AgentEvent],
        ) -> Pin<Box<dyn Future<Output = Result<i64, String>> + Send + 'a>> {
            Box::pin(async move {
                self.appends.fetch_add(1, Ordering::SeqCst);
                self.events.lock().unwrap().extend_from_slice(events);
                Ok(41)
            })
        }
    }

    fn durable_prompt() -> Vec<AgentEvent> {
        vec![AgentEvent::MessageChunk {
            run: RunId::new("user"),
            role: agent_core::Role::User,
            delta: agent_core::ContentBlock::text("/goal finish the migration"),
        }]
    }

    #[tokio::test]
    async fn rejected_prompt_never_reaches_the_durable_journal() {
        let provider = AdmissionProvider {
            rejection: Some(
                "an unfinished goal already exists (blocked): finish it with update_goal",
            ),
        };
        let journal = RecordingJournal::default();

        let error = admit_and_append_prompt(
            &provider,
            &SessionId::new("session-1"),
            &PromptInput::text("/goal finish the migration"),
            &journal,
            &durable_prompt(),
        )
        .await
        .unwrap_err();

        assert!(error.contains("unfinished goal"));
        assert_eq!(journal.appends.load(Ordering::SeqCst), 0);
        assert!(journal.events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn admitted_prompt_is_journaled_exactly_once() {
        let provider = AdmissionProvider { rejection: None };
        let journal = RecordingJournal::default();
        let events = durable_prompt();

        let checkpoint = admit_and_append_prompt(
            &provider,
            &SessionId::new("session-1"),
            &PromptInput::text("/goal finish the migration"),
            &journal,
            &events,
        )
        .await
        .unwrap();

        assert_eq!(checkpoint, 41);
        assert_eq!(journal.appends.load(Ordering::SeqCst), 1);
        let recorded = journal.events.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert!(matches!(
            &recorded[0],
            AgentEvent::MessageChunk {
                role: agent_core::Role::User,
                ..
            }
        ));
    }
}
