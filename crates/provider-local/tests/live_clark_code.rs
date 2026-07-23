//! Live `clark-code` conversation matrix for the local coding provider.
//!
//! Ignored by default. This makes real Clark Platform model calls and mutates a
//! temporary project directory only when live env is explicit:
//!
//! ```sh
//! CLARK_CODE_LIVE=1 \
//! CLARK_CODE_BASE_URL=https://api.clarkslabs.com/v1 \
//! CLARK_CODE_MODEL=clark-code:deepseek_v4_pro \
//! CLARK_CODE_API_KEY=ck_live_... \
//!   cargo test -p provider-local --test live_clark_code -- --ignored --nocapture --test-threads=1
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_core::domain::{AgentEvent, ContentBlock, PendingUpload, RunStatus, ToolStatus};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use base64::Engine as _;
use futures::StreamExt;
use provider_local::LocalAgentProvider;
use serde_json::json;

const TURN_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Debug)]
struct LiveConfig {
    base_url: String,
    model: String,
    api_key: String,
}

fn is_clark_code_model(value: &str) -> bool {
    value == "clark-code" || value.starts_with("clark-code:")
}

fn live_config() -> Option<LiveConfig> {
    if std::env::var("CLARK_CODE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping: set CLARK_CODE_LIVE=1 to permit live clark-code calls");
        return None;
    }
    let base_url = match std::env::var("CLARK_CODE_BASE_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            eprintln!("skipping: set CLARK_CODE_BASE_URL to the Clark Platform provider URL");
            return None;
        }
    };
    let model = match std::env::var("CLARK_CODE_MODEL") {
        Ok(value) if is_clark_code_model(value.trim()) => value,
        Ok(value) => {
            eprintln!("skipping: CLARK_CODE_MODEL must be a clark-code tier, got {value:?}");
            return None;
        }
        Err(_) => {
            eprintln!("skipping: set CLARK_CODE_MODEL to an explicit clark-code tier");
            return None;
        }
    };
    let api_key = match std::env::var("CLARK_CODE_API_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            eprintln!("skipping: set CLARK_CODE_API_KEY");
            return None;
        }
    };
    Some(LiveConfig {
        base_url,
        model,
        api_key,
    })
}

#[test]
fn live_matrix_accepts_backend_owned_clark_code_aliases_only() {
    assert!(is_clark_code_model("clark-code"));
    assert!(is_clark_code_model("clark-code:deepseek_v4_pro"));
    assert!(!is_clark_code_model("deepseek/deepseek-v4-pro"));
    assert!(!is_clark_code_model("clark"));
}

#[derive(Default, Debug)]
struct TurnSummary {
    finished: bool,
    status: Option<RunStatus>,
    run_error: Option<String>,
    usage: Option<agent_core::domain::RunUsage>,
    text: String,
    tools: Vec<String>,
    errors: Vec<String>,
    tool_statuses: BTreeMap<String, Vec<ToolStatus>>,
    permission_requests: usize,
    event_counts: BTreeMap<&'static str, usize>,
}

impl TurnSummary {
    fn require_done(&self, label: &str) {
        assert!(self.finished, "{label}: run did not finish: {self:?}");
        assert_eq!(
            self.status,
            Some(RunStatus::Done),
            "{label}: run did not finish cleanly: {self:?}"
        );
    }

    fn require_tool(&self, label: &str, tool: &str) {
        assert!(
            self.tools.iter().any(|seen| seen == tool),
            "{label}: expected tool {tool}, got {:?}",
            self.tools
        );
    }

    fn require_cloud_research_first(&self, label: &str) {
        let research = self
            .tools
            .iter()
            .position(|tool| tool == "clark_research")
            .unwrap_or_else(|| panic!("{label}: expected clark_research, got {:?}", self.tools));
        let discovery = self
            .tools
            .iter()
            .position(|tool| tool == "tool_search")
            .unwrap_or_else(|| panic!("{label}: expected tool_search, got {:?}", self.tools));
        assert!(
            discovery < research,
            "{label}: research must be activated before use: {:?}",
            self.tools
        );
        assert!(
            !self.tools[..research]
                .iter()
                .any(|tool| tool == "web_fetch" || tool == "bash"),
            "{label}: local retrieval ran before Clark Cloud Agent: {:?}",
            self.tools
        );
    }
}

async fn new_live_provider(
    cfg: &LiveConfig,
    cwd: &Path,
    extra: serde_json::Value,
) -> (LocalAgentProvider, agent_core::provider::Session) {
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some(cfg.api_key.clone()),
            extra: merge_extra(
                json!({
                    "base_url": cfg.base_url,
                    "model": cfg.model,
                    "cwd": cwd.to_string_lossy(),
                }),
                extra,
            ),
            ..Default::default()
        })
        .await
        .expect("connect local clark-code provider");
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(cwd.to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .expect("new local session");
    (provider, session)
}

