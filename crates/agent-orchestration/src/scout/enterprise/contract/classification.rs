use serde::{Deserialize, Serialize};

/// Ordered handling policy for enterprise topology metadata.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum EnterpriseClassification {
    Public,
    #[default]
    Internal,
    Confidential,
    Restricted,
    SecretReferenceOnly,
    DoNotStore,
}

impl EnterpriseClassification {
    pub fn join(self, other: Self) -> Self {
        self.max(other)
    }

    pub fn permits(self, record: Self) -> bool {
        record <= self && record != Self::DoNotStore
    }

    pub fn rank(self) -> u8 {
        self as u8
    }

    pub(super) fn validate_persistable(self) -> Result<(), String> {
        if self == Self::DoNotStore {
            return Err(
                "DoNotStore enterprise observations must be rejected before event construction"
                    .into(),
            );
        }
        Ok(())
    }
}

pub(super) fn is_default_classification(value: &EnterpriseClassification) -> bool {
    *value == EnterpriseClassification::Internal
}
