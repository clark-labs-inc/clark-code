use super::*;
use std::path::PathBuf;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickChatWorkspace {
    pub id: String,
    pub path: String,
}

fn prepare_quick_chat_workspace_at(
    root: &std::path::Path,
    requested_id: Option<&str>,
) -> Result<QuickChatWorkspace, String> {
    let id = match requested_id {
        Some(id) => uuid::Uuid::parse_str(id)
            .map_err(|_| "Quick Chat conversation id must be a UUID".to_string())?,
        None => uuid::Uuid::new_v4(),
    };
    std::fs::create_dir_all(root).map_err(|error| format!("create Quick Chat root: {error}"))?;
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("open Quick Chat root: {error}"))?;
    let path = canonical_root.join(id.to_string());
    provider_local::initialize_quick_chat_workspace(&path)
        .map_err(|error| format!("create Quick Chat workspace: {error}"))?;
    Ok(QuickChatWorkspace {
        id: id.to_string(),
        path: path.to_string_lossy().into_owned(),
    })
}

/// Allocate or reopen one durable, app-managed Quick Chat checkout. The UUID
/// becomes the cloud conversation id; the local home-directory prefix is
/// intentionally re-resolved on every device.
#[tauri::command]
pub fn prepare_quick_chat_workspace(id: Option<String>) -> Result<QuickChatWorkspace, String> {
    let root = provider_local::workspace_root()
        .ok_or_else(|| "Quick Chat workspace root is unavailable".to_string())?;
    prepare_quick_chat_workspace_at(&root, id.as_deref())
}

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
    let _account_lifecycle = state.account_lifecycle.read().await;
    let sid = SessionId::new(session_id);
    let session_key = SessionKey::from_session(&sid)?;
    let entry = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
        .ok_or("no such session")?;
    let mut s = entry.lock().await;
    s.provider
        .side_question(&sid, &question)
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
    /// Absolute path to the scope's `.agent/memory` directory.
    pub dir: String,
    /// Whether the scope holds any memory (an index or at least one fact).
    pub exists: bool,
    /// Contents of the always-loaded `MEMORY.md` index, if present.
    pub index: Option<String>,
    /// Per-fact memory files (newest first).
    pub facts: Vec<MemoryFactView>,
}

/// Read one scope's `.agent/memory` directory into a viewer overview. The
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
/// root (`<checkout>/.agent/memory/`). Read-only.
#[tauri::command]
pub async fn local_list_memory(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<MemoryOverview, String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let session_key = SessionKey::parse(session_id)?;
    let entry = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
        .ok_or("no such conversation")?;
    let root = session_memory_root(&entry.lock().await.session)?;
    let mem_dir = provider_local::memory_dir(&root);
    Ok(memory_overview(&provider_local::LocalExecutor, &mem_dir).await)
}

async fn native_global_memory_dir(state: &AppState) -> Result<std::path::PathBuf, String> {
    let owner_scope = state
        .runtime_registry
        .cloud_account()
        .await
        .map(|account| account.account.as_str().to_string())
        .ok_or("Clark Code must be signed in before reading global memory")?;
    provider_local::global_memory_dir_for_scope(&owner_scope)
        .ok_or_else(|| "the signed-in account's global memory is unavailable".to_string())
}

/// List the current account's isolated global memory. The account partition is
/// selected only by the server-validated native authority; no renderer account
/// label crosses this command boundary.
#[tauri::command]
pub async fn local_list_global_memory(
    state: State<'_, AppState>,
) -> Result<MemoryOverview, String> {
    let mem_dir = native_global_memory_dir(state.inner()).await?;
    Ok(memory_overview(&provider_local::LocalExecutor, &mem_dir).await)
}

/// List project-relative file paths under `cwd` for the `@`-mention picker.
/// Read-only; skips ignored directories. Runs the walk off the UI thread.
#[tauri::command]
pub async fn local_list_files(
    cwd: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    if cwd.trim().is_empty() {
        return Ok(Vec::new());
    }
    let root = std::path::PathBuf::from(cwd);
    let exec = project_executor(remote, state.inner()).await?;
    Ok(provider_local::list_project_files(exec.as_ref(), &root).await)
}

