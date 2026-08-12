//! Live configuration mutations for an idle local-provider session.

use agent_core::{ProviderConfiguration, ProviderConfigurationChange};

use super::*;

const MEMORY_SECTION_MARKER: &str = "\n# Memory\n";

impl LocalAgentProvider {
    pub(super) fn memory_config(&self, config: &LocalConfig) -> crate::tools::memory::MemoryConfig {
        crate::tools::memory::MemoryConfig {
            global_dir: config
                .memory_scope
                .as_deref()
                .and_then(crate::memory::global_memory_dir_for_scope),
            personal: self.context_provider.clone(),
        }
    }

    pub(super) async fn current_configuration(&self) -> Result<ProviderConfiguration> {
        let config = self.config()?;
        let style = self.session.lock().await.output_style.clone();
        Ok(crate::configuration::current(config, &style))
    }

    pub(super) async fn apply_configuration_change(
        &mut self,
        session: &SessionId,
        change: ProviderConfigurationChange,
    ) -> Result<ProviderConfiguration> {
        if self.session_id.as_ref() != Some(session) {
            return Err(Error::Unsupported(
                "Clark Code can only configure the active local session".into(),
            ));
        }
        match change {
            ProviderConfigurationChange::Model { model } => {
                self.apply_model(&model)?;
            }
            ProviderConfigurationChange::OutputStyle { style } => {
                if !crate::prompt::OUTPUT_STYLES
                    .iter()
                    .any(|candidate| candidate.id == style)
                {
                    return Err(Error::Unsupported(format!(
                        "Clark Code does not advertise output style `{style}`"
                    )));
                }
                self.session.lock().await.output_style = style;
            }
            ProviderConfigurationChange::Memories { enabled } => {
                self.apply_memories(enabled).await?;
            }
            ProviderConfigurationChange::Experiment { id, enabled } => {
                if id != "browser" {
                    return Err(Error::Unsupported(format!(
                        "Clark Code does not advertise experiment `{id}`"
                    )));
                }
                self.apply_browser(enabled).await?;
            }
        }
        self.current_configuration().await
    }

    fn apply_model(&mut self, model: &str) -> Result<()> {
        let capability = crate::configuration::model(self.config()?, model).ok_or_else(|| {
            Error::Unsupported(format!("the host does not advertise model `{model}`"))
        })?;
        let reasoning = capability.reasoning_effort.as_deref();
        let config = self.config()?;
        let image_config = config
            .api_key
            .clone()
            .filter(|_| {
                !config
                    .image_generation_excluded_models
                    .iter()
                    .any(|id| id == model)
            })
            .map(|api_key| crate::tools::image::ImageGenerationConfig {
                base_url: config.base_url.clone(),
                api_key,
            });
        if config.tools_enabled {
            let registry = self
                .registry
                .as_mut()
                .and_then(Arc::get_mut)
                .ok_or_else(|| Error::Other("tool registry is still in use".into()))?;
            registry.disable_image_generation();
            if let Some(image_config) = image_config {
                registry.enable_image_generation(image_config);
            }
        }
        let llm = self.llm.take().ok_or(Error::NotConnected)?;
        self.llm = Some(llm.with_model(model).with_reasoning_effort(reasoning));
        let config = self.config.as_mut().ok_or(Error::NotConnected)?;
        config.model = model.to_string();
        config.reasoning_effort = reasoning.map(str::to_string);
        Ok(())
    }

    async fn apply_memories(&mut self, enabled: bool) -> Result<()> {
        let config = self.config()?.clone();
        if !config.tools_enabled {
            return Err(Error::Unsupported(
                "this provider session has no memory tools".into(),
            ));
        }
        let section = if enabled {
            let sandbox = self.sandbox.as_ref().ok_or(Error::NotConnected)?.clone();
            Some(self.render_memory_section(&config, &sandbox).await?)
        } else {
            None
        };
        let memory_config = enabled.then(|| self.memory_config(&config));
        let registry = self
            .registry
            .as_mut()
            .and_then(Arc::get_mut)
            .ok_or_else(|| Error::Other("tool registry is still in use".into()))?;
        if enabled {
            registry.enable_memory(memory_config.expect("created when enabled"));
        } else {
            registry.disable_memory();
        }
        let mut session = self.session.lock().await;
        replace_memory_section(&mut session.system_prompt, section.as_deref());
        if !enabled {
            session.deferred_tools.remove("memory");
            session.deferred_tools.remove("memory_recall");
        }
        drop(session);
        self.config
            .as_mut()
            .ok_or(Error::NotConnected)?
            .memories_enabled = enabled;
        Ok(())
    }

