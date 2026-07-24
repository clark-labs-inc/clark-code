use super::*;

/// `/btw` — answer a one-off side question against the session's current
/// context WITHOUT interrupting the active run. The provider forks a
/// tool-less, single-turn model call over the session transcript (never
/// mutating it); the answer text returns here for the overlay to render.
/// Holding the session lock for the call's duration pauses that session's
/// snapshot emission only — the run's engine task keeps executing and its
/// buffered events flush when this returns. Other sessions are unaffected
/// (per-entry locks).
#[tauri::command]
pub async fn side_question(
    session_id: String,
    question: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let entry = state
        .session_entry(&session_id)
        .await
        .ok_or("no such session")?;
    let mut s = entry.lock().await;
    s.provider
        .side_question(&SessionId::new(session_id), &question)
        .await
        .map_err(|e| e.to_string())
}

/// One per-fact memory file, flattened for the UI.
#[derive(serde::Serialize)]
pub struct MemoryFactView {
    pub file: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub kind: Option<String>,
    pub body: String,
}

/// Everything the memory viewer needs for one scope (project or global).
#[derive(serde::Serialize)]
pub struct MemoryOverview {
    /// Absolute path to the scope's `.clark/memory` directory.
    pub dir: String,
    /// Whether the scope holds any memory (an index or at least one fact).
    pub exists: bool,
    /// Contents of the always-loaded `MEMORY.md` index, if present.
    pub index: Option<String>,
    /// Per-fact memory files (newest first).
    pub facts: Vec<MemoryFactView>,
}

/// Read one scope's `.clark/memory` directory into a viewer overview. The
/// directory is always local here (the desktop machine), so `LocalExecutor`.
async fn memory_overview(
    exec: &dyn provider_local::Executor,
    mem_dir: &std::path::Path,
) -> MemoryOverview {
    let facts_raw = provider_local::load_facts(exec, mem_dir).await;
    let index = provider_local::load_index(exec, mem_dir).await;
    let exists = index.is_some() || !facts_raw.is_empty();
    let facts = facts_raw
        .into_iter()
        .map(|f| MemoryFactView {
            file: f.header.file,
            name: f.header.name,
            description: f.header.description,
            kind: f.header.kind.map(|k| k.label().to_string()),
            body: f.body,
        })
        .collect();
    MemoryOverview {
        dir: mem_dir.to_string_lossy().to_string(),
        exists,
        index,
        facts,
    }
}

fn session_memory_root(session: &Session) -> Result<std::path::PathBuf, String> {
    let environment = session
        .environment
        .as_ref()
        .ok_or("this conversation is not bound to a project")?;
    if environment.remote {
        return Err(
            "Project memory preview is unavailable for remote conversations in this version."
                .into(),
        );
    }
    let root = environment
        .checkout_root
        .as_deref()
        .filter(|root| !root.trim().is_empty())
        .ok_or("this conversation is not bound to a project")?;
    Ok(std::path::PathBuf::from(root))
}

