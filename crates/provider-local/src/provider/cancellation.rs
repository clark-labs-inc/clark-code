use super::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

/// Run-addressed cancellation. A provider can have overlapping prompt tasks,
/// so a single "most recently assigned" token cannot implement
/// `Provider::cancel(session, run)` correctly.
#[derive(Clone, Default)]
pub(crate) struct RunCancellationRegistry {
    tokens: Arc<std::sync::Mutex<HashMap<String, CancellationToken>>>,
}

impl RunCancellationRegistry {
    pub(super) fn register(&self, run: &RunId, token: CancellationToken) {
        self.tokens
            .lock()
            .expect("run cancellation registry lock")
            .insert(run.as_str().to_string(), token);
    }

    pub(super) fn cancel(&self, run: &RunId) -> bool {
        let token = self
            .tokens
            .lock()
            .expect("run cancellation registry lock")
            .get(run.as_str())
            .cloned();
        if let Some(token) = token {
            token.cancel();
            true
        } else {
            false
        }
    }

    pub(crate) fn remove(&self, run: &RunId) {
        self.tokens
            .lock()
            .expect("run cancellation registry lock")
            .remove(run.as_str());
    }

    pub(super) fn has_active(&self) -> bool {
        !self
            .tokens
            .lock()
            .expect("run cancellation registry lock")
            .is_empty()
    }

    pub(super) fn cancel_all(&self) {
        let tokens = self
            .tokens
            .lock()
            .expect("run cancellation registry lock")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for token in tokens {
            token.cancel();
        }
    }
}

pub(super) struct ManualCompactionRegistration {
    pub(super) registry: RunCancellationRegistry,
    pub(super) run: RunId,
    pub(super) latch: Arc<AtomicBool>,
}

impl Drop for ManualCompactionRegistration {
    fn drop(&mut self) {
        self.registry.remove(&self.run);
        self.latch.store(false, Ordering::Release);
    }
}
