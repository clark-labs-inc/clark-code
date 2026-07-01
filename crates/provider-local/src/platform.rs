//! Read the user's **personal memory** from Clark's Platform API.
//!
//! Clark extracts durable per-user facts from the user's conversations
//! server-side (the `clark-memory-extraction` pipeline) and exposes them at
//! `GET {base_url}/memories` for a `ck_live_` key (scope `memories:read`). We
//! layer these on top of the agent's local file-based memory: read-only recall,
//! injected at session start and available through the `memory` tool. The key
//! resolves to its owning user, so no user id is passed.

use std::time::Duration;

use serde::Deserialize;

/// One personal memory returned by `GET /v1/memories`.
#[derive(Clone, Debug, Deserialize)]
pub struct PersonalMemory {
    #[serde(default)]
    pub key: Option<String>,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Deserialize)]
struct MemoryList {
    #[serde(default)]
    data: Vec<PersonalMemory>,
}

/// Fetch the signed-in user's personal memories from Clark. Best-effort: a short
/// timeout and any error (offline, missing `memories:read` scope, 4xx/5xx) maps
/// to `Err` so callers can degrade to local-only memory silently.
pub async fn recall_personal_memories(
    base_url: &str,
    api_key: &str,
) -> Result<Vec<PersonalMemory>, String> {
    let url = format!("{}/memories", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GET /memories → {}", resp.status()));
    }
    let list: MemoryList = resp.json().await.map_err(|e| e.to_string())?;
    Ok(list.data)
}

/// A compact prompt/recall section for the user's personal memories, or `None`
/// if there are none.
pub fn personal_memory_section(memories: &[PersonalMemory]) -> Option<String> {
    if memories.is_empty() {
        return None;
    }
    let mut s = String::from("## Personal memory (learned by Clark across your work)\n");
    for m in memories {
        let line = m.content.trim().replace('\n', " ");
        if line.is_empty() {
            continue;
        }
        s.push_str(&format!("- {line}\n"));
    }
    Some(s)
}
