//! Composition helpers for standalone headless coding-agent workers.
//!
//! The binary in `src/main.rs` registers the first-party coding plugin. A
//! different worker can reuse this library, register additional
//! [`code_host::HeadlessPlugin`] implementations, and keep the same host
//! protocol and project/trajectory policy.

pub mod coding;
pub mod config;
pub mod project;
pub mod stdio;

use code_host::{HeadlessHost, PluginError, ProjectRegistry, RegistryError};
use thiserror::Error;

use config::{ConfigError, WorkerConfig};

/// Build the default worker host. Callers may mutate the returned host to
/// register additional typed plugins before handing it to their transport.
pub fn build_host(config: &WorkerConfig) -> Result<HeadlessHost, WorkerBuildError> {
    build_host_with_coding(
        config,
        coding::CodingPlugin::new(config.provider.clone(), config.execution_residency),
    )
}

/// Build a worker host with a product-composed coding plugin while retaining
/// the canonical project registry and built-in project plugin.
pub fn build_host_with_coding(
    config: &WorkerConfig,
    coding: coding::CodingPlugin,
) -> Result<HeadlessHost, WorkerBuildError> {
    config.validate()?;
    let projects = ProjectRegistry::new(config.projects.clone())?;
    let mut host = HeadlessHost::new(projects, config.trajectory_root.clone());
    if config.enabled_plugins.contains("coding") {
        host.register_plugin(coding)?;
    }
    if config.enabled_plugins.contains("project") {
        host.register_plugin(project::ProjectPlugin::new())?;
    }
    Ok(host)
}

#[derive(Debug, Error)]
pub enum WorkerBuildError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Plugin(#[from] PluginError),
}