/// Read sealed and in-progress Security scanner artifacts from the selected
/// checkout. The provider owns parsing and bounds; the desktop receives only
/// canonical scan records, never arbitrary project files.
#[tauri::command]
pub async fn local_list_security_scans(
    cwd: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<Vec<provider_local::SecurityScanRecord>, String> {
    if cwd.trim().is_empty() {
        return Ok(Vec::new());
    }
    let root = std::path::PathBuf::from(cwd);
    let exec = project_executor(remote, state.inner()).await?;
    provider_local::list_security_scans(exec.as_ref(), &root).await
}

/// Read an agent-authored document (Markdown) so the UI can render it inline.
/// Confined to the app-managed workspace (`~/.agent/workspace`) — it never reads
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
/// app-managed workspace or this conversation's native-owned project roots and
/// return it as a `data:` URL for inline `<img>` rendering.
#[tauri::command]
pub async fn read_image_data_url(
    path: String,
    session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let mut roots = provider_local::workspace_root()
        .and_then(|root| root.canonicalize().ok())
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(session_id) = session_id {
        let session_key = SessionKey::parse(session_id)?;
        let entry = state
            .runtime_registry
            .current_session_entry(&session_key)
            .await
            .ok_or("no such conversation")?;
        let session = entry.lock().await;
        let environment = session
            .session
            .environment
            .as_ref()
            .ok_or("this conversation has no filesystem binding")?;
        if environment.remote {
            return Err("remote images cannot be read from this device".into());
        }
        roots.extend(
            environment
                .workspace_roots
                .iter()
                .chain(environment.checkout_root.iter())
                .chain(environment.docs_root.iter())
                .filter_map(|root| PathBuf::from(root).canonicalize().ok()),
        );
    }
    tokio::task::spawn_blocking(move || read_image_from_roots(&path, &roots))
        .await
        .map_err(|error| format!("read failed: {error}"))?
}

fn read_image_from_roots(path: &str, roots: &[PathBuf]) -> Result<String, String> {
    use base64::Engine as _;

    const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
    let canon = PathBuf::from(&path)
        .canonicalize()
        .map_err(|e| format!("{path}: {e}"))?;
    if !roots.iter().any(|root| canon.starts_with(root)) {
        return Err("path is outside this conversation's workspace".into());
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
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => return Err("not a supported image type".into()),
    };
    let bytes = std::fs::read(&canon).map_err(|e| format!("read failed: {e}"))?;
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
    exec_core::suppress_std_console_window(&mut cmd);
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
    use super::{
        native_global_memory_dir, prepare_quick_chat_workspace_at, read_image_from_roots,
        session_memory_root,
    };
    use crate::runtime_registry::{AccountKey, CloudAccountState};
    use crate::AppState;
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

    #[tokio::test]
    async fn global_memory_partition_comes_only_from_native_account_authority() {
        let state = AppState::new();
        assert!(native_global_memory_dir(&state).await.is_err());

        state
            .runtime_registry
            .set_cloud_account(Some(CloudAccountState {
                rest_base: "https://product.example".into(),
                account: AccountKey::new("server-account-a").unwrap(),
                token: zeroize::Zeroizing::new("benchmark-token".into()),
            }))
            .await;

        let selected = native_global_memory_dir(&state).await.unwrap();
        assert_eq!(
            selected,
            provider_local::global_memory_dir_for_scope("server-account-a").unwrap()
        );
        assert_ne!(
            selected,
            provider_local::global_memory_dir_for_scope("server-account-b").unwrap()
        );
    }

    #[test]
    fn quick_chat_workspace_is_stable_and_confined() {
        let root = tempfile::tempdir().unwrap();
        let id = "912a9700-7f5f-4f18-9785-b5d9315a41b4";
        let first = prepare_quick_chat_workspace_at(root.path(), Some(id)).unwrap();
        let reopened = prepare_quick_chat_workspace_at(root.path(), Some(id)).unwrap();

        assert_eq!(first.path, reopened.path);
        assert_eq!(first.id, id);
        assert!(std::path::Path::new(&first.path).starts_with(root.path().canonicalize().unwrap()));
        assert!(std::path::Path::new(&first.path).is_dir());
        assert!(prepare_quick_chat_workspace_at(root.path(), Some("../escape")).is_err());
    }

    #[test]
    fn image_preview_accepts_only_native_approved_roots() {
        let approved = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let image = approved.path().join("result.png");
        let escaped = outside.path().join("secret.png");
        std::fs::write(&image, [1, 2, 3, 4]).unwrap();
        std::fs::write(&escaped, [5, 6, 7, 8]).unwrap();
        let roots = vec![approved.path().canonicalize().unwrap()];

        assert_eq!(
            read_image_from_roots(image.to_str().unwrap(), &roots).unwrap(),
            "data:image/png;base64,AQIDBA=="
        );
        assert!(read_image_from_roots(escaped.to_str().unwrap(), &roots)
            .unwrap_err()
            .contains("outside this conversation's workspace"));
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
