//! Clark provider — clean-room WebSocket + MessagePack client for the Clark
//! gateway, behind the `agent_core::Provider` trait.
//!
//! Built from the observed wire contract (no Clark source): connect with an
//! `Authorization: Bearer <token>` header, bind realtime with
//! `resume_session` (protocol_version 2), submit turns through the canonical
//! `/api/conversation-sync/commands` HTTP endpoint, and translate the streamed
//! `{type:"event"}` frames into normalized events.

pub mod command;
mod provider;
pub mod sse;
mod translate;
pub mod transport;

pub use provider::ClarkProvider;
pub use transport::ClarkSocket;
