use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::{CancelAck, ComputerUseError};

const CANCEL_QUIESCE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LEASE_DURATION: Duration = Duration::from_secs(5);

#[derive(Clone, Default)]
pub(crate) struct InputLeaseCoordinator {
    shared: Arc<(Mutex<LeaseState>, Condvar)>,
}

#[derive(Default)]
struct LeaseState {
    active: Option<ActiveLease>,
}

struct ActiveLease {
    id: String,
    cancelled: bool,
    user_takeover: bool,
}

pub(crate) struct InputLease {
    coordinator: InputLeaseCoordinator,
    id: String,
    started: Instant,
}

impl InputLeaseCoordinator {
    pub fn begin(&self) -> Result<InputLease, ComputerUseError> {
        let (lock, _) = self.shared.as_ref();
        let mut state = lock
            .lock()
            .map_err(|_| ComputerUseError::Os("input lease lock poisoned".to_string()))?;
        if state.active.is_some() {
            return Err(ComputerUseError::RateLimited);
        }
        let sequence = LEASE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("lease-{}-{sequence}", std::process::id());
        state.active = Some(ActiveLease {
            id: id.clone(),
            cancelled: false,
            user_takeover: false,
        });
        Ok(InputLease {
            coordinator: self.clone(),
            id,
            started: Instant::now(),
        })
    }

    pub fn cancel_active(&self) -> Result<CancelAck, ComputerUseError> {
        let (lock, condition) = self.shared.as_ref();
        let mut state = lock
            .lock()
            .map_err(|_| ComputerUseError::Os("input lease lock poisoned".to_string()))?;
        let Some(id) = state.active.as_mut().map(|active| {
            active.cancelled = true;
            active.id.clone()
        }) else {
            return Ok(CancelAck {
                lease_id: None,
                quiesced: true,
                helper_terminated: false,
            });
        };
        let deadline = Instant::now() + CANCEL_QUIESCE_TIMEOUT;
        while state.active.as_ref().is_some_and(|active| active.id == id) {
            let now = Instant::now();
            if now >= deadline {
                return Ok(CancelAck {
                    lease_id: Some(id),
                    quiesced: false,
                    helper_terminated: false,
                });
            }
            let wait = deadline.saturating_duration_since(now);
            let (next, _) = condition
                .wait_timeout(state, wait)
                .map_err(|_| ComputerUseError::Os("input lease lock poisoned".to_string()))?;
            state = next;
        }
        Ok(CancelAck {
            lease_id: Some(id),
            quiesced: true,
            helper_terminated: false,
        })
    }

    #[cfg(any(feature = "helper-service", test))]
    pub fn mark_user_takeover(&self) {
        if let Ok(mut state) = self.shared.0.lock() {
            if let Some(active) = state.active.as_mut() {
                active.user_takeover = true;
            }
        }
    }

    #[cfg(test)]
    pub fn has_active(&self) -> bool {
        self.shared
            .0
            .lock()
            .map(|state| state.active.is_some())
            .unwrap_or(false)
    }
}

impl InputLease {
    pub fn check(&self) -> Result<(), ComputerUseError> {
        if self.started.elapsed() > MAX_LEASE_DURATION {
            return Err(ComputerUseError::InputCancelled);
        }
        let state = self
            .coordinator
            .shared
            .0
            .lock()
            .map_err(|_| ComputerUseError::Os("input lease lock poisoned".to_string()))?;
        let active = state
            .active
            .as_ref()
            .filter(|active| active.id == self.id)
            .ok_or(ComputerUseError::InputCancelled)?;
        if active.user_takeover {
            return Err(ComputerUseError::UserTakeover);
        }
        if active.cancelled {
            return Err(ComputerUseError::InputCancelled);
        }
        Ok(())
    }
}

impl Drop for InputLease {
    fn drop(&mut self) {
        let (lock, condition) = self.coordinator.shared.as_ref();
        if let Ok(mut state) = lock.lock() {
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.id == self.id)
            {
                state.active = None;
                condition.notify_all();
            }
        }
    }
}

static LEASE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_ack_waits_for_lease_drop() {
        let coordinator = InputLeaseCoordinator::default();
        let lease = coordinator.begin().unwrap();
        let cancel = coordinator.clone();
        let waiter = std::thread::spawn(move || cancel.cancel_active().unwrap());
        std::thread::sleep(Duration::from_millis(20));
        assert!(!waiter.is_finished());
        assert!(matches!(
            lease.check(),
            Err(ComputerUseError::InputCancelled)
        ));
        drop(lease);
        let ack = waiter.join().unwrap();
        assert!(ack.quiesced);
    }

    #[test]
    fn physical_takeover_stops_the_active_lease() {
        let coordinator = InputLeaseCoordinator::default();
        let lease = coordinator.begin().unwrap();
        coordinator.mark_user_takeover();
        assert!(matches!(lease.check(), Err(ComputerUseError::UserTakeover)));
    }
}
