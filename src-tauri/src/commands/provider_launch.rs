use agent_core::ProviderConfig;
use serde_json::Value;

/// Renderer-safe provider launch inputs. Network endpoints, headers, bearer
/// tokens, and account partitions have no representation at this boundary;
/// native provider preparation derives them from the active RuntimeRegistry
/// generation after deserialization.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderLaunchRequest {
    command: Option<Vec<String>>,
    cwd: Option<String>,
    #[serde(default)]
    extra: Value,
}

impl ProviderLaunchRequest {
    pub(super) fn into_provider_config(self, provider_id: &str) -> Result<ProviderConfig, String> {
        match provider_id {
            "acp" => {
                if !self.extra.is_null() {
                    return Err("ACP launch configuration accepts only its command and cwd".into());
                }
            }
            "clark" => {
                if self.command.is_some() || self.cwd.is_some() || !self.extra.is_null() {
                    return Err("Clark connection configuration is native-owned".into());
                }
            }
            "local" | "specialist" => {
                if self.command.is_some() {
                    return Err("local provider executables are native-owned".into());
                }
            }
            _ => return Err(format!("unknown provider: {provider_id}")),
        }
        Ok(ProviderConfig {
            command: self.command,
            cwd: self.cwd,
            extra: self.extra,
            ..ProviderConfig::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderLaunchRequest;

    #[test]
    fn ipc_has_no_connection_or_credential_fields() {
        for forbidden in ["endpoint", "headers", "auth_token"] {
            let value = serde_json::json!({ forbidden: "renderer-controlled" });
            assert!(serde_json::from_value::<ProviderLaunchRequest>(value).is_err());
        }
        let request: ProviderLaunchRequest =
            serde_json::from_value(serde_json::json!({})).expect("empty Clark request is valid");
        assert!(request.into_provider_config("clark").is_ok());
    }

    #[test]
    fn renderer_cannot_choose_native_executables_or_account_partition() {
        let local: ProviderLaunchRequest = serde_json::from_value(serde_json::json!({
            "command": ["untrusted-provider"],
        }))
        .unwrap();
        assert!(local.into_provider_config("local").is_err());

        let clark: ProviderLaunchRequest = serde_json::from_value(serde_json::json!({
            "extra": { "memory_scope": "another-account" },
        }))
        .unwrap();
        assert!(clark.into_provider_config("clark").is_err());
    }
}