    async fn apply_browser(&mut self, enabled: bool) -> Result<()> {
        if !self.config()?.tools_enabled {
            return Err(Error::Unsupported(
                "this provider session has no experimental tools".into(),
            ));
        }
        let browser = enabled
            .then(|| {
                self.config()
                    .ok()
                    .and_then(|config| config.browser_binary.clone())
            })
            .flatten();
        if enabled && browser.is_none() {
            return Err(Error::Unsupported(
                "this product does not provide a managed browser binary".into(),
            ));
        }
        let registry = self
            .registry
            .as_mut()
            .and_then(Arc::get_mut)
            .ok_or_else(|| Error::Other("tool registry is still in use".into()))?;
        if enabled {
            registry.enable_browser(browser.expect("checked above"));
        } else {
            registry.disable_browser();
            self.session.lock().await.deferred_tools.remove("browser");
        }
        self.config
            .as_mut()
            .ok_or(Error::NotConnected)?
            .browser_enabled = enabled;
        Ok(())
    }

    pub(super) async fn render_memory_section(
        &self,
        config: &LocalConfig,
        sandbox: &Sandbox,
    ) -> Result<String> {
        let mut memory = String::new();
        if let Some(project) = crate::memory::scope_listing(
            self.executor.as_ref(),
            &crate::memory::memory_dir(sandbox.root()),
            "Project",
            Some(sandbox.root()),
        )
        .await
        {
            memory.push_str(&project);
            memory.push('\n');
        }
        if let Some(global_dir) = config
            .memory_scope
            .as_deref()
            .and_then(crate::memory::global_memory_dir_for_scope)
        {
            if let Some(global) = crate::memory::scope_listing(
                &crate::exec::LocalExecutor,
                &global_dir,
                "Global",
                None,
            )
            .await
            {
                memory.push_str(&global);
                memory.push('\n');
            }
        }
        if let Some(provider) = &self.context_provider {
            if let Ok(memories) = provider.personal_memories().await {
                let memories = crate::platform::scope_personal_memories(
                    memories,
                    self.repository_fingerprint.as_deref(),
                );
                if let Some(personal) = crate::platform::personal_memory_section(&memories) {
                    memory.push_str(&personal);
                    memory.push('\n');
                }
            }
        }
        let mut section = format!(
            "{MEMORY_SECTION_MARKER}{}",
            crate::memory::memory_guidance()
        );
        if !memory.is_empty() {
            section.push('\n');
            section.push_str(&memory);
        }
        Ok(section)
    }
}

fn replace_memory_section(prompt: &mut String, section: Option<&str>) {
    if let Some(index) = prompt.find(MEMORY_SECTION_MARKER) {
        prompt.truncate(index);
    }
    if let Some(section) = section {
        prompt.push_str(section);
    }
}

pub(super) fn without_memory_section(prompt: &str) -> String {
    prompt
        .find(MEMORY_SECTION_MARKER)
        .map_or_else(|| prompt.to_string(), |index| prompt[..index].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_memory_is_idempotent() {
        let mut prompt = "base\n# Memory\nold".to_string();
        replace_memory_section(&mut prompt, Some("\n# Memory\nnew"));
        assert_eq!(prompt, "base\n# Memory\nnew");
        replace_memory_section(&mut prompt, None);
        assert_eq!(prompt, "base");
    }

    #[test]
    fn scout_turn_prompt_excludes_the_complete_memory_section() {
        let prompt =
            "base instructions\n# Skills\nscout catalog\n# Memory\nguidance\npersonal facts";
        assert_eq!(
            without_memory_section(prompt),
            "base instructions\n# Skills\nscout catalog"
        );
    }
}
