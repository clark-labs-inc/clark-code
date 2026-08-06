use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{CapsuleHostError, CapsuleHostResult};

const MAX_MODULE_BYTES: usize = 16 * 1024 * 1024;
const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_LINEAR_MEMORY_BYTES: usize = 512 * 1024 * 1024;
const MAX_TABLE_ELEMENTS: u32 = 1_000_000;
const MAX_FUEL: u64 = 10_000_000_000;
const MAX_DEADLINE: Duration = Duration::from_secs(120);
const MAX_CONCURRENT_INSTANCES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleHostLimits {
    pub max_module_bytes: usize,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_linear_memory_bytes: usize,
    pub max_table_elements: u32,
    pub max_fuel: u64,
    pub deadline_ms: u64,
    pub max_concurrent_instances: usize,
}

impl CapsuleHostLimits {
    pub fn validate(self) -> CapsuleHostResult<Self> {
        validate_usize("max_module_bytes", self.max_module_bytes, MAX_MODULE_BYTES)?;
        validate_usize("max_input_bytes", self.max_input_bytes, MAX_INPUT_BYTES)?;
        validate_usize("max_output_bytes", self.max_output_bytes, MAX_OUTPUT_BYTES)?;
        validate_usize(
            "max_linear_memory_bytes",
            self.max_linear_memory_bytes,
            MAX_LINEAR_MEMORY_BYTES,
        )?;
        if self.max_table_elements == 0 || self.max_table_elements > MAX_TABLE_ELEMENTS {
            return Err(CapsuleHostError::InvalidLimit("max_table_elements"));
        }
        if self.max_fuel == 0 || self.max_fuel > MAX_FUEL {
            return Err(CapsuleHostError::InvalidLimit("max_fuel"));
        }
        let deadline = self.deadline();
        if deadline < Duration::from_millis(1) || deadline > MAX_DEADLINE {
            return Err(CapsuleHostError::InvalidLimit("deadline_ms"));
        }
        validate_usize(
            "max_concurrent_instances",
            self.max_concurrent_instances,
            MAX_CONCURRENT_INSTANCES,
        )?;
        Ok(self)
    }

    pub fn deadline(self) -> Duration {
        Duration::from_millis(self.deadline_ms)
    }
}

impl Default for CapsuleHostLimits {
    fn default() -> Self {
        Self {
            max_module_bytes: 8 * 1024 * 1024,
            max_input_bytes: 8 * 1024 * 1024,
            max_output_bytes: 16 * 1024 * 1024,
            max_linear_memory_bytes: 64 * 1024 * 1024,
            max_table_elements: 4096,
            max_fuel: 50_000_000,
            deadline_ms: 5_000,
            max_concurrent_instances: 8,
        }
    }
}

fn validate_usize(field: &'static str, value: usize, maximum: usize) -> CapsuleHostResult<()> {
    if value == 0 || value > maximum {
        return Err(CapsuleHostError::InvalidLimit(field));
    }
    Ok(())
}
