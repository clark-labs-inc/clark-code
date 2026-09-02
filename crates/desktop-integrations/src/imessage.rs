//! Bounded, text-only reader for the local Messages database. The schema is an
//! observed macOS implementation detail, not a supported Apple history API.
//! Schema changes fail closed; there is no fallback crawl or attachment read.
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::{Availability, Conversation, Integration, Manifest, Message};

mod native;
pub use native::open_privacy_settings;

pub struct IMessage {
    database: PathBuf,
}

impl IMessage {
    pub fn local() -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        let home = std::env::var_os("HOME").ok_or("Home directory unavailable")?;
        #[cfg(target_os = "macos")]
        let database = PathBuf::from(home).join("Library/Messages/chat.db");
        #[cfg(not(target_os = "macos"))]
        let database = PathBuf::new();
        native::initialize();
        Ok(Self { database })
    }

    fn db(&self) -> Result<Connection, String> {
        open_database(&self.database)
    }
}

fn open_database(path: &Path) -> Result<Connection, String> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "Messages history is unavailable. Grant Full Disk Access to this exact app in System Settings, then restart it. This is app-wide access, not a per-conversation macOS permission.")?;
    db.busy_timeout(std::time::Duration::from_millis(500))
        .map_err(|_| "Messages database unavailable")?;
    Ok(db)
}

const SELF_CONVERSATIONS: &str = "
    SELECT CAST(c.ROWID AS TEXT), c.chat_identifier
    FROM chat c
    WHERE c.service_name='iMessage'
      AND c.chat_identifier IN (
        SELECT DISTINCT destination_caller_id FROM message
        WHERE is_from_me=1 AND service='iMessage' AND destination_caller_id IS NOT NULL
      )
      AND (SELECT COUNT(*) FROM chat_handle_join j WHERE j.chat_id=c.ROWID)=1
      AND EXISTS (SELECT 1 FROM chat_handle_join j JOIN handle h ON h.ROWID=j.handle_id
                  WHERE j.chat_id=c.ROWID AND h.id=c.chat_identifier)
    ORDER BY c.ROWID DESC LIMIT 30";

fn conversations(db: &Connection) -> Result<Vec<Conversation>, String> {
    let mut query = db
        .prepare(SELF_CONVERSATIONS)
        .map_err(|_| "Unsupported Messages database schema; no fallback read was attempted")?;
    let conversations = query
        .query_map([], |row| {
            Ok(Conversation {
                id: row.get(0)?,
                self_address: row.get(1)?,
            })
        })
        .map_err(|_| "Cannot query self-conversations")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| String::from("Cannot decode self-conversations"))?;
    Ok(conversations)
}

fn validate(db: &Connection, conversation: &Conversation) -> Result<(), String> {
    if conversations(db)?.iter().any(|candidate| {
        candidate.id == conversation.id && candidate.self_address == conversation.self_address
    }) {
        Ok(())
    } else {
        Err("Selected conversation is no longer an eligible self-conversation".into())
    }
}

fn read(db: &Connection, conversation: &Conversation) -> Result<Vec<Message>, String> {
    validate(db, conversation)?;
    let mut query = db.prepare(
        "SELECT CAST(m.ROWID AS TEXT), substr(m.text,1,4000), m.is_from_me,
                CAST(m.date / 1000000000 AS INTEGER) + 978307200
         FROM message m WHERE m.service='iMessage'
           AND EXISTS (SELECT 1 FROM chat_message_join j WHERE j.message_id=m.ROWID AND j.chat_id=?1)
         ORDER BY m.ROWID DESC LIMIT 50",
    ).map_err(|_| "Unsupported Messages text schema")?;
    let mut messages = query
        .query_map([&conversation.id], |row| {
            Ok(Message {
                id: row.get(0)?,
                text: row
                    .get::<_, Option<String>>(1)?
                    .unwrap_or_else(|| "[Non-plain-text message omitted]".into()),
                from_me: row.get(2)?,
                unix_seconds: row.get(3)?,
            })
        })
        .map_err(|_| "Cannot read selected conversation")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Cannot decode selected text")?;
    messages.reverse();
    Ok(messages)
}

impl Integration for IMessage {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "imessage".into(),
            name: "iMessage".into(),
            description: "Read exact text selected from one self-conversation through a task-scoped Clark Code tool. No send, draft, attachments, polling, or inbound task trigger.".into(),
            capabilities: vec!["selected_text".into(), "read_tool".into()],
            experimental: true,
        }
    }

    fn availability(&self) -> Availability {
        Availability {
            supported: cfg!(target_os = "macos"),
            detail: "Read-only prototype. Full Disk Access belongs to the whole app, while Clark's integration grant limits only read_imessage_selection to exact text you enable for one task. Sandboxed coding file tools deny Messages paths, but Full Access, MCP, external agents, terminals, and computer use can still bypass that task scope. Do not grant OS access if you need isolation from those tools.".into(),
        }
    }

    fn epoch(&self) -> u64 {
        native::epoch()
    }

    fn interactive(&self) -> bool {
        native::interactive()
    }

    fn approve_read(&self, task: &str) -> bool {
        native::confirm(
            "Grant read-only iMessage access?",
            &format!(
                "Task: {task}\n\nList self-conversation addresses and let you select text for up to 15 minutes. The read_imessage_selection tool can access only the exact messages you later enable in Settings. A tool result is shared with this task's model as untrusted quoted context.\n\nWARNING: Full Disk Access is app-wide. Full Access, MCP, external agents, terminals, and computer use can bypass this task scope. Revoke OS permission separately in System Settings."
            ),
            "Grant read-only access",
        )
    }

    fn conversations(&self) -> Result<Vec<Conversation>, String> {
        conversations(&self.db()?)
    }

    fn read(&self, conversation: &Conversation) -> Result<Vec<Message>, String> {
        read(&self.db()?, conversation)
    }
}

#[cfg(test)]
mod tests;