fn merge_extra(mut base: serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    let Some(base_obj) = base.as_object_mut() else {
        return extra;
    };
    if let Some(extra_obj) = extra.as_object() {
        for (key, value) in extra_obj {
            base_obj.insert(key.clone(), value.clone());
        }
    }
    base
}

async fn drive_turn(
    provider: &mut LocalAgentProvider,
    session_id: &agent_core::ids::SessionId,
    prompt: &str,
) -> TurnSummary {
    drive_prompt(provider, session_id, PromptInput::text(prompt)).await
}

async fn drive_prompt(
    provider: &mut LocalAgentProvider,
    session_id: &agent_core::ids::SessionId,
    input: PromptInput,
) -> TurnSummary {
    let mut stream = provider.prompt(session_id, input).await.expect("prompt");
    let mut summary = TurnSummary::default();
    let collect = async {
        while let Some(ev) = stream.next().await {
            *summary.event_counts.entry(event_name(&ev)).or_default() += 1;
            match ev {
                AgentEvent::MessageChunk {
                    delta: ContentBlock::Text { text },
                    ..
                } => summary.text.push_str(&text),
                AgentEvent::ToolCall { call, .. } => {
                    let tool = call
                        .tool_name
                        .clone()
                        .unwrap_or_else(|| tool_name_from_title(&call.title));
                    summary.tools.push(tool);
                }
                AgentEvent::ToolCallUpdate { id, patch, .. } => {
                    if let Some(status) = patch.status {
                        summary
                            .tool_statuses
                            .entry(id.as_str().to_string())
                            .or_default()
                            .push(status);
                    }
                }
                AgentEvent::PermissionRequest { request } => {
                    summary.permission_requests += 1;
                    provider
                        .respond(
                            session_id,
                            ClientResponse::Permission {
                                request: request.id,
                                option: "allow_once".into(),
                                feedback: None,
                            },
                        )
                        .await
                        .expect("allow permission once");
                }
                AgentEvent::RunFinished { outcome, .. } => {
                    summary.finished = true;
                    summary.status = Some(outcome.status);
                    summary.run_error = outcome.error;
                    summary.usage = outcome.usage;
                    break;
                }
                AgentEvent::Error { code, message, .. } => {
                    summary.errors.push(format!("{code}: {message}"));
                }
                _ => {}
            }
        }
        summary
    };
    tokio::time::timeout(TURN_TIMEOUT, collect)
        .await
        .expect("live clark-code turn timed out")
}

fn event_name(ev: &AgentEvent) -> &'static str {
    match ev {
        AgentEvent::RunStarted { .. } => "RunStarted",
        AgentEvent::Checkpoint { .. } => "Checkpoint",
        AgentEvent::MessageChunk { .. } => "MessageChunk",
        AgentEvent::MessagePhase { .. } => "MessagePhase",
        AgentEvent::ToolCall { .. } => "ToolCall",
        AgentEvent::ToolCallUpdate { .. } => "ToolCallUpdate",
        AgentEvent::ExecutionChecklistUpdated { .. } => "ExecutionChecklistUpdated",
        AgentEvent::ProposedPlanUpdated { .. } => "ProposedPlanUpdated",
        AgentEvent::GoalUpdated { .. } => "GoalUpdated",
        AgentEvent::PermissionRequest { .. } => "PermissionRequest",
        AgentEvent::Artifact { .. } => "Artifact",
        AgentEvent::Surface { .. } => "Surface",
        AgentEvent::FanOut { .. } => "FanOut",
        AgentEvent::ProviderIncidentUpdated { .. } => "ProviderIncidentUpdated",
        AgentEvent::ModeChanged { .. } => "ModeChanged",
        AgentEvent::ContextCompacted { .. } => "ContextCompacted",
        AgentEvent::Trace { .. } => "Trace",
        AgentEvent::RunFinished { .. } => "RunFinished",
        AgentEvent::Error { .. } => "Error",
    }
}

fn tool_name_from_title(title: &str) -> String {
    title.split(':').next().unwrap_or(title).trim().to_string()
}

fn write_project_fixture(root: &Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        "fn main() {\n    println!(\"CLARK_LIVE_READ_SENTINEL_7391\");\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("notes/alpha.md"),
        "alpha note with CLARK_LIVE_GREP_SENTINEL_5142\n",
    )
    .unwrap();
}

