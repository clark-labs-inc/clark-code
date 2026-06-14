//! Clark provider — clean-room WebSocket + MessagePack client for the Clark
//! gateway, behind the `agent_core::Provider` trait.
//!
//! Built from the observed wire contract (no Clark source): connect with an
//! `Authorization: Bearer <token>` header, receive `{type:"connected"}`, send
//! `resume_session` (protocol_version 2) then `send_message`, and translate the
//! streamed `{type:"event"}` frames into normalized events.

mod provider;
mod sse;
mod translate;
pub mod transport;

pub use provider::ClarkProvider;
pub use transport::ClarkSocket;
