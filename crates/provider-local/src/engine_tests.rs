mod tests {
    use super::*;
    use agent_loop::SteeringSource;

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
                agent_loop::AgentMessage::User {
                    content: agent_loop::UserContent::Text(t),
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

    #[test]
    fn failed_goal_iterations_preserve_usage_and_elapsed_time() {
        let mut session = crate::loop_state::SessionState::default();
        crate::tools::goal::start_goal(&mut session, "finish the migration".into()).unwrap();
        let goal = session.goal.as_mut().unwrap();

        account_goal_iteration(goal, 12_500, 47);
        account_goal_iteration(goal, 750, 3);

        assert_eq!(goal.tokens_used, 13_250);
        assert_eq!(goal.time_used_seconds, 50);
    }
}
