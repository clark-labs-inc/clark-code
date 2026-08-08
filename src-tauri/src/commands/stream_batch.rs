use std::time::Duration;

use agent_core::AgentEvent;
use futures::{Stream, StreamExt};
use tokio::time::{timeout_at, Instant};

pub(super) const MAX_EVENT_BATCH: usize = 64;
pub(super) const EVENT_BATCH_WINDOW: Duration = Duration::from_millis(16);

/// Collect one bounded provider batch.
///
/// `ready_chunks` flushes as soon as the currently-ready queue is empty, which
/// turns a token stream into one SQLite transaction, full snapshot clone, and
/// WebView IPC payload per network frame. Waiting for at most one display frame
/// keeps the write-before-render durability boundary while amortizing that
/// fixed work. Terminal events still flush with the batch that contains them.
pub(super) async fn next_event_batch<S>(stream: &mut S) -> Option<Vec<AgentEvent>>
where
    S: Stream<Item = AgentEvent> + Unpin,
{
    let first = stream.next().await?;
    let mut events = Vec::with_capacity(MAX_EVENT_BATCH);
    events.push(first);
    let deadline = Instant::now() + EVENT_BATCH_WINDOW;

    while events.len() < MAX_EVENT_BATCH {
        match timeout_at(deadline, stream.next()).await {
            Ok(Some(event)) => {
                let terminal = matches!(event, AgentEvent::RunFinished { .. });
                events.push(event);
                if terminal {
                    break;
                }
            }
            Ok(None) | Err(_) => break,
        }
    }

    Some(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{RunId, RunOutcome, RunStatus};
    use futures::stream;

    fn started(index: usize) -> AgentEvent {
        AgentEvent::RunStarted {
            run: RunId::new(format!("run-{index}")),
        }
    }

    #[tokio::test]
    async fn caps_immediately_ready_events() {
        let mut input = stream::iter((0..MAX_EVENT_BATCH + 5).map(started));
        let first = next_event_batch(&mut input).await.unwrap();
        let second = next_event_batch(&mut input).await.unwrap();
        assert_eq!(first.len(), MAX_EVENT_BATCH);
        assert_eq!(second.len(), 5);
    }

    #[tokio::test]
    async fn terminal_event_flushes_without_consuming_a_later_run() {
        let terminal = AgentEvent::RunFinished {
            run: RunId::new("run-0"),
            outcome: RunOutcome {
                status: RunStatus::Done,
                stop_reason: None,
                error: None,
                failure_kind: None,
                usage: None,
                execution: None,
            },
        };
        let mut input = stream::iter([started(0), terminal, started(1)]);
        let first = next_event_batch(&mut input).await.unwrap();
        let second = next_event_batch(&mut input).await.unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 1);
    }
}
