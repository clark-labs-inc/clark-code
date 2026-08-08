use std::time::Instant;

use zeroize::Zeroizing;

use super::{AccountKey, RuntimeRegistry};

const MAX_COMMAND_CLAIMS: usize = 512;

pub(super) struct CommandClaim {
    pub(super) account: AccountKey,
    host_id: String,
    instance_id: String,
    token: Zeroizing<String>,
    stored_at: Instant,
}

impl RuntimeRegistry {
    pub(crate) async fn store_command_claim(
        &self,
        account: AccountKey,
        command_id: String,
        host_id: String,
        instance_id: String,
        token: String,
    ) -> Result<(), String> {
        if command_id.is_empty() || host_id.is_empty() || instance_id.is_empty() || token.is_empty()
        {
            return Err("local agent command claim is invalid".into());
        }
        let mut claims = self.command_claims.lock().await;
        claims.insert(
            command_id,
            CommandClaim {
                account,
                host_id,
                instance_id,
                token: Zeroizing::new(token),
                stored_at: Instant::now(),
            },
        );
        while claims.len() > MAX_COMMAND_CLAIMS {
            let Some(oldest) = claims
                .iter()
                .min_by_key(|(_, claim)| claim.stored_at)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            claims.remove(&oldest);
        }
        Ok(())
    }

    pub(crate) async fn command_claim(
        &self,
        account: &AccountKey,
        command_id: &str,
        host_id: &str,
        instance_id: &str,
    ) -> Result<String, String> {
        let claims = self.command_claims.lock().await;
        let claim = claims
            .get(command_id)
            .ok_or("local agent command claim is no longer available")?;
        if &claim.account != account || claim.host_id != host_id || claim.instance_id != instance_id
        {
            return Err("local agent command claim belongs to a different account or host".into());
        }
        Ok(claim.token.as_str().to_string())
    }

    pub(crate) async fn remove_command_claim(&self, account: &AccountKey, command_id: &str) {
        let mut claims = self.command_claims.lock().await;
        if claims
            .get(command_id)
            .is_some_and(|claim| &claim.account == account)
        {
            claims.remove(command_id);
        }
    }

    pub(super) async fn remove_account_command_claims(&self, account: &AccountKey) {
        self.command_claims
            .lock()
            .await
            .retain(|_, claim| &claim.account != account);
    }
}
