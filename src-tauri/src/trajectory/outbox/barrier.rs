use std::path::PathBuf;
use std::time::{Duration, Instant};

use rusqlite::params;

use super::{blocking, open, owner_key, sql_error};

/// Wait until the cloud has acknowledged every local trajectory batch covered
/// by a snapshot checkpoint. Later batches do not block an older projection.
pub(crate) async fn wait_for_acknowledged_prefix(
    path: PathBuf,
    owner_scope: String,
    conversation_id: String,
    checkpoint_seq: i64,
    timeout: Duration,
) -> Result<(), String> {
    if checkpoint_seq <= 0 {
        return Ok(());
    }
    let deadline = Instant::now() + timeout;
    loop {
        let path = path.clone();
        let owner_scope = owner_scope.clone();
        let conversation_id = conversation_id.clone();
        let pending = blocking(move || {
            let conn = open(&path)?;
            conn.query_row(
                r#"SELECT EXISTS(
                       SELECT 1 FROM trajectory_outbox
                       WHERE owner_key = ?1 AND conversation_id = ?2
                         AND local_seq <= ?3 AND acknowledged = 0
                   )"#,
                params![owner_key(&owner_scope), conversation_id, checkpoint_seq],
                |row| row.get::<_, bool>(0),
            )
            .map_err(sql_error)
        })
        .await?;
        if !pending {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "cloud history is still syncing through local checkpoint {checkpoint_seq}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
