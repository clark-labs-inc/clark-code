use std::path::{Path, PathBuf};

use agent_core::{ProviderConfiguration, ProviderConfigurationChange};

#[path = "settings_storage.rs"]
mod storage;

pub(crate) use storage::ConfigurationPreferences;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfigurationSection {
    Model,
}

impl ConfigurationSection {
    pub(crate) fn from_command(command: &str) -> Option<Self> {
        (command == "model").then_some(Self::Model)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConfigurationRequest {
    Inspect(ConfigurationSection),
    Change(ProviderConfigurationChange),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModelConfiguration {
    live: ProviderConfiguration,
    locked_reason: Option<String>,
}

impl ModelConfiguration {
    pub(crate) fn path(cwd: &Path) -> PathBuf {
        directories::BaseDirs::new().map_or_else(
            || cwd.join(".clark/agent.conf"),
            |directories| directories.config_dir().join("clark/agent.conf"),
        )
    }

    pub(crate) fn active(live: ProviderConfiguration) -> Self {
        Self {
            live,
            locked_reason: None,
        }
    }

    pub(crate) fn locked(reason: impl Into<String>) -> Self {
        Self {
            live: ProviderConfiguration::default(),
            locked_reason: Some(reason.into()),
        }
    }

    pub(crate) fn handles_line(line: &str) -> bool {
        line.trim()
            .strip_prefix('/')
            .and_then(|line| line.split_whitespace().next())
            == Some("model")
    }

    pub(crate) fn request(&self, line: &str) -> Option<Result<ConfigurationRequest, String>> {
        let mut words = line.trim().strip_prefix('/')?.split_whitespace();
        let section = ConfigurationSection::from_command(words.next()?)?;
        let arguments = words.collect::<Vec<_>>();
        if arguments.is_empty() {
            return Some(Ok(ConfigurationRequest::Inspect(section)));
        }
        if let Some(reason) = &self.locked_reason {
            return Some(Err(reason.clone()));
        }
        Some(self.model_request(&arguments))
    }

    pub(crate) fn report(&self, _section: ConfigurationSection) -> String {
        if let Some(reason) = &self.locked_reason {
            return format!("Clark model\n{reason}\nNo setting was changed.");
        }
        let selected = self.live.model.as_deref().unwrap_or("not reported");
        let reasoning = self
            .live
            .reasoning_effort
            .as_deref()
            .unwrap_or("provider default");
        let choices = self
            .live
            .models
            .iter()
            .map(|model| {
                format!(
                    "- {} ({}) · reasoning {} · {}",
                    model.id,
                    model.label,
                    model
                        .reasoning_effort
                        .as_deref()
                        .unwrap_or("provider default"),
                    model.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "Clark model\nActive: {selected}\nReasoning: {reasoning} (owned by the selected model capability)\nAvailable from provider:\n{choices}\nUsage: /model MODEL_ID"
        )
    }

    pub(crate) fn replace_live(&mut self, live: ProviderConfiguration) {
        self.live = live;
    }

    pub(crate) fn save(&self, path: &Path) -> Result<(), String> {
        ConfigurationPreferences::from_live(&self.live)?.save(path)
    }

    fn model_request(&self, arguments: &[&str]) -> Result<ConfigurationRequest, String> {
        if arguments.len() != 1 {
            return Err("Usage: /model MODEL_ID".into());
        }
        let model = arguments[0];
        if !self
            .live
            .models
            .iter()
            .any(|candidate| candidate.id == model)
        {
            return Err(format!(
                "Clark provider does not advertise model `{model}`. Run /model to inspect choices."
            ));
        }
        Ok(ConfigurationRequest::Change(
            ProviderConfigurationChange::Model {
                model: model.into(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{ExperimentCapability, ModelCapability, OutputStyleCapability};
    use std::fs;

    fn capabilities() -> ProviderConfiguration {
        ProviderConfiguration {
            model: Some("clark-code:free".into()),
            reasoning_effort: Some("max".into()),
            models: vec![ModelCapability {
                id: "clark-code:free".into(),
                label: "Free".into(),
                description: "Fast".into(),
                reasoning_effort: Some("max".into()),
            }],
            output_style: Some("default".into()),
            output_styles: vec![OutputStyleCapability {
                id: "default".into(),
                label: "Default".into(),
                description: "Normal".into(),
            }],
            memories_enabled: Some(true),
            experiments: vec![ExperimentCapability {
                id: "browser".into(),
                label: "Browser".into(),
                description: "Guarded browsing".into(),
                enabled: false,
            }],
        }
    }

    #[test]
    fn requests_are_validated_against_provider_capabilities() {
        let settings = ModelConfiguration::active(capabilities());
        assert!(matches!(
            settings.request("/model clark-code:free"),
            Some(Ok(ConfigurationRequest::Change(
                ProviderConfigurationChange::Model { .. }
            )))
        ));
        assert!(settings
            .request("/model invented")
            .unwrap()
            .unwrap_err()
            .contains("does not advertise"));
        assert!(settings
            .report(ConfigurationSection::Model)
            .contains("Reasoning: max"));
        assert!(!ModelConfiguration::handles_line("/personality default"));
        assert!(!ModelConfiguration::handles_line(
            "/experimental browser on"
        ));
    }

    #[test]
    fn preferences_round_trip_and_reject_removed_capabilities() {
        let directory =
            std::env::temp_dir().join(format!("clark-tui-settings-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("agent.conf");
        let settings = ModelConfiguration::active(capabilities());
        settings.save(&path).unwrap();
        let restored = ConfigurationPreferences::load(&path, &capabilities()).unwrap();
        assert_eq!(restored.model(), "clark-code:free");
        fs::write(
            &path,
            "version=1\nmodel=retired\npersonality=default\nmemories=true\nexperimental.browser=false\n",
        )
        .unwrap();
        assert!(ConfigurationPreferences::load(&path, &capabilities())
            .unwrap_err()
            .contains("not advertised"));
    }

    #[test]
    fn specialist_owned_model_is_visible_and_immutable() {
        let settings = ModelConfiguration::locked(
            "Paid specialist model and reasoning are selected by its Clark capability.",
        );
        assert!(settings
            .report(ConfigurationSection::Model)
            .contains("selected by its Clark capability"));
        assert!(settings
            .request("/model clark-code:free")
            .unwrap()
            .unwrap_err()
            .contains("selected by"));
    }
}
