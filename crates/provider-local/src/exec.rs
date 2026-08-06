//! The execution backend the tools run against.
//!
//! The [`Executor`] trait + [`LocalExecutor`] live in the dependency-light
//! [`exec_core`] crate so worker composition roots can reuse the same
//! primitives without pulling in this provider's HTTP / model dependencies.

pub use exec_core::{Executor, LocalExecutor};
