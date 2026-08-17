use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use agent_core::ProviderConfig;
use code_host::{CodingSessionRecipe, PluginError};
use provider_local::LocalAgentProvider;
use serde_json::{json, Value};

/// Compile-time extension point for product-owned capabilities attached to a
/// single remote coding session. The recipe contains no credential; the
/// branded worker resolves credentials from its native-injected environment.
pub trait CodingSessionExtension: Send + Sync {
    fn id(&self) -> &str;

    fn configure_provider(
        &self,
        provider: LocalAgentProvider,
        config: &Value,
        project_root: &Path,
    ) -> Result<LocalAgentProvider, String>;
}

pub(super) fn register_extension(
    extensions: &mut BTreeMap<String, Arc<dyn CodingSessionExtension>>,
    extension: Arc<dyn CodingSessionExtension>,
) -> Result<(), String> {
    let id = extension.id().to_string();
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("coding session extension id is invalid".into());
    }
    if extensions.insert(id.clone(), extension).is_some() {
        return Err(format!(
            "coding session extension is already registered: {id}"
        ));
    }
    Ok(())
}

pub(super) fn apply_session_recipe(
    config: &mut ProviderConfig,
    recipe: &CodingSessionRecipe,
) -> Result<(), PluginError> {
    let extra = config.extra.as_object_mut().ok_or_else(|| {
        PluginError::InvalidInput("worker provider configuration is not an object".into())
    })?;
    if let Some(kind) = recipe.specialist_kind.as_ref() {
        extra.insert("specialist_kind".into(), Value::String(kind.clone()));
        // Specialist skill instructions extend the canonical remote prompt.
        // The generic headless override would otherwise hide that base policy.
        extra.remove("system_prompt_override");
    }
    if !recipe.hard_constraints.is_empty() {
        extra.insert(
            "hard_constraints".into(),
            serde_json::to_value(&recipe.hard_constraints)
                .map_err(|error| PluginError::InvalidInput(error.to_string()))?,
        );
    }
    if let Some(scout) = recipe.scout_cartography.as_ref() {
        extra.insert(
            "scout_cartography".into(),
            serde_json::to_value(scout)
                .map_err(|error| PluginError::InvalidInput(error.to_string()))?,
        );
        extra.insert("orchestration".into(), json!({ "enabled": true }));
    }
    Ok(())
}

pub(super) fn apply_session_extensions(
    mut provider: LocalAgentProvider,
    recipe: &CodingSessionRecipe,
    project_root: &Path,
    registered: &BTreeMap<String, Arc<dyn CodingSessionExtension>>,
) -> Result<LocalAgentProvider, PluginError> {
    for extension in &recipe.extensions {
        let handler = registered.get(&extension.id).ok_or_else(|| {
            PluginError::InvalidInput(format!(
                "remote coding worker does not provide session extension {}",
                extension.id
            ))
        })?;
        provider = handler
            .configure_provider(provider, &extension.config, project_root)
            .map_err(PluginError::InvalidInput)?;
    }
    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use code_host::CodingSessionExtensionRecipe;

    #[test]
    fn generic_worker_rejects_an_unregistered_product_extension() {
        let recipe = CodingSessionRecipe {
            extensions: vec![CodingSessionExtensionRecipe {
                id: "clark_cloud_advisor".into(),
                config: json!({ "organization_id": "org-1" }),
            }],
            ..CodingSessionRecipe::default()
        };
        let error = apply_session_extensions(
            LocalAgentProvider::new(),
            &recipe,
            Path::new("/srv/project"),
            &BTreeMap::new(),
        )
        .err()
        .expect("generic worker must reject branded recipes");
        assert!(error
            .to_string()
            .contains("does not provide session extension clark_cloud_advisor"));
    }
}
