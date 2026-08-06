use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::validate::{validate_digest, validate_identifier};
use crate::{ProtocolError, ProtocolResult};

macro_rules! digest_id {
    ($name:ident, $prefix:literal, $field:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> ProtocolResult<Self> {
                let identifier = Self(value.into());
                identifier.validate()?;
                Ok(identifier)
            }

            pub(crate) fn from_digest(digest: String) -> ProtocolResult<Self> {
                Self::new(format!("{}{}", $prefix, digest))
            }

            pub fn validate(&self) -> ProtocolResult<()> {
                let digest = self.0.strip_prefix($prefix).ok_or_else(|| {
                    ProtocolError::invalid($field, concat!("must start with ", $prefix))
                })?;
                validate_digest($field, digest)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

macro_rules! uuid_id {
    ($name:ident, $prefix:literal, $field:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> ProtocolResult<Self> {
                let identifier = Self(value.into());
                identifier.validate()?;
                Ok(identifier)
            }

            pub fn validate(&self) -> ProtocolResult<()> {
                let raw = self.0.strip_prefix($prefix).ok_or_else(|| {
                    ProtocolError::invalid($field, concat!("must start with ", $prefix))
                })?;
                let parsed = Uuid::parse_str(raw)
                    .map_err(|_| ProtocolError::invalid($field, "must contain a canonical UUID"))?;
                if parsed.hyphenated().to_string() != raw {
                    return Err(ProtocolError::invalid(
                        $field,
                        "must contain a canonical lowercase UUID",
                    ));
                }
                Ok(())
            }

            #[cfg(not(target_arch = "wasm32"))]
            pub fn random() -> Self {
                Self(format!("{}{}", $prefix, Uuid::new_v4().hyphenated()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

digest_id!(TargetId, "target:", "target_id");
digest_id!(AuthContextId, "authctx:", "auth_context_id");
digest_id!(RecordId, "record:", "record_id");
digest_id!(ReceiptId, "receipt:", "receipt_id");

uuid_id!(AuthContextHandle, "auth:", "auth_context_handle");
uuid_id!(CursorHandle, "cursor:", "cursor_handle");
uuid_id!(RequestId, "request:", "request_id");

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AdapterId(String);

impl AdapterId {
    pub fn new(value: impl Into<String>) -> ProtocolResult<Self> {
        let identifier = Self(value.into());
        identifier.validate()?;
        Ok(identifier)
    }

    pub fn validate(&self) -> ProtocolResult<()> {
        if self.0.trim() != self.0 || self.0.len() > 160 {
            return Err(ProtocolError::invalid(
                "adapter_id",
                "must contain 1 to 160 characters without surrounding whitespace",
            ));
        }
        let Some((name, version)) = self.0.rsplit_once('@') else {
            return Err(ProtocolError::invalid(
                "adapter_id",
                "must be versioned as namespace/name@version",
            ));
        };
        if name.is_empty() || !name.contains('/') || version.is_empty() {
            return Err(ProtocolError::invalid(
                "adapter_id",
                "must be versioned as namespace/name@version",
            ));
        }
        for component in name.split('/') {
            validate_identifier("adapter_id", component, 64)?;
        }
        validate_identifier("adapter_id", version, 64)?;
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AdapterId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
