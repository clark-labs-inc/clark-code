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
            "local" => {
                if self.command.is_some() {
                    return Err("local provider executables are native-owned".into());
                }
            }
            _ => {
                if self.command.is_some() {
                    return Err("product provider executables are native-owned".into());
                }
            }
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
            serde_json::from_value(serde_json::json!({})).expect("empty product request is valid");
        assert!(request.into_provider_config("product-provider").is_ok());
    }

    #[test]
    fn renderer_cannot_choose_native_executables_or_account_partition() {
        let local: ProviderLaunchRequest = serde_json::from_value(serde_json::json!({
            "command": ["untrusted-provider"],
        }))
        .unwrap();
        assert!(local.into_provider_config("local").is_err());

        let product: ProviderLaunchRequest = serde_json::from_value(serde_json::json!({
            "command": ["untrusted-provider"],
        }))
        .unwrap();
        assert!(product.into_provider_config("product-provider").is_err());
    }
}
