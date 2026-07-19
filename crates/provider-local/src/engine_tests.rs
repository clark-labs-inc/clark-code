mod tests {
    use super::*;
    use clark_agent::SteeringSource;

    #[tokio::test]
    async fn steering_queue_injects_in_order_and_recovers_leftovers() {
        let steering = EngineSteering::default();
        steering.push_user_text("first".into());
        steering.push_user_text("second".into());

        // The loop drains via the SteeringSource seam…
        let drained = steering.next_steering_messages().await;
        assert_eq!(drained.len(), 2);
        let texts: Vec<_> = drained
            .iter()
            .map(|m| match m {
                clark_agent::AgentMessage::User {
                    content: clark_agent::UserContent::Text(t),
                    ..
                } => t.as_str(),
                other => panic!("expected user text, got {other:?}"),
            })
            .collect();
        assert_eq!(texts, vec!["first", "second"]);

        // …and anything left after the run ends is recoverable, not lost.
        steering.push_user_text("too late".into());
        assert_eq!(steering.drain_all().len(), 1);
        assert!(steering.drain_all().is_empty());
    }
}
