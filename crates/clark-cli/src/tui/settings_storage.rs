use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use agent_core::ProviderConfiguration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigurationPreferences {
    model: String,
    output_style: String,
    memories_enabled: bool,
    experiments: BTreeMap<String, bool>,
}

impl ConfigurationPreferences {
    pub(crate) fn load(path: &Path, capabilities: &ProviderConfiguration) -> Result<Self, String> {
        let stored = match fs::read_to_string(path) {
            Ok(contents) => StoredPreferences::decode(&contents)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                StoredPreferences::default()
            }
            Err(error) => {
                return Err(format!(
                    "could not read Clark agent settings {}: {error}",
                    path.display()
                ))
            }
        };
        stored.resolve(capabilities)
    }

    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    pub(crate) fn reasoning_effort<'a>(
        &self,
        capabilities: &'a ProviderConfiguration,
    ) -> Option<&'a str> {
        capabilities
            .models
            .iter()
            .find(|model| model.id == self.model)
            .and_then(|model| model.reasoning_effort.as_deref())
    }

    pub(crate) fn output_style(&self) -> &str {
        &self.output_style
    }

    pub(crate) fn memories_enabled(&self) -> bool {
        self.memories_enabled
    }

    pub(crate) fn experiment_enabled(&self, id: &str) -> Option<bool> {
        self.experiments.get(id).copied()
    }

    pub(super) fn from_live(configuration: &ProviderConfiguration) -> Result<Self, String> {
        StoredPreferences::default().resolve(configuration)
    }

    pub(super) fn save(&self, path: &Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Clark agent settings path has no parent".to_string())?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create Clark agent settings directory {}: {error}",
                parent.display()
            )
        })?;
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, self.encode()).map_err(|error| {
            format!(
                "could not write Clark agent settings {}: {error}",
                temporary.display()
            )
        })?;
        fs::rename(&temporary, path).map_err(|error| {
            format!(
                "could not replace Clark agent settings {}: {error}",
                path.display()
            )
        })
    }

    fn encode(&self) -> String {
        let mut lines = vec![
            "version=1".to_string(),
            format!("model={}", self.model),
            format!("personality={}", self.output_style),
            format!("memories={}", self.memories_enabled),
        ];
        lines.extend(
            self.experiments
                .iter()
                .map(|(id, enabled)| format!("experimental.{id}={enabled}")),
        );
        format!("{}\n", lines.join("\n"))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct StoredPreferences {
    model: Option<String>,
    output_style: Option<String>,
    memories_enabled: Option<bool>,
    experiments: BTreeMap<String, bool>,
}

impl StoredPreferences {
    fn decode(contents: &str) -> Result<Self, String> {
        let mut stored = Self::default();
        let mut version = None;
        let mut seen = HashSet::new();
        for line in contents.lines().filter(|line| !line.trim().is_empty()) {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("invalid Clark agent setting line {line:?}"))?;
            if !seen.insert(key.to_string()) {
                return Err(format!("duplicate Clark agent setting {key:?}"));
            }
            match key {
                "version" => version = Some(value),
                "model" => stored.model = Some(setting_value(key, value)?),
                "personality" => stored.output_style = Some(setting_value(key, value)?),
                "memories" => stored.memories_enabled = Some(parse_bool(key, value)?),
                key if key.starts_with("experimental.") => {
                    let id = key.trim_start_matches("experimental.");
                    let id = setting_value("experimental id", id)?;
                    stored.experiments.insert(id, parse_bool(key, value)?);
                }
                _ => return Err(format!("unknown Clark agent setting {key:?}")),
            }
        }
        if version != Some("1") {
            return Err("unsupported or missing Clark agent settings version".into());
        }
        Ok(stored)
    }

    fn resolve(
        self,
        capabilities: &ProviderConfiguration,
    ) -> Result<ConfigurationPreferences, String> {
        let model = self
            .model
            .or_else(|| capabilities.model.clone())
            .ok_or("Clark provider did not report an active model")?;
        if !capabilities
            .models
            .iter()
            .any(|candidate| candidate.id == model)
        {
            return Err(format!(
                "saved model `{model}` is not advertised by this Clark provider"
            ));
        }
        let output_style = self
            .output_style
            .or_else(|| capabilities.output_style.clone())
            .ok_or("Clark provider did not report an active personality")?;
        if !capabilities
            .output_styles
            .iter()
            .any(|candidate| candidate.id == output_style)
        {
            return Err(format!(
                "saved personality `{output_style}` is not advertised by this Clark provider"
            ));
        }
        let memories_enabled = self
            .memories_enabled
            .or(capabilities.memories_enabled)
            .ok_or("Clark provider does not expose memory controls")?;
        let mut experiments = capabilities
            .experiments
            .iter()
            .map(|experiment| (experiment.id.clone(), experiment.enabled))
            .collect::<BTreeMap<_, _>>();
        for (id, enabled) in self.experiments {
            let Some(current) = experiments.get_mut(&id) else {
                return Err(format!(
                    "saved experiment `{id}` is not advertised by this Clark provider"
                ));
            };
            *current = enabled;
        }
        Ok(ConfigurationPreferences {
            model,
            output_style,
            memories_enabled,
            experiments,
        })
    }
}

fn setting_value(key: &str, value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(format!("invalid Clark agent setting {key:?}"));
    }
    Ok(value.to_string())
}

fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("invalid Boolean value for {key:?}")),
    }
}
