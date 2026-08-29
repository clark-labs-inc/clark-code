use agent_core::{AgentEvent, ContentBlock, Role, RunId};

/// User turns are not owned by a provider run because they are journaled
/// before the provider allocates one. Keep the synthetic run identity for
/// compatibility, but bracket every accepted turn so adjacent prompts cannot
/// be mistaken for streaming chunks of one message.
const USER_RUN: &str = "user";

pub(super) fn user_message_events(blocks: &[ContentBlock]) -> Vec<AgentEvent> {
    if blocks.is_empty() {
        return Vec::new();
    }

    let run = RunId::new(USER_RUN);
    let mut events = Vec::with_capacity(blocks.len() + 1);
    events.push(AgentEvent::MessageStreamStarted {
        run: run.clone(),
        role: Role::User,
    });
    events.extend(
        blocks
            .iter()
            .cloned()
            .map(|delta| AgentEvent::MessageChunk {
                run: run.clone(),
                role: Role::User,
                delta,
            }),
    );
    events
}

#[cfg(test)]
mod tests {
    use agent_core::{apply, Snapshot, TimelineItem};

    use super::*;

    #[test]
    fn adjacent_user_prompts_remain_distinct_after_replay() {
        let mut snapshot = Snapshot::new();
        let events = user_message_events(&[
            ContentBlock::text("what it means \"sealed\""),
            ContentBlock::ResourceLink {
                uri: "attachment://receipt".into(),
                name: Some("receipt".into()),
            },
        ])
        .into_iter()
        .chain(user_message_events(&[ContentBlock::text("stop")]))
        .collect::<Vec<_>>();

        for event in &events {
            apply(&mut snapshot, event);
        }

        let messages = snapshot
            .timeline
            .iter()
            .filter_map(|item| match item {
                TimelineItem::Message {
                    role: Role::User,
                    blocks,
                    ..
                } => Some(blocks),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0],
            &vec![
                ContentBlock::text("what it means \"sealed\""),
                ContentBlock::ResourceLink {
                    uri: "attachment://receipt".into(),
                    name: Some("receipt".into()),
                },
            ]
        );
        assert_eq!(messages[1], &vec![ContentBlock::text("stop")]);
    }
}