#[tokio::test]
#[ignore = "requires explicit live clark-code env; makes one real DeepSeek V4 Pro call"]
async fn live_clark_code_skills_end_to_end() {
    let Some(cfg) = live_config() else {
        return;
    };
    assert_eq!(
        cfg.model, "clark-code:deepseek_v4_pro",
        "skills_e2e: refusing paid validation with any model except DeepSeek V4 Pro"
    );

    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join(".clark/skills/paid-receipt");
    std::fs::create_dir_all(skill_dir.join("references")).unwrap();
    let contract =
        "skill=paid-receipt\nmodel=deepseek-v4-pro\nsentinel=CLARK_SKILL_RESOURCE_SENTINEL_7319\n";
    std::fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: paid-receipt
description: Produce the paid Clark skill receipt for a live end-to-end validation.
---

# Paid Clark skill receipt

You must call `read_skill` with `skill` set to `paid-receipt` and `resource` set
to `references/contract.md`. Do not use `read_file` for that resource.

Then call `write_file` to create `SKILL_E2E_RECEIPT.md` containing the resource
text verbatim, with no fences or extra text. Finally reply with exactly
`CLARK_SKILL_E2E_DONE_9472`.
"#,
    )
    .unwrap();
    std::fs::write(skill_dir.join("references/contract.md"), contract).unwrap();

    let (mut provider, session) = new_live_provider(
        &cfg,
        dir.path(),
        json!({
            "research": false,
            "memories": false,
            "permissions": {
                "bash": "ask",
                "write_file": "ask",
                "edit_file": "ask"
            }
        }),
    )
    .await;

    let summary = drive_turn(
        &mut provider,
        &session.id,
        "Use $paid-receipt to produce the paid Clark skill receipt. Follow the skill exactly.",
    )
    .await;
    println!(
        "[skills_e2e] model={} tools={:?} permission_requests={} usage={:?} text={:?}",
        cfg.model, summary.tools, summary.permission_requests, summary.usage, summary.text
    );

    summary.require_done("skills_e2e");
    summary.require_tool("skills_e2e", "read_skill");
    summary.require_tool("skills_e2e", "write_file");
    assert!(
        summary.permission_requests >= 1,
        "skills_e2e: expected a write permission request: {summary:?}"
    );
    assert_eq!(
        summary.text.trim(),
        "CLARK_SKILL_E2E_DONE_9472",
        "skills_e2e: model did not return the exact completion receipt"
    );

    let receipt = std::fs::read_to_string(dir.path().join("SKILL_E2E_RECEIPT.md"))
        .expect("skills_e2e: read generated receipt");
    assert_eq!(
        receipt.trim_end(),
        contract.trim_end(),
        "skills_e2e: written receipt did not preserve the skill resource"
    );
    println!("[skills_e2e receipt] {receipt:?}");
}

#[tokio::test]
#[ignore = "requires explicit live clark-code env; makes real model calls"]
async fn live_clark_code_feature_matrix() {
    let Some(cfg) = live_config() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    write_project_fixture(dir.path());

    let (mut provider, session) = new_live_provider(
        &cfg,
        dir.path(),
        json!({
            "research": false,
            "memories": true,
            "permissions": {
                "bash": "ask",
                "write_file": "ask",
                "edit_file": "ask"
            }
        }),
    )
    .await;

    let pong = drive_turn(
        &mut provider,
        &session.id,
        "Reply with exactly this token and no extra words: CLARK_LIVE_PONG_2001",
    )
    .await;
    println!("[pong] {pong:?}");
    pong.require_done("pong");
    assert!(
        pong.text.contains("CLARK_LIVE_PONG_2001"),
        "pong: expected sentinel in assistant text: {:?}",
        pong.text
    );

    let read_search = drive_turn(
        &mut provider,
        &session.id,
        "Use list_dir, glob, grep, and read_file. List the project root, glob for **/*.rs, grep for CLARK_LIVE_GREP_SENTINEL_5142, read src/main.rs, then answer with both CLARK_LIVE_READ_SENTINEL_7391 and CLARK_LIVE_GREP_SENTINEL_5142.",
    )
    .await;
    println!("[read_search] {read_search:?}");
    read_search.require_done("read_search");
    for tool in ["list_dir", "glob", "grep", "read_file"] {
        read_search.require_tool("read_search", tool);
    }
    assert!(read_search.text.contains("CLARK_LIVE_READ_SENTINEL_7391"));
    assert!(read_search.text.contains("CLARK_LIVE_GREP_SENTINEL_5142"));

    let mutate = drive_turn(
        &mut provider,
        &session.id,
        "Use write_file to create live.txt with content `alpha`. Read live.txt. Use edit_file to replace alpha with beta. Then use bash to run `cat live.txt > bash-copy.txt`. Read bash-copy.txt and answer with CLARK_LIVE_MUTATE_DONE.",
    )
    .await;
    println!("[mutate] {mutate:?}");
    mutate.require_done("mutate");
    for tool in ["write_file", "read_file", "edit_file", "bash"] {
        mutate.require_tool("mutate", tool);
    }
    assert!(
        mutate.permission_requests >= 3,
        "mutate: expected write/edit/bash permission prompts, got {mutate:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("live.txt")).unwrap(),
        "beta"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("bash-copy.txt")).unwrap(),
        "beta"
    );

    let memory = drive_turn(
        &mut provider,
        &session.id,
        "Use the memory tool to remember a project fact titled `live e2e sentinel` with content `CLARK_MEMORY_SENTINEL_8402`. Then use memory recall and answer with CLARK_MEMORY_SENTINEL_8402.",
    )
    .await;
    println!("[memory] {memory:?}");
    memory.require_done("memory");
    memory.require_tool("memory", "memory");
    assert!(memory.text.contains("CLARK_MEMORY_SENTINEL_8402"));
    assert!(
        find_file_containing(
            &dir.path().join(".clark/memory"),
            "CLARK_MEMORY_SENTINEL_8402"
        )
        .is_some(),
        "memory: expected project memory file to contain sentinel"
    );
}

