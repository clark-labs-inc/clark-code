use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::ComputerUseError;

const MAX_CLIENT_CAPABILITIES: usize = 64;

#[derive(Clone, Default)]
pub(super) struct ActionGate {
    shared: Arc<(Mutex<ActionGateState>, Condvar)>,
}

#[derive(Default)]
struct ActionGateState {
    generation: u64,
    observations: HashSet<String>,
    prepared: HashSet<String>,
    active: bool,
}

pub(super) struct ActionGateGuard {
    gate: ActionGate,
}

impl ActionGate {
    pub(super) fn generation(&self) -> Result<u64, ComputerUseError> {
        self.shared
            .0
            .lock()
            .map(|state| state.generation)
            .map_err(|_| poisoned())
    }

    pub(super) fn register_observation(
        &self,
        generation: u64,
        observation_id: &str,
    ) -> Result<bool, ComputerUseError> {
        let mut state = self.shared.0.lock().map_err(|_| poisoned())?;
        if state.generation != generation {
            return Ok(false);
        }
        insert_bounded(&mut state.observations, observation_id);
        Ok(true)
    }

    pub(super) fn consume_observation(
        &self,
        generation: u64,
        observation_id: &str,
    ) -> Result<(), ComputerUseError> {
        let mut state = self.shared.0.lock().map_err(|_| poisoned())?;
        if state.generation != generation || !state.observations.remove(observation_id) {
            return Err(ComputerUseError::ObservationStale);
        }
        Ok(())
    }

    pub(super) fn register_prepared(
        &self,
        generation: u64,
        prepared_id: &str,
    ) -> Result<bool, ComputerUseError> {
        let mut state = self.shared.0.lock().map_err(|_| poisoned())?;
        if state.generation != generation {
            return Ok(false);
        }
        insert_bounded(&mut state.prepared, prepared_id);
        Ok(true)
    }

    pub(super) fn begin_prepared(
        &self,
        prepared_id: &str,
    ) -> Result<ActionGateGuard, ComputerUseError> {
        self.begin(|state| state.prepared.remove(prepared_id))
            .map_err(|error| {
                if matches!(error, ComputerUseError::ObservationStale) {
                    ComputerUseError::PreparedActionNotFound(prepared_id.to_string())
                } else {
                    error
                }
            })
    }

    pub(super) fn begin_observation(
        &self,
        observation_id: &str,
    ) -> Result<ActionGateGuard, ComputerUseError> {
        self.begin(|state| state.observations.remove(observation_id))
    }

    fn begin(
        &self,
        consume: impl FnOnce(&mut ActionGateState) -> bool,
    ) -> Result<ActionGateGuard, ComputerUseError> {
        let mut state = self.shared.0.lock().map_err(|_| poisoned())?;
        if state.active {
            return Err(ComputerUseError::RateLimited);
        }
        if !consume(&mut state) {
            return Err(ComputerUseError::ObservationStale);
        }
        state.active = true;
        Ok(ActionGateGuard { gate: self.clone() })
    }

    pub(super) fn cancel(&self) -> Result<bool, ComputerUseError> {
        let mut state = self.shared.0.lock().map_err(|_| poisoned())?;
        state.generation = if state.generation == u64::MAX {
            1
        } else {
            state.generation + 1
        };
        state.observations.clear();
        state.prepared.clear();
        Ok(state.active)
    }

    pub(super) fn is_active(&self) -> Result<bool, ComputerUseError> {
        self.shared
            .0
            .lock()
            .map(|state| state.active)
            .map_err(|_| poisoned())
    }

    pub(super) fn wait_inactive(&self, timeout: Duration) -> Result<bool, ComputerUseError> {
        let deadline = Instant::now() + timeout;
        let (lock, condition) = self.shared.as_ref();
        let mut state = lock.lock().map_err(|_| poisoned())?;
        while state.active {
            let now = Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            let (next, _) = condition
                .wait_timeout(state, deadline.saturating_duration_since(now))
                .map_err(|_| poisoned())?;
            state = next;
        }
        Ok(true)
    }
}

impl Drop for ActionGateGuard {
    fn drop(&mut self) {
        let (lock, condition) = self.gate.shared.as_ref();
        if let Ok(mut state) = lock.lock() {
            state.active = false;
            condition.notify_all();
        }
    }
}

fn insert_bounded(values: &mut HashSet<String>, value: &str) {
    if values.len() >= MAX_CLIENT_CAPABILITIES {
        if let Some(oldest) = values.iter().next().cloned() {
            values.remove(&oldest);
        }
    }
    values.insert(value.to_string());
}

fn poisoned() -> ComputerUseError {
    ComputerUseError::HelperUnavailable("computer-use action gate was poisoned".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_invalidates_late_and_existing_capabilities() {
        let gate = ActionGate::default();
        let generation = gate.generation().unwrap();
        assert!(gate
            .register_observation(generation, "observation-before-cancel")
            .unwrap());
        assert!(!gate.cancel().unwrap());
        assert!(!gate
            .register_prepared(generation, "late-prepared-action")
            .unwrap());
        assert!(matches!(
            gate.begin_observation("observation-before-cancel"),
            Err(ComputerUseError::ObservationStale)
        ));
    }

    #[test]
    fn quiescence_waits_for_the_registered_action_guard() {
        let gate = ActionGate::default();
        let generation = gate.generation().unwrap();
        assert!(gate
            .register_prepared(generation, "prepared-action")
            .unwrap());
        let action = gate.begin_prepared("prepared-action").unwrap();
        assert!(gate.cancel().unwrap());

        let waiter_gate = gate.clone();
        let waiter =
            std::thread::spawn(move || waiter_gate.wait_inactive(Duration::from_secs(1)).unwrap());
        std::thread::sleep(Duration::from_millis(20));
        assert!(!waiter.is_finished());
        drop(action);
        assert!(waiter.join().unwrap());
    }
}
