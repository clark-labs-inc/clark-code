use tokio::sync::OwnedRwLockReadGuard;

use crate::state::AppState;

pub(crate) struct AccountAccess {
    pub owner_scope: String,
    _account_lifecycle: Option<OwnedRwLockReadGuard<()>>,
}

/// Return the one native account generation used by ordinary cloud commands.
/// No renderer-controlled endpoint, token, or account label participates.
pub(crate) async fn current_account_access(state: &AppState) -> Result<AccountAccess, String> {
    let account_lifecycle = state.account_lifecycle.clone().read_owned().await;
    let account = state
        .runtime_registry
        .cloud_account()
        .await
        .ok_or("this product has no active signed-in account")?;
    Ok(AccountAccess {
        owner_scope: account.account.as_str().to_string(),
        _account_lifecycle: Some(account_lifecycle),
    })
}

#[cfg(test)]
mod tests {
    use super::current_account_access;
    use crate::runtime_registry::{AccountKey, CloudAccountState};
    use crate::AppState;

    #[tokio::test]
    async fn same_account_requests_do_not_serialize_behind_each_other() {
        let state = AppState::new();
        state
            .runtime_registry
            .set_cloud_account(Some(CloudAccountState {
                rest_base: "https://product.invalid".into(),
                account: AccountKey::new("account-a").unwrap(),
                token: zeroize::Zeroizing::new("token-a".into()),
            }))
            .await;

        let first = current_account_access(&state).await.unwrap();
        let second = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            current_account_access(&state),
        )
        .await
        .expect("same-account admission must remain concurrent")
        .unwrap();

        assert_eq!(first.owner_scope, second.owner_scope);
    }

    #[tokio::test]
    async fn account_access_is_one_atomic_registry_generation() {
        let state = AppState::new();
        assert!(current_account_access(&state).await.is_err());
        state
            .runtime_registry
            .set_cloud_account(Some(CloudAccountState {
                rest_base: "https://product.invalid".into(),
                account: AccountKey::new("account-a").unwrap(),
                token: zeroize::Zeroizing::new("opaque-token".into()),
            }))
            .await;
        let access = current_account_access(&state).await.unwrap();
        assert_eq!(access.owner_scope, "account-a");
    }
}