#[tokio::test]
#[ignore = "requires explicit live clark-code env; makes real model calls"]
async fn live_clark_code_compacts_and_continues() {
    let Some(cfg) = live_config() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    write_project_fixture(dir.path());

    let (mut provider, session) = new_live_provider(
        &cfg,
        dir.path(),
        json!({
            "research": false,
            "memories": false,
            "auto_compact_token_limit": 2_000,
            "compact_request_token_limit": 1_600,
            "compact_recent_user_token_budget": 400
        }),
    )
    .await;
    let prompt = format!(
        "This turn intentionally exceeds the live compaction threshold. {}\nNow answer with exactly CLARK_LIVE_COMPACTION_DONE_3003.",
        "Important project context. ".repeat(900)
    );
    let summary = drive_turn(&mut provider, &session.id, &prompt).await;
    println!("[compaction] {summary:?}");
    summary.require_done("compaction");
    assert!(
        summary.text.contains("CLARK_LIVE_COMPACTION_DONE_3003"),
        "compaction: expected sentinel after compaction: {:?}",
        summary.text
    );
}

#[tokio::test]
#[ignore = "requires explicit live clark-code env plus CLARK_CODE_LIVE_RESEARCH=1"]
async fn live_clark_code_research_tool() {
    if std::env::var("CLARK_CODE_LIVE_RESEARCH").ok().as_deref() != Some("1") {
        eprintln!("skipping: set CLARK_CODE_LIVE_RESEARCH=1 to permit research tool spend");
        return;
    }
    let Some(cfg) = live_config() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    write_project_fixture(dir.path());
    let (mut provider, session) = new_live_provider(
        &cfg,
        dir.path(),
        json!({
            "research": true,
            "research_model": cfg.model,
            "memories": false
        }),
    )
    .await;
    let research = drive_turn(
        &mut provider,
        &session.id,
        "Use clark_research to check the current official Rust programming language website domain. Then answer with CLARK_RESEARCH_SENTINEL and the domain.",
    )
    .await;
    println!("[research] {research:?}");
    research.require_done("research");
    research.require_tool("research", "clark_research");
    assert!(research.text.contains("CLARK_RESEARCH_SENTINEL"));
    assert!(
        research.text.contains("rust-lang.org"),
        "research: expected rust-lang.org in answer: {:?}",
        research.text
    );
}

