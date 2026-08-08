use agent_core::provider::ProviderConfig;

/// Extra containment applied only to an orchestrator-owned disposable writer.
///
/// The normal interactive provider intentionally attaches user-level context,
/// a document workspace, project hooks, and optional external tools. An
/// unattended writer must not inherit any of those ambient capabilities: its
/// complete writable world is the ephemeral repository clone selected by the
/// multi-repository control plane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ProviderIsolation {
    disposable_writer: bool,
}

impl ProviderIsolation {
    pub(super) fn from_provider_config(config: &ProviderConfig) -> Self {
        Self {
            disposable_writer: config
                .extra
                .get("isolated_writer")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        }
    }

    pub(super) fn disposable_writer(self) -> bool {
        self.disposable_writer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolation_is_explicit_and_defaults_off() {
        assert!(
            !ProviderIsolation::from_provider_config(&ProviderConfig::default())
                .disposable_writer()
        );
        let config = ProviderConfig {
            extra: serde_json::json!({"isolated_writer": true}),
            ..Default::default()
        };
        assert!(ProviderIsolation::from_provider_config(&config).disposable_writer());
    }
}
