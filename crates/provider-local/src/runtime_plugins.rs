//! Product-owned runtime plugin bundles for the canonical Clark agent loop.
//!
//! Tool schemas remain in [`crate::tools::ToolPack`]. This companion seam owns
//! message ingress and event egress, allowing a downstream product to attach a
//! durable mailbox without teaching the neutral provider its protocol.

use std::sync::Arc;

pub use agent_loop::{
    AgentEvent as RuntimeAgentEvent, AgentMessage as RuntimeAgentMessage,
    EventSink as RuntimeEventSink, FollowUpSource as RuntimeFollowUpSource,
    Plugin as RuntimePlugin, PluginCapabilities as RuntimePluginCapabilities,
    SteeringSource as RuntimeSteeringSource, UserContent as RuntimeUserContent,
};

pub trait RuntimePluginPack: Send + Sync {
    fn id(&self) -> &str;

    fn steering_sources(&self) -> Vec<Arc<dyn RuntimeSteeringSource>> {
        Vec::new()
    }

    fn follow_up_sources(&self) -> Vec<Arc<dyn RuntimeFollowUpSource>> {
        Vec::new()
    }

    fn event_sinks(&self) -> Vec<Arc<dyn RuntimeEventSink>> {
        Vec::new()
    }
}

pub(crate) struct CompositeEventSink {
    sinks: Vec<Arc<dyn RuntimeEventSink>>,
}

impl CompositeEventSink {
    pub(crate) fn new(sinks: Vec<Arc<dyn RuntimeEventSink>>) -> Self {
        Self { sinks }
    }
}

#[async_trait::async_trait]
impl RuntimeEventSink for CompositeEventSink {
    async fn emit(&self, event: RuntimeAgentEvent) {
        for sink in &self.sinks {
            sink.emit(event.clone()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct CaptureSink(Arc<Mutex<Vec<RuntimeAgentEvent>>>);

    #[async_trait::async_trait]
    impl RuntimeEventSink for CaptureSink {
        async fn emit(&self, event: RuntimeAgentEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[tokio::test]
    async fn composite_event_sink_forwards_the_same_typed_event_to_every_owner() {
        let first = Arc::new(Mutex::new(Vec::new()));
        let second = Arc::new(Mutex::new(Vec::new()));
        let sink = CompositeEventSink::new(vec![
            Arc::new(CaptureSink(first.clone())),
            Arc::new(CaptureSink(second.clone())),
        ]);
        sink.emit(RuntimeAgentEvent::AgentStart).await;
        assert_eq!(first.lock().unwrap().len(), 1);
        assert_eq!(second.lock().unwrap().len(), 1);
    }
}
