//! Clark-owned live configuration catalog for the local coding provider.
//!
//! Clients render this descriptor through `Provider::configuration`; they do
//! not duplicate model or output-style choices in their own UI code.

use agent_core::{
    ExperimentCapability, ModelCapability, OutputStyleCapability, ProviderConfiguration,
};

use crate::config::{LocalConfig, DEFAULT_MODEL};

#[derive(Clone, Copy)]
struct CodingModel {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    reasoning_effort: &'static str,
}

const CODING_MODELS: &[CodingModel] = &[
    CodingModel {
        id: "clark-code:free",
        label: "Free",
        description: "Fast coding and agent work",
        reasoning_effort: "max",
    },
    CodingModel {
        id: "clark-code:glm52",
        label: "GLM 5.2",
        description: "Daily driver for coding and security",
        reasoning_effort: "xhigh",
    },
    CodingModel {
        id: "clark-code:kimi_k3",
        label: "Kimi K3",
        description: "Super intelligence",
        reasoning_effort: "max",
    },
];

pub(crate) fn model(model: &str) -> Option<ModelCapability> {
    CODING_MODELS
        .iter()
        .find(|candidate| candidate.id == model)
        .map(|candidate| ModelCapability {
            id: candidate.id.into(),
            label: candidate.label.into(),
            description: candidate.description.into(),
            reasoning_effort: Some(candidate.reasoning_effort.into()),
        })
}

pub fn defaults() -> ProviderConfiguration {
    configuration(DEFAULT_MODEL, "default", true, false)
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
    )
}

fn configuration(
    model_id: &str,
    output_style: &str,
    memories_enabled: bool,
    browser_enabled: bool,
) -> ProviderConfiguration {
    let selected_model = model(model_id);
    ProviderConfiguration {
        model: Some(model_id.to_string()),
        reasoning_effort: selected_model
            .as_ref()
            .and_then(|model| model.reasoning_effort.clone()),
        models: CODING_MODELS
            .iter()
            .map(|model| ModelCapability {
                id: model.id.into(),
                label: model.label.into(),
                description: model.description.into(),
                reasoning_effort: Some(model.reasoning_effort.into()),
            })
            .collect(),
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
            description: "Download and use Clark's guarded browser tool on demand".into(),
            enabled: browser_enabled,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_owned_by_the_provider_catalog() {
        let settings = defaults();
        assert_eq!(settings.model.as_deref(), Some(DEFAULT_MODEL));
        assert_eq!(settings.reasoning_effort.as_deref(), Some("max"));
        assert_eq!(settings.models.len(), 3);
        assert!(settings.models.iter().all(|model| {
            !model.id.is_empty()
                && !model.label.is_empty()
                && model
                    .reasoning_effort
                    .as_deref()
                    .is_some_and(|effort| !effort.is_empty())
        }));
        assert_eq!(settings.output_styles.len(), 3);
        assert_eq!(settings.memories_enabled, Some(true));
        assert_eq!(settings.experiments[0].id, "browser");
    }
}
