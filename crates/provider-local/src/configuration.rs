//! Host-supplied live configuration catalog for the local coding provider.

use agent_core::{
    ExperimentCapability, ModelCapability, OutputStyleCapability, ProviderConfiguration,
};

use crate::config::{LocalConfig, DEFAULT_MODEL};

fn default_models() -> Vec<ModelCapability> {
    vec![ModelCapability {
        id: DEFAULT_MODEL.into(),
        label: "Local model".into(),
        description: "OpenAI-compatible local coding model".into(),
        reasoning_effort: None,
    }]
}

pub(crate) fn model(config: &LocalConfig, model: &str) -> Option<ModelCapability> {
    config
        .models
        .iter()
        .find(|candidate| candidate.id == model)
        .cloned()
}

pub fn defaults() -> ProviderConfiguration {
    configuration(DEFAULT_MODEL, "default", true, false, default_models())
}

pub(crate) fn current(config: &LocalConfig, output_style: &str) -> ProviderConfiguration {
    configuration(
        &config.model,
        if output_style.is_empty() {
            "default"
        } else {
            output_style
        },
        config.memories_enabled,
        config.browser_enabled,
        config.models.clone(),
    )
}

fn configuration(
    model_id: &str,
    output_style: &str,
    memories_enabled: bool,
    browser_enabled: bool,
    models: Vec<ModelCapability>,
) -> ProviderConfiguration {
    let selected_model = models.iter().find(|model| model.id == model_id);
    ProviderConfiguration {
        model: Some(model_id.to_string()),
        reasoning_effort: selected_model.and_then(|model| model.reasoning_effort.clone()),
        models,
        output_style: Some(output_style.to_string()),
        output_styles: crate::prompt::OUTPUT_STYLES
            .iter()
            .map(|style| OutputStyleCapability {
                id: style.id.into(),
                label: style.label.into(),
                description: style.description.into(),
            })
            .collect(),
        memories_enabled: Some(memories_enabled),
        experiments: vec![ExperimentCapability {
            id: "browser".into(),
            label: "Browser tool".into(),
            description: "Download and use the guarded browser tool on demand".into(),
            enabled: browser_enabled,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_product_neutral() {
        let settings = defaults();
        assert_eq!(settings.model.as_deref(), Some(DEFAULT_MODEL));
        assert_eq!(settings.models.len(), 1);
        assert_eq!(settings.models[0].label, "Local model");
        assert_eq!(settings.output_styles.len(), 3);
        assert_eq!(settings.memories_enabled, Some(true));
        assert_eq!(settings.experiments[0].id, "browser");
    }
}
