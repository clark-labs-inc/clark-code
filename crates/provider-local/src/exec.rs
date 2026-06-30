//! The execution backend the tools run against.
//!
//! The [`Executor`] trait + [`LocalExecutor`] live in the dependency-light
//! [`exec_core`] crate so the remote `clark-exec-server` can reuse the exact
//! same primitives without pulling in this provider's HTTP / model deps. This
//! module re-exports them (so the rest of `provider-local` keeps referring to
//! `crate::exec::*`) and adds [`RemoteExecutor`], the client that forwards those
//! primitives to a remote exec-server over the [`exec_protocol`] WebSocket.

pub use exec_core::{Executor, LocalExecutor};

mod remote;
pub use remote::RemoteExecutor;
