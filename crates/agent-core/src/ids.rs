//! Strongly-typed string identifiers. Newtypes keep `SessionId` and `RunId` from
//! being accidentally swapped while staying transparent on the wire (plain JSON
//! strings).

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// Identifies a connected conversation with one provider.
    SessionId
);
string_id!(
    /// Identifies one agent turn (work produced for a single user prompt).
    RunId
);
string_id!(
    /// Identifies a single tool invocation within a run.
    ToolCallId
);
string_id!(
    /// Identifies a registered provider implementation (e.g. `"acp"`, `"clark"`).
    ProviderId
);
string_id!(
    /// Identifies a pending host-side permission request.
    PermissionRequestId
);
