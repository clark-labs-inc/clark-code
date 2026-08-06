pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS journal_conversation (
    owner_key TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    metadata_json BLOB NOT NULL,
    base_snapshot_json BLOB NOT NULL,
    base_rev INTEGER NOT NULL DEFAULT 0,
    checkpoint_seq INTEGER NOT NULL DEFAULT 0,
    local_live INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (owner_key, conversation_id)
);
CREATE TABLE IF NOT EXISTS trajectory_outbox (
    local_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id TEXT NOT NULL UNIQUE,
    owner_key TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    max_recorded_at_ms INTEGER NOT NULL,
    request_json BLOB NOT NULL,
    acknowledged INTEGER NOT NULL DEFAULT 0,
    replayable INTEGER NOT NULL DEFAULT 1,
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY (owner_key, conversation_id)
        REFERENCES journal_conversation(owner_key, conversation_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS trajectory_outbox_pending
    ON trajectory_outbox(owner_key, conversation_id, acknowledged, local_seq);
"#;
