#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusValue {
    pub(crate) label: String,
    pub(crate) value: String,
    pub(crate) source: String,
}

impl StatusValue {
    pub(crate) fn new(
        label: impl Into<String>,
        value: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            source: source.into(),
        }
    }

    fn render(&self) -> String {
        format!("{}: {} (source: {})", self.label, self.value, self.source)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct UsageSnapshot {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) context_tokens: u64,
    pub(crate) context_limit: Option<u64>,
    pub(crate) cost_usd: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusPanel {
    authentication: StatusValue,
    organization: StatusValue,
    plan: StatusValue,
    workspace: StatusValue,
    provider: StatusValue,
    sync: StatusValue,
    configuration: Vec<StatusValue>,
}

impl StatusPanel {
    pub(crate) fn new(
        authentication: StatusValue,
        organization: StatusValue,
        plan: StatusValue,
        workspace: StatusValue,
        provider: StatusValue,
        sync: StatusValue,
        configuration: Vec<StatusValue>,
    ) -> Self {
        Self {
            authentication,
            organization,
            plan,
            workspace,
            provider,
            sync,
            configuration,
        }
    }

    pub(crate) fn render(&self, provider_status: &str, usage: Option<UsageSnapshot>) -> String {
        let mut values = vec![
            self.authentication.clone(),
            self.organization.clone(),
            self.plan.clone(),
            self.workspace.clone(),
            self.provider.clone(),
            self.sync.clone(),
        ];
        values.push(StatusValue::new(
            "Live provider state",
            provider_status,
            "agent_core Provider event stream",
        ));
        let configuration = self
            .configuration
            .iter()
            .map(StatusValue::render)
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "Clark status\n{}\n{}\n{}",
            values
                .iter()
                .map(StatusValue::render)
                .collect::<Vec<_>>()
                .join("\n"),
            render_usage(usage),
            if configuration.is_empty() {
                "Effective configuration: not reported".to_string()
            } else {
                format!("Effective configuration\n{configuration}")
            }
        )
    }

    pub(crate) fn mark_synchronized(&mut self, receipt: &str) {
        self.sync.value = receipt.to_string();
        self.sync.source = "Clark cloud synchronization receipt".into();
    }

    pub(crate) fn set_configuration(
        &mut self,
        label: &str,
        value: impl Into<String>,
        source: impl Into<String>,
    ) {
        let value = value.into();
        let source = source.into();
        if let Some(existing) = self
            .configuration
            .iter_mut()
            .find(|existing| existing.label == label)
        {
            existing.value = value;
            existing.source = source;
        } else {
            self.configuration
                .push(StatusValue::new(label, value, source));
        }
    }
}

fn render_usage(usage: Option<UsageSnapshot>) -> String {
    let Some(usage) = usage else {
        return "Usage: not reported (source: agent_core Provider event stream)".into();
    };
    let context = match usage.context_limit {
        Some(limit) => format!("{} / {} tokens", usage.context_tokens, limit),
        None => format!("{} tokens; limit not reported", usage.context_tokens),
    };
    let cost = usage
        .cost_usd
        .map_or_else(|| "not reported".into(), |cost| format!("${cost:.4}"));
    format!(
        "Usage\nInput: {} tokens\nOutput: {} tokens\nContext: {context}\nCost: {cost}\nSource: agent_core Provider event stream",
        usage.input_tokens, usage.output_tokens
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> StatusPanel {
        StatusPanel::new(
            StatusValue::new("Authentication", "verified", "Clark Cloud /cli/context"),
            StatusValue::new("Organization", "org-7", "Clark credential resolution"),
            StatusValue::new("Plan", "paid entitlement verified", "Clark access response"),
            StatusValue::new("Workspace", "Scientist · /project", "Clark CLI selection"),
            StatusValue::new("Provider", "native specialist", "Clark runtime"),
            StatusValue::new(
                "Cloud sync",
                "preflight complete",
                "Clark runtime preflight",
            ),
            vec![StatusValue::new(
                "Model",
                "provider-selected",
                "provider capability",
            )],
        )
    }

    #[test]
    fn status_keeps_identity_sync_usage_and_configuration_sources_distinct() {
        let rendered = panel().render(
            "working",
            Some(UsageSnapshot {
                input_tokens: 1200,
                output_tokens: 80,
                context_tokens: 9000,
                context_limit: Some(100_000),
                cost_usd: None,
            }),
        );
        for label in [
            "Authentication:",
            "Organization:",
            "Plan:",
            "Workspace:",
            "Provider:",
            "Cloud sync:",
            "Live provider state:",
            "Effective configuration",
        ] {
            assert!(rendered.contains(label), "missing {label} in {rendered}");
        }
        assert!(rendered.contains("Context: 9000 / 100000 tokens"));
        assert!(rendered.contains("Cost: not reported"));
    }

    #[test]
    fn synchronization_receipt_replaces_pending_state() {
        let mut panel = panel();
        panel.mark_synchronized("3 artifacts uploaded");
        let rendered = panel.render("complete", None);
        assert!(rendered.contains("Cloud sync: 3 artifacts uploaded"));
        assert!(rendered.contains("source: Clark cloud synchronization receipt"));
        assert!(rendered.contains("Usage: not reported"));
    }
}
