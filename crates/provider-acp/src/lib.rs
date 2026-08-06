//! Agent Client Protocol (ACP) provider.
//!
//! ACP is JSON-RPC 2.0 over stdio; the agent is a local CLI launched as a child
//! process (a Tauri sidecar in the app). We are the *client*: we send
//! `initialize` / `session/new` / `session/prompt`, and serve the agent's
//! `session/request_permission`, `fs/*`, and `terminal/*` calls.
//!
//! [`AcpProvider`] implements the stdio transport and the `Provider` trait
//! end-to-end.

mod orchestration;
mod provider;
mod translate;
pub mod transport;

pub use orchestration::read_only_harness;
pub use provider::AcpProvider;
pub use transport::{spawn_child, BoxRead, BoxWrite, Incoming, Peer};

/// ACP method names (client → agent), pinned against the published protocol.
pub mod method {
    pub const INITIALIZE: &str = "initialize";
    pub const AUTHENTICATE: &str = "authenticate";
    pub const SESSION_NEW: &str = "session/new";
    pub const SESSION_LOAD: &str = "session/load";
    pub const SESSION_RESUME: &str = "session/resume";
    pub const SESSION_PROMPT: &str = "session/prompt";
    pub const SESSION_SET_MODE: &str = "session/set_mode";
    pub const SESSION_CANCEL: &str = "session/cancel"; // notification

    // Agent → client (we serve these):
    pub const SESSION_UPDATE: &str = "session/update"; // notification
    pub const SESSION_REQUEST_PERMISSION: &str = "session/request_permission";
    pub const FS_READ_TEXT_FILE: &str = "fs/read_text_file";
    pub const FS_WRITE_TEXT_FILE: &str = "fs/write_text_file";
    pub const TERMINAL_CREATE: &str = "terminal/create";
    pub const TERMINAL_OUTPUT: &str = "terminal/output";
    pub const TERMINAL_RELEASE: &str = "terminal/release";
    pub const TERMINAL_WAIT_FOR_EXIT: &str = "terminal/wait_for_exit";
    pub const TERMINAL_KILL: &str = "terminal/kill";
}

/// Current ACP protocol version we negotiate in `initialize`.
pub const ACP_PROTOCOL_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    #[test]
    fn method_names_are_stable() {
        assert_eq!(super::method::SESSION_PROMPT, "session/prompt");
        assert_eq!(
            super::method::SESSION_REQUEST_PERMISSION,
            "session/request_permission"
        );
    }
}
