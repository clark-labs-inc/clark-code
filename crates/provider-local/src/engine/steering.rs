use crate::root_execution::RootExecutionTrace;

/// Queue shared between `Provider::steer` and the active agent run. Leftovers
/// remain drainable when a terminal batch ends before injecting them.
pub(crate) struct EngineSteering {
    queue: std::sync::Mutex<std::collections::VecDeque<clark_agent::AgentMessage>>,
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
        self.queue.lock().expect("steering queue lock").push_back(
            clark_agent::AgentMessage::User {
                content: clark_agent::UserContent::Text(text),
                timestamp: None,
            },
        );
    }

    pub(super) fn drain_all(&self) -> Vec<clark_agent::AgentMessage> {
        self.queue
            .lock()
            .expect("steering queue lock")
            .drain(..)
            .collect()
    }
}

impl clark_agent::Plugin for EngineSteering {
    fn name(&self) -> &'static str {
        "desktop_steering"
    }

    fn capabilities(&self) -> clark_agent::PluginCapabilities {
        clark_agent::PluginCapabilities::steering()
    }
}

#[async_trait::async_trait]
impl clark_agent::SteeringSource for EngineSteering {
    async fn next_steering_messages(&self) -> Vec<clark_agent::AgentMessage> {
        self.drain_all()
    }
}
