use crate::root_execution::RootExecutionTrace;

/// Queue shared between `Provider::steer` and the active agent run. Leftovers
/// remain drainable when a terminal batch ends before injecting them.
pub(crate) struct EngineSteering {
    queue: std::sync::Mutex<std::collections::VecDeque<agent_loop::AgentMessage>>,
    execution: Option<RootExecutionTrace>,
}

impl Default for EngineSteering {
    fn default() -> Self {
        Self {
            queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
            execution: None,
        }
    }
}

impl EngineSteering {
    pub(super) fn with_execution(execution: RootExecutionTrace) -> Self {
        Self {
            execution: Some(execution),
            ..Self::default()
        }
    }

    pub fn push_user_text(&self, text: String) {
        if let Some(execution) = &self.execution {
            execution.steering();
        }
        self.queue
            .lock()
            .expect("steering queue lock")
            .push_back(agent_loop::AgentMessage::User {
                content: agent_loop::UserContent::Text(text),
                timestamp: None,
            });
    }

    pub(super) fn drain_all(&self) -> Vec<agent_loop::AgentMessage> {
        self.queue
            .lock()
            .expect("steering queue lock")
            .drain(..)
            .collect()
    }
}

impl agent_loop::Plugin for EngineSteering {
    fn name(&self) -> &'static str {
        "desktop_steering"
    }

    fn capabilities(&self) -> agent_loop::PluginCapabilities {
        agent_loop::PluginCapabilities::steering()
    }
}

#[async_trait::async_trait]
impl agent_loop::SteeringSource for EngineSteering {
    async fn next_steering_messages(&self) -> Vec<agent_loop::AgentMessage> {
        self.drain_all()
    }
}