/// List the project-scoped memory for a live conversation's host-owned checkout
/// root (`<checkout>/.clark/memory/`). Read-only.
#[tauri::command]
pub async fn local_list_memory(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<MemoryOverview, String> {
    let entry = state
        .session_entry(&session_id)
        .await
        .ok_or("no such conversation")?;
    let root = session_memory_root(&entry.lock().await.session)?;
    let mem_dir = provider_local::memory_dir(&root);
    Ok(memory_overview(&provider_local::LocalExecutor, &mem_dir).await)
}

/// List the user's global memory (`~/.clark/memory/`). Read-only.
#[tauri::command]
pub async fn local_list_global_memory() -> Result<MemoryOverview, String> {
    let Some(mem_dir) = provider_local::global_memory_dir() else {
        return Err("could not resolve your home directory".into());
    };
    Ok(memory_overview(&provider_local::LocalExecutor, &mem_dir).await)
}

/// List project-relative file paths under `cwd` for the `@`-mention picker.
/// Read-only; skips ignored directories. Runs the walk off the UI thread.
#[tauri::command]
pub async fn local_list_files(
    cwd: String,
    remote: Option<RemoteArg>,
) -> Result<Vec<String>, String> {
    if cwd.trim().is_empty() {
        return Ok(Vec::new());
    }
    let root = std::path::PathBuf::from(cwd);
    let exec = project_executor(remote).await?;
    Ok(provider_local::list_project_files(exec.as_ref(), &root).await)
}

/// Read an agent-authored document (Markdown) so the UI can render it inline.
/// Confined to the app-managed workspace (`~/.clark/workspace`) — it never reads
/// arbitrary files — and capped so a pathological file can't be slurped whole.
#[tauri::command]
pub async fn read_doc_text(path: String) -> Result<String, String> {
    const MAX_DOC_BYTES: u64 = 4 * 1024 * 1024;
    let root = provider_local::workspace_root()
        .ok_or_else(|| "no workspace directory".to_string())?
        .canonicalize()
        .map_err(|e| format!("workspace: {e}"))?;
    let canon = PathBuf::from(&path)
        .canonicalize()
        .map_err(|e| format!("{path}: {e}"))?;
    if !canon.starts_with(&root) {
        return Err("path is outside the document workspace".into());
    }
    let meta = std::fs::metadata(&canon).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    if meta.len() > MAX_DOC_BYTES {
        return Err("document too large to preview".into());
    }
    tokio::task::spawn_blocking(move || std::fs::read_to_string(&canon).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("read failed: {e}"))?
}

/// Read a locally-captured screenshot (or other small image) from the
/// app-managed workspace and return it as a `data:` URL for inline `<img>`
/// rendering. Confined to `~/.clark/workspace`, same root and containment
/// check as `read_doc_text`.
#[tauri::command]
pub async fn read_image_data_url(path: String) -> Result<String, String> {
    use base64::Engine as _;

    const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
    let root = provider_local::workspace_root()
        .ok_or_else(|| "no workspace directory".to_string())?
        .canonicalize()
        .map_err(|e| format!("workspace: {e}"))?;
    let canon = PathBuf::from(&path)
        .canonicalize()
        .map_err(|e| format!("{path}: {e}"))?;
    if !canon.starts_with(&root) {
        return Err("path is outside the document workspace".into());
    }
    let meta = std::fs::metadata(&canon).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    if meta.len() > MAX_IMAGE_BYTES {
        return Err("image too large to preview".into());
    }
    let mime = match canon
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        _ => return Err("not a supported image type".into()),
    };
    let bytes =
        tokio::task::spawn_blocking(move || std::fs::read(&canon).map_err(|e| e.to_string()))
            .await
            .map_err(|e| format!("read failed: {e}"))??;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Write an agent-authored document's text to a user-chosen path (the OS save
/// dialog returns an absolute path). The content itself is the in-memory text
/// the UI already rendered — the workspace file is only the source of truth for
/// reading — so the destination is unconstrained (a real download). Capped so a
/// pathological payload can't stream gigabytes to disk in one call.
#[tauri::command]
pub async fn save_doc_text(path: String, text: String) -> Result<(), String> {
    const MAX_DOC_BYTES: usize = 8 * 1024 * 1024;
    if text.len() > MAX_DOC_BYTES {
        return Err("document too large to save".into());
    }
    let p = PathBuf::from(&path);
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
        }
        std::fs::write(&p, text).map_err(|e| format!("write failed: {e}"))
    })
    .await
    .map_err(|e| format!("save failed: {e}"))?
}

/// Open a file (or folder) with the OS default handler — for a source file on a
/// dev machine that's typically the user's editor. `reveal` shows it in the file
/// manager instead of opening it. Never executes the file directly.
#[tauri::command]
pub fn open_path(path: String, reveal: bool) -> Result<(), String> {
    let p = path.trim();
    if p.is_empty() {
        return Err("empty path".into());
    }
    let mut cmd = open_command(p, reveal);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn open_command(path: &str, reveal: bool) -> std::process::Command {
    let mut c = std::process::Command::new("open");
    if reveal {
        c.arg("-R");
    }
    c.arg(path);
    c
}

#[cfg(test)]
mod tests {
    use super::session_memory_root;
    use agent_core::provider::SessionEnvironment;
    use agent_core::{CollaborationMode, ProviderCapabilities, ProviderId, Session, SessionId};

    fn session(root: Option<&str>, remote: bool) -> Session {
        Session {
            id: SessionId::new("memory-session"),
            provider: ProviderId::new("local"),
            capabilities: ProviderCapabilities::default(),
            mode: None,
            collaboration_mode: CollaborationMode::Default,
            environment: Some(SessionEnvironment {
                checkout_root: root.map(str::to_string),
                remote,
                ..Default::default()
            }),
        }
    }

    #[test]
    fn project_memory_root_comes_from_the_live_session() {
        assert_eq!(
            session_memory_root(&session(Some("/trusted/project"), false)).unwrap(),
            std::path::PathBuf::from("/trusted/project")
        );
        assert!(session_memory_root(&session(None, false)).is_err());
        assert!(session_memory_root(&session(Some("/remote/project"), true)).is_err());
    }
}

#[cfg(target_os = "windows")]
fn open_command(path: &str, reveal: bool) -> std::process::Command {
    if reveal {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{path}"));
        c
    } else {
        let mut c = std::process::Command::new("powershell.exe");
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "& { param($target) Start-Process -FilePath $target }",
            path,
        ]);
        c
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_command(path: &str, reveal: bool) -> std::process::Command {
    // No portable "reveal" on Linux — open the containing folder instead.
    let target = if reveal {
        std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string())
    } else {
        path.to_string()
    };
    let mut c = std::process::Command::new("xdg-open");
    c.arg(target);
    c
}
