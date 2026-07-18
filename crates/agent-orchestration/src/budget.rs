use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetConfig {
    pub limit_weighted_tokens: u64,
    pub max_cost_usd: Option<f64>,
    pub non_cached_input_weight: f64,
    pub cached_input_weight: f64,
    pub output_weight: f64,
    pub reminder_at_remaining_tokens: Vec<u64>,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            limit_weighted_tokens: 120_000,
            max_cost_usd: None,
            non_cached_input_weight: 1.0,
            cached_input_weight: 0.1,
            output_weight: 4.0,
            reminder_at_remaining_tokens: vec![40_000, 15_000, 5_000],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageCharge {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BudgetSnapshot {
    pub weighted_tokens_used: f64,
    pub weighted_tokens_reserved: f64,
    pub cost_usd: f64,
    pub exhausted: bool,
}

struct State {
    snapshot: BudgetSnapshot,
    delivered_reminders: BTreeSet<(String, usize)>,
}

#[derive(Clone)]
pub struct SharedBudget {
    config: BudgetConfig,
    state: Arc<Mutex<State>>,
}

pub struct BudgetReservation {
    budget: SharedBudget,
    weighted_tokens: f64,
    active: bool,
}

impl SharedBudget {
    pub fn new(config: BudgetConfig) -> Result<Self, String> {
        if config.limit_weighted_tokens == 0 {
            return Err("token budget must be greater than zero".to_string());
        }
        for (name, value) in [
            ("non_cached_input_weight", config.non_cached_input_weight),
            ("cached_input_weight", config.cached_input_weight),
            ("output_weight", config.output_weight),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be finite and non-negative"));
            }
        }
        if config
            .max_cost_usd
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            return Err("max_cost_usd must be finite and non-negative".to_string());
        }
        Ok(Self {
            config,
            state: Arc::new(Mutex::new(State {
                snapshot: BudgetSnapshot::default(),
                delivered_reminders: BTreeSet::new(),
            })),
        })
    }

    pub fn record(&self, usage: &UsageCharge) -> BudgetSnapshot {
        let weighted = weighted_usage(&self.config, usage);
        let mut state = self.state.lock().expect("budget lock");
        state.snapshot.weighted_tokens_used += weighted;
        state.snapshot.cost_usd += usage.cost_usd.max(0.0);
        update_exhausted(&self.config, &mut state.snapshot);
        state.snapshot.clone()
    }

    pub fn try_reserve(&self, weighted_tokens: u64) -> Result<BudgetReservation, String> {
        if weighted_tokens == 0 {
            return Err("budget reservations must be greater than zero".into());
        }
        let weighted_tokens = weighted_tokens as f64;
        let mut state = self.state.lock().expect("budget lock");
        let projected = state.snapshot.weighted_tokens_used
            + state.snapshot.weighted_tokens_reserved
            + weighted_tokens;
        if projected > self.config.limit_weighted_tokens as f64
            || self
                .config
                .max_cost_usd
                .is_some_and(|limit| state.snapshot.cost_usd >= limit)
        {
            return Err(format!(
                "shared orchestration budget cannot reserve {weighted_tokens:.0} weighted tokens"
            ));
        }
        state.snapshot.weighted_tokens_reserved += weighted_tokens;
        update_exhausted(&self.config, &mut state.snapshot);
        Ok(BudgetReservation {
            budget: self.clone(),
            weighted_tokens,
            active: true,
        })
    }

    pub fn snapshot(&self) -> BudgetSnapshot {
        self.state.lock().expect("budget lock").snapshot.clone()
    }

    pub fn take_reminder(&self, agent: &str) -> Option<u64> {
        let mut state = self.state.lock().expect("budget lock");
        let remaining = (self.config.limit_weighted_tokens as f64
            - state.snapshot.weighted_tokens_used
            - state.snapshot.weighted_tokens_reserved)
            .max(0.0) as u64;
        let reminder_index = self
            .config
            .reminder_at_remaining_tokens
            .iter()
            .filter(|threshold| remaining <= **threshold)
            .count();
        if reminder_index == 0
            || !state
                .delivered_reminders
                .insert((agent.to_string(), reminder_index))
        {
            return None;
        }
        Some(remaining)
    }
}

impl BudgetReservation {
    pub fn settle(mut self, usage: &UsageCharge) -> BudgetSnapshot {
        let mut state = self.budget.state.lock().expect("budget lock");
        state.snapshot.weighted_tokens_reserved =
            (state.snapshot.weighted_tokens_reserved - self.weighted_tokens).max(0.0);
        state.snapshot.weighted_tokens_used += weighted_usage(&self.budget.config, usage);
        state.snapshot.cost_usd += usage.cost_usd.max(0.0);
        update_exhausted(&self.budget.config, &mut state.snapshot);
        self.active = false;
        state.snapshot.clone()
    }
}

impl Drop for BudgetReservation {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut state = self.budget.state.lock().expect("budget lock");
        state.snapshot.weighted_tokens_reserved =
            (state.snapshot.weighted_tokens_reserved - self.weighted_tokens).max(0.0);
        update_exhausted(&self.budget.config, &mut state.snapshot);
    }
}

fn weighted_usage(config: &BudgetConfig, usage: &UsageCharge) -> f64 {
    let non_cached = usage.input_tokens.saturating_sub(usage.cached_input_tokens);
    non_cached as f64 * config.non_cached_input_weight
        + usage.cached_input_tokens as f64 * config.cached_input_weight
        + usage.output_tokens as f64 * config.output_weight
}

fn update_exhausted(config: &BudgetConfig, snapshot: &mut BudgetSnapshot) {
    snapshot.exhausted = snapshot.weighted_tokens_used + snapshot.weighted_tokens_reserved
        >= config.limit_weighted_tokens as f64
        || config
            .max_cost_usd
            .is_some_and(|limit| snapshot.cost_usd >= limit);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_weights_uncached_and_output_tokens() {
        let budget = SharedBudget::new(BudgetConfig {
            limit_weighted_tokens: 100,
            ..Default::default()
        })
        .unwrap();
        let snapshot = budget.record(&UsageCharge {
            input_tokens: 50,
            cached_input_tokens: 20,
            output_tokens: 10,
            cost_usd: 0.2,
        });
        assert_eq!(snapshot.weighted_tokens_used, 72.0);
        assert!(!snapshot.exhausted);
        assert_eq!(snapshot.cost_usd, 0.2);
    }

    #[test]
    fn concurrent_reservations_fail_before_they_can_oversubscribe() {
        let budget = SharedBudget::new(BudgetConfig {
            limit_weighted_tokens: 100,
            ..Default::default()
        })
        .unwrap();
        let first = budget.try_reserve(60).unwrap();
        assert!(budget.try_reserve(50).is_err());
        assert_eq!(budget.snapshot().weighted_tokens_reserved, 60.0);
        drop(first);
        assert_eq!(budget.snapshot().weighted_tokens_reserved, 0.0);
        assert!(budget.try_reserve(50).is_ok());
    }

    #[test]
    fn settling_replaces_a_reservation_with_authoritative_usage() {
        let budget = SharedBudget::new(BudgetConfig {
            limit_weighted_tokens: 100,
            ..Default::default()
        })
        .unwrap();
        let reservation = budget.try_reserve(80).unwrap();
        let snapshot = reservation.settle(&UsageCharge {
            input_tokens: 20,
            output_tokens: 5,
            ..Default::default()
        });
        assert_eq!(snapshot.weighted_tokens_reserved, 0.0);
        assert_eq!(snapshot.weighted_tokens_used, 40.0);
        assert!(!snapshot.exhausted);
    }
}