#[tokio::test]
#[ignore = "requires explicit live clark-code env plus CLARK_CODE_LIVE_RESEARCH=1"]
async fn live_clark_code_routes_external_research_to_cloud_first() {
    if std::env::var("CLARK_CODE_LIVE_RESEARCH").ok().as_deref() != Some("1") {
        eprintln!("skipping: set CLARK_CODE_LIVE_RESEARCH=1 to permit research tool spend");
        return;
    }
    let Some(cfg) = live_config() else {
        return;
    };
    let scenarios = [
        (
            "commercial_offering",
            "Research how https://vorflux.com structures its commercial offering and summarize the sales motion for a comparable white-glove service.",
        ),
        (
            "library_documentation",
            "Check the current official reqwest documentation and report the supported API for configuring per-request timeouts.",
        ),
        (
            "service_outage",
            "Investigate whether GitHub is currently reporting a service outage or active incident and summarize the evidence.",
        ),
        (
            "single_url",
            "Read https://www.rust-lang.org/ and briefly summarize what the current page says.",
        ),
    ];

    for (label, prompt) in scenarios {
        let dir = tempfile::tempdir().unwrap();
        write_project_fixture(dir.path());
        let (mut provider, session) = new_live_provider(
            &cfg,
            dir.path(),
            json!({
                "research": true,
                "research_model": cfg.model,
                "memories": false
            }),
        )
        .await;
        let summary = drive_turn(&mut provider, &session.id, prompt).await;
        println!("[{label}] {summary:?}");
        summary.require_done(label);
        summary.require_cloud_research_first(label);
    }
}

/// Build a tiny, valid, solid-color PNG (`size`x`size`) with no external
/// image-encoding crate: one uncompressed ("stored") deflate block, so every
/// byte is exact and independently checksummed (Adler-32 for zlib, CRC-32 per
/// PNG chunk) rather than a hand-typed/memorized base64 blob that might not
/// decode.
fn solid_color_png(size: u32, rgb: [u8; 3]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &b in bytes {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        crc ^ 0xFFFF_FFFF
    }

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        let mut out = Vec::with_capacity(4 + body.len() + 4);
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    }

    // One scanline: a leading filter-type byte (0 = None) + `size` RGB pixels.
    // Every row is identical for a solid color, so build it once and repeat.
    let mut row = vec![0u8];
    for _ in 0..size {
        row.extend_from_slice(&rgb);
    }
    let mut raw = Vec::with_capacity(row.len() * size as usize);
    for _ in 0..size {
        raw.extend_from_slice(&row);
    }

    // zlib-wrap `raw` in a single uncompressed ("stored") deflate block —
    // avoids needing a real compressor for a handful of solid-color bytes.
    // Safe up to a 65,535-byte scanline buffer (a single stored block's
    // limit); `size` here is tiny, so this always fits in one block.
    let mut zlib = vec![0x78, 0x01]; // zlib header (deflate, 32K window, fastest level)
    zlib.push(0x01); // BFINAL=1, BTYPE=00 (stored), byte-aligned
    let len = raw.len() as u16;
    zlib.extend_from_slice(&len.to_le_bytes());
    zlib.extend_from_slice(&(!len).to_le_bytes());
    zlib.extend_from_slice(&raw);
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // depth 8, color type 2 (RGB), default comp/filter/interlace

    let mut png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend(chunk(b"IHDR", &ihdr));
    png.extend(chunk(b"IDAT", &zlib));
    png.extend(chunk(b"IEND", &[]));
    png
}

#[tokio::test]
#[ignore = "requires explicit live clark-code env plus CLARK_CODE_LIVE_VISION=1"]
async fn live_clark_code_vision_fallback_describes_an_attached_image() {
    if std::env::var("CLARK_CODE_LIVE_VISION").ok().as_deref() != Some("1") {
        eprintln!("skipping: set CLARK_CODE_LIVE_VISION=1 to permit vision-fallback spend");
        return;
    }
    let Some(cfg) = live_config() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    write_project_fixture(dir.path());
    let (mut provider, session) = new_live_provider(
        &cfg,
        dir.path(),
        json!({ "research": false, "memories": false }),
    )
    .await;

    let png = solid_color_png(8, [255, 0, 0]);
    let input = PromptInput {
        blocks: vec![ContentBlock::text(
            "What color is the attached image? Answer with exactly one word for the color, \
             then the sentinel CLARK_VISION_SENTINEL_6210.",
        )],
        attachments: vec![PendingUpload {
            filename: "swatch.png".into(),
            content_type: "image/png".into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(&png),
        }],
    };
    let vision = drive_prompt(&mut provider, &session.id, input).await;
    println!("[vision] {vision:?}");
    vision.require_done("vision");
    assert!(
        vision.text.contains("CLARK_VISION_SENTINEL_6210"),
        "vision: expected sentinel in assistant text: {:?}",
        vision.text
    );
    assert!(
        vision.text.to_lowercase().contains("red"),
        "vision: expected the coding model to relay the vision model's color \
         description (red) despite neither being vision-capable itself: {:?}",
        vision.text
    );
}

fn find_file_containing(root: &Path, needle: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_containing(&path, needle) {
                return Some(found);
            }
        } else if std::fs::read_to_string(&path)
            .ok()
            .is_some_and(|text| text.contains(needle))
        {
            return Some(path);
        }
    }
    None
}
