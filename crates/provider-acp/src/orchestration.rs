use std::sync::Arc;

use agent_orchestration::{
    HarnessKind, ProviderHarness, ProviderHarnessConfig, ReadOnlyEnforcement, WorkspaceGuard,
};

use crate::AcpProvider;

/// Build a live ACP-backed delegated harness.
///
/// External ACP processes do not share Clark's local tool gate, so this helper
/// refuses prompt-only/host-gate safety claims. The command must be wrapped in
/// an OS read-only sandbox or run against a disposable checkout.
pub fn read_only_harness(
    config: ProviderHarnessConfig,
    workspace: Arc<dyn WorkspaceGuard>,
) -> Result<ProviderHarness, String> {
    if config.kind != HarnessKind::Acp {
        return Err("ACP adapter requires harness kind=acp".to_string());
    }
    if config
        .provider_config
        .command
        .as_ref()
        .is_none_or(Vec::is_empty)
    {
        return Err("ACP adapter requires a non-empty provider command".to_string());
    }
    if config.enforcement == ReadOnlyEnforcement::HostToolGate {
        return Err(
            "ACP adapter requires an OS sandbox or disposable checkout boundary".to_string(),
        );
    }
    ProviderHarness::new(
        config,
        Arc::new(|| Box::new(AcpProvider::new()) as Box<dyn agent_core::provider::Provider>),
        workspace,
    )
}
