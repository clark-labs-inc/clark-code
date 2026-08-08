use serde::{Deserialize, Serialize};

use crate::{CapsuleError, CapsuleResult};

pub const CAPSULE_ABI_VERSION: u16 = 1;

const ABSOLUTE_MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;
const ABSOLUTE_MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const ABSOLUTE_MAX_RECORDS: usize = 10_000;
const ABSOLUTE_MAX_NESTING_DEPTH: usize = 64;
const ABSOLUTE_MAX_STRUCTURAL_TOKENS: usize = 2_000_000;
const ABSOLUTE_MAX_STRING_TOKEN_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleLimits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_records: usize,
    pub max_nesting_depth: usize,
    pub max_structural_tokens: usize,
    pub max_string_token_bytes: usize,
}

impl CapsuleLimits {
    pub fn validate(self) -> CapsuleResult<Self> {
        validate_limit(
            "max_input_bytes",
            self.max_input_bytes,
            ABSOLUTE_MAX_INPUT_BYTES,
        )?;
        validate_limit(
            "max_output_bytes",
            self.max_output_bytes,
            ABSOLUTE_MAX_OUTPUT_BYTES,
        )?;
        validate_limit("max_records", self.max_records, ABSOLUTE_MAX_RECORDS)?;
        validate_limit(
            "max_nesting_depth",
            self.max_nesting_depth,
            ABSOLUTE_MAX_NESTING_DEPTH,
        )?;
        validate_limit(
            "max_structural_tokens",
            self.max_structural_tokens,
            ABSOLUTE_MAX_STRUCTURAL_TOKENS,
        )?;
        validate_limit(
            "max_string_token_bytes",
            self.max_string_token_bytes,
            ABSOLUTE_MAX_STRING_TOKEN_BYTES,
        )?;
        Ok(self)
    }
}

impl Default for CapsuleLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 8 * 1024 * 1024,
            max_output_bytes: 16 * 1024 * 1024,
            max_records: 1_000,
            max_nesting_depth: 32,
            max_structural_tokens: 500_000,
            max_string_token_bytes: 16 * 1024,
        }
    }
}

fn validate_limit(field: &'static str, value: usize, absolute_max: usize) -> CapsuleResult<()> {
    if value == 0 || value > absolute_max {
        return Err(CapsuleError::invalid(
            field,
            format!("must be between 1 and {absolute_max}"),
        ));
    }
    Ok(())
}
