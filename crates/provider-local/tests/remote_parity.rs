//! Phase 2 verification: drive a real `clark-exec-server` over a loopback
//! WebSocket and prove (a) `RemoteExecutor` is behaviorally identical to
//! `LocalExecutor`, (b) a bad token is rejected, and (c) process output survives
//! a dropped-and-reopened connection via `process/resume`.

use std::path::PathBuf;
use std::time::Duration;

use agent_orchestration::{
    EnterpriseId, EnterpriseSigningKey, EnterpriseTrustChain, EnterpriseTrustManifest,
};
use exec_protocol::{
    method, AuthParams, Notification, ProcessResumeParams, ProcessStartParams, Request, Response,
    Stream as WireStream, PROTOCOL_VERSION,
};
use futures::{SinkExt, StreamExt};
use provider_local::{
    discover_skill_catalog_snapshot, install_skill_pack, uninstall_skill_pack,
    InstallSkillPackRequest, SkillPackAction, SkillPackScope,
};
use provider_local::{Executor, LocalExecutor, RemoteExecutor};
use scout_adapter_runtime::{
    CensusRequest as AdapterCensusRequest, CensusResponse as AdapterCensusResponse,
    ScoutAdapterRequest, ScoutAdapterResponse, SERVICE_NAME as SCOUT_ADAPTER_SERVICE,
};
use scout_store::{ScoutStoreRequest, ScoutStoreResponse, SERVICE_NAME as SCOUT_STORE_SERVICE};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "test-capability-token";

fn platform_command(posix: &'static str, powershell: &'static str) -> &'static str {
    if cfg!(windows) {
        powershell
    } else {
        posix
    }
}

/// Bind an ephemeral server, run it in the background, return its `ws://` URL.
async fn start_server(root: Option<PathBuf>) -> String {
    let server = exec_server::bind(exec_server::Config {
        token: TOKEN.to_string(),
        root,
        home: None,
        addr: "127.0.0.1:0".to_string(),
    })
    .await
    .expect("bind exec-server");
    let addr = server.local_addr().expect("local_addr");
    tokio::spawn(server.serve());
    format!("ws://{addr}")
}

#[tokio::test]
async fn remote_matches_local_for_every_primitive() {
    let dir = tempfile::tempdir().unwrap();
    let url = start_server(None).await;
    let remote = RemoteExecutor::connect(&url, TOKEN).await.expect("connect");
    let local = LocalExecutor;

    // write + read round-trip (binary-safe).
    let file = dir.path().join("nested/a.txt");
    let bytes = vec![0u8, 159, 146, 150, b'h', b'i'];
    remote.write(&file, &bytes).await.unwrap();
    assert_eq!(remote.read(&file).await.unwrap(), bytes);
    assert_eq!(local.read(&file).await.unwrap(), bytes, "landed on disk");
    remote.sync_file(&file).await.unwrap();
    remote.sync_directory(file.parent().unwrap()).await.unwrap();

    let private = dir.path().join("private/signing.key");
    remote
        .write_private(&private, b"host-held-seed")
        .await
        .unwrap();
    assert!(!remote
        .write_private_new(&private, b"replacement")
        .await
        .unwrap());
    assert_eq!(local.read(&private).await.unwrap(), b"host-held-seed");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    let service_error = remote
        .target_service_call("unsupported-test-service", dir.path(), b"{}")
        .await
        .unwrap_err();
    assert!(service_error.contains("unsupported target service"));

    let scout_root = dir.path().join("scout-index");
    std::fs::create_dir_all(scout_root.join("trust")).unwrap();
    std::fs::create_dir_all(scout_root.join("batches")).unwrap();
    std::fs::create_dir_all(scout_root.join("private")).unwrap();
    let enterprise = EnterpriseId::new("remote-parity-enterprise").unwrap();
    let coordinator = EnterpriseSigningKey::from_seed([0x55; 32]);
    let root = EnterpriseTrustManifest::initial(
        enterprise.clone(),
        "trust:00000000-0000-4000-8000-000000000055".into(),
        1,
        1_000_000,
        &coordinator,
    )
    .unwrap();
    let chain = EnterpriseTrustChain {
        anchor_manifest_id: root.manifest_id.clone(),
        manifests: vec![root],
    };
    std::fs::write(
        scout_root.join("trust/chain.json"),
        serde_json::to_vec(&chain).unwrap(),
    )
    .unwrap();
    std::fs::write(
        scout_root.join("private/anchor-manifest-id"),
        chain.anchor_manifest_id.as_bytes(),
    )
    .unwrap();
    let request = serde_json::to_vec(&ScoutStoreRequest::Rebuild {
        enterprise_id: enterprise,
    })
    .unwrap();
    let response = remote
        .target_service_call(SCOUT_STORE_SERVICE, &scout_root, &request)
        .await
        .unwrap();
    let response: ScoutStoreResponse = serde_json::from_slice(&response).unwrap();
    let ScoutStoreResponse::Rebuilt(receipt) = response else {
        panic!("wrong remote Scout index response");
    };
    assert!(receipt.rebuilt);

    let adapter_root = dir.path().join("scout-adapter-private");
    let request =
        serde_json::to_vec(&ScoutAdapterRequest::Census(AdapterCensusRequest::default())).unwrap();
    let response = remote
        .target_service_call(SCOUT_ADAPTER_SERVICE, &adapter_root, &request)
        .await
        .unwrap();
    let response: ScoutAdapterResponse = serde_json::from_slice(&response).unwrap();
    let ScoutAdapterResponse::Census(AdapterCensusResponse::Succeeded { target, .. }) = response
    else {
        panic!("wrong remote Scout adapter response");
    };
    assert!(target.target_id.as_str().starts_with("target:"));
    assert!(adapter_root.join("vault.json").is_file());

    // metadata parity.
    let rm = remote.metadata(&file).await.unwrap();
    let lm = local.metadata(&file).await.unwrap();
    assert_eq!(rm.len, lm.len);
    assert_eq!(rm.is_dir, lm.is_dir);
    assert_eq!(rm.is_symlink, lm.is_symlink);
    assert_eq!(rm.len, bytes.len() as u64);

    // create_dir_all + read_dir parity.
    remote
        .create_dir_all(&dir.path().join("sub"))
        .await
        .unwrap();
    let mut rd: Vec<_> = remote
        .read_dir(dir.path())
        .await
        .unwrap()
        .into_iter()
        .map(|e| (e.name, e.is_dir, e.is_symlink))
        .collect();
    rd.sort();
    assert!(rd.contains(&("nested".to_string(), true, false)));
    assert!(rd.contains(&("sub".to_string(), true, false)));

    let renamed = dir.path().join("nested/renamed.txt");
    remote.rename(&file, &renamed).await.unwrap();
    assert!(!file.exists());
    assert_eq!(remote.read(&renamed).await.unwrap(), bytes);
    assert_eq!(
        remote.canonicalize(&renamed).await.unwrap(),
        local.canonicalize(&renamed).await.unwrap()
    );

    let removed_file = dir.path().join("remove/file.txt");
    remote.write(&removed_file, b"remove me").await.unwrap();
    remote.remove_file(&removed_file).await.unwrap();
    assert!(!removed_file.exists());
    remote
        .create_dir_all(&dir.path().join("remove/tree/nested"))
        .await
        .unwrap();
    remote
        .remove_dir_all(&dir.path().join("remove/tree"))
        .await
        .unwrap();
    assert!(!dir.path().join("remove/tree").exists());

    // walk parity — same file set, ignored dirs skipped.
    std::fs::create_dir_all(dir.path().join("node_modules/x")).unwrap();
    std::fs::write(dir.path().join("node_modules/x/y.js"), "").unwrap();
    let rel = |paths: Vec<PathBuf>| {
        let mut v: Vec<String> = paths
            .into_iter()
            .map(|p| {
                p.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        v.sort();
        v
    };
    let remote_walk = rel(remote
        .walk(dir.path())
        .await
        .unwrap()
        .into_iter()
        .map(|w| w.path)
        .collect());
    let local_walk = rel(local
        .walk(dir.path())
        .await
        .unwrap()
        .into_iter()
        .map(|w| w.path)
        .collect());
    assert_eq!(remote_walk, local_walk);
    assert!(remote_walk.iter().any(|p| p == "nested/renamed.txt"));
    assert!(!remote_walk.iter().any(|p| p.contains("node_modules")));

    // exec parity — output + exit code.
    let cancel = CancellationToken::new();
    let out = remote
        .exec(
            platform_command(
                "echo out; echo err 1>&2; exit 7",
                r#"[Console]::Out.WriteLine("out"); [Console]::Error.WriteLine("err"); exit 7"#,
            ),
            dir.path(),
            Duration::from_secs(10),
            &cancel,
        )
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "out");
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "err");
    assert_eq!(out.code, Some(7));

    let terminal = remote
        .exec_streaming_pty(
            platform_command(
                "test -t 0 && test -t 1 && printf terminal",
                r#"if (-not [Console]::IsInputRedirected -and -not [Console]::IsOutputRedirected) { [Console]::Out.Write("terminal"); exit 0 }; exit 1"#,
            ),
            dir.path(),
            Duration::from_secs(10),
            &cancel,
            &|_, _| {},
        )
        .await
        .unwrap();
    assert_eq!(terminal.code, Some(0));
    assert!(String::from_utf8_lossy(&terminal.stdout).contains("terminal"));
}

#[tokio::test]
async fn remote_managed_skill_pack_survives_reconnect_update_and_uninstall() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("remote-project");
    let source = project.join("fixtures/superpowers/skills/brainstorming");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("SKILL.md"),
        "---\nname: brainstorming\ndescription: Explore requirements\n---\n\nRemote v1.\n",
    )
    .unwrap();
    let url = start_server(Some(project.clone())).await;
    let remote = RemoteExecutor::connect(&url, TOKEN).await.unwrap();

    let installed = install_skill_pack(
        &remote,
        &project,
        InstallSkillPackRequest {
            pack_id: "superpowers".into(),
            source_path: project
                .join("fixtures/superpowers")
                .to_string_lossy()
                .into_owned(),
            scope: SkillPackScope::Project,
        },
    )
    .await
    .unwrap();
    assert_eq!(installed.action, SkillPackAction::Installed);

    let reconnected = RemoteExecutor::connect(&url, TOKEN).await.unwrap();
    let snapshot = discover_skill_catalog_snapshot(
        &reconnected,
        &project,
        "remote:test",
        &std::collections::HashSet::new(),
        &[],
    )
    .await;
    assert!(snapshot
        .skills
        .iter()
        .any(|skill| skill.name == "brainstorming" && skill.enabled));

    std::fs::write(
        source.join("SKILL.md"),
        "---\nname: brainstorming\ndescription: Explore requirements\n---\n\nRemote v2.\n",
    )
    .unwrap();
    let updated = install_skill_pack(
        &reconnected,
        &project,
        InstallSkillPackRequest {
            pack_id: "superpowers".into(),
            source_path: project
                .join("fixtures/superpowers")
                .to_string_lossy()
                .into_owned(),
            scope: SkillPackScope::Project,
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.action, SkillPackAction::Updated);

    let removed = uninstall_skill_pack(
        &reconnected,
        &project,
        "superpowers",
        SkillPackScope::Project,
    )
    .await
    .unwrap();
    assert_eq!(removed.action, SkillPackAction::Uninstalled);
    let after = discover_skill_catalog_snapshot(
        &reconnected,
        &project,
        "remote:test",
        &std::collections::HashSet::new(),
        &[],
    )
    .await;
    assert!(!after
        .skills
        .iter()
        .any(|skill| skill.name == "brainstorming"));
}

#[tokio::test]
async fn remote_catalog_uses_the_target_home_for_personal_skills() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let home = temp.path().join("remote-home");
    let skill = home.join(".agents/skills/superpowers/brainstorming/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        &skill,
        "---\nname: brainstorming\ndescription: Explore requirements\n---\n\nPersonal remote skill.\n",
    )
    .unwrap();
    let server = exec_server::bind(exec_server::Config {
        token: TOKEN.to_string(),
        root: Some(project.clone()),
        home: Some(home.clone()),
        addr: "127.0.0.1:0".to_string(),
    })
    .await
    .unwrap();
    let address = server.local_addr().unwrap();
    tokio::spawn(server.serve());
    let remote = RemoteExecutor::connect(&format!("ws://{address}"), TOKEN)
        .await
        .unwrap();

    assert_eq!(
        remote.home_dir(&project).await.unwrap(),
        home.canonicalize().unwrap()
    );
    let snapshot = discover_skill_catalog_snapshot(
        &remote,
        &project,
        "remote:personal",
        &std::collections::HashSet::new(),
        &[],
    )
    .await;
    let brainstorming = snapshot
        .skills
        .iter()
        .find(|skill| skill.name == "brainstorming")
        .unwrap();
    assert_eq!(brainstorming.scope, provider_local::SkillScope::User);
    assert_eq!(
        brainstorming.origin,
        provider_local::SkillOrigin::Compatible
    );
}

#[tokio::test]
async fn remote_background_process_accepts_input_and_reports_exit() {
    let dir = tempfile::tempdir().unwrap();
    let url = start_server(Some(dir.path().to_path_buf())).await;
    let remote = RemoteExecutor::connect(&url, TOKEN).await.unwrap();
    let process = remote
        .background_start(
            platform_command(
                "read value; printf 'remote:%s' \"$value\"",
                r#"$value = [Console]::In.ReadLine(); [Console]::Out.Write("remote:$value")"#,
            ),
            dir.path(),
        )
        .await
        .unwrap();
    remote
        .background_write(&process, b"hello\n", true)
        .await
        .unwrap();

    let mut cursor = 0;
    let mut output = Vec::new();
    let mut exit = None;
    let attempts = if cfg!(windows) { 500 } else { 100 };
    for _ in 0..attempts {
        let status = remote.background_status(&process, cursor).await.unwrap();
        cursor = status.cursor;
        for chunk in status.output {
            output.extend_from_slice(&chunk.data);
        }
        if status.exit_code.is_some() {
            exit = status.exit_code;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(exit, Some(Some(0)));
    assert_eq!(String::from_utf8_lossy(&output), "remote:hello");
}

#[tokio::test]
async fn remote_background_output_is_bounded_and_reports_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let url = start_server(Some(dir.path().to_path_buf())).await;
    let remote = RemoteExecutor::connect(&url, TOKEN).await.unwrap();
    let process = remote
        .background_start(
            platform_command(
                "yes x | head -c 1100000",
                r#"[Console]::Out.Write("x" * 1100000)"#,
            ),
            dir.path(),
        )
        .await
        .unwrap();
    let attempts = if cfg!(windows) { 500 } else { 100 };
    for _ in 0..attempts {
        let status = remote.background_status(&process, 0).await.unwrap();
        if status.exit_code.is_some() {
            let bytes = status
                .output
                .iter()
                .map(|chunk| chunk.data.len())
                .sum::<usize>();
            assert!(status.truncated);
            assert!(bytes <= 1_048_576, "retained {bytes} bytes");
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("large remote background process never finished");
}

#[tokio::test]
async fn exec_honors_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let url = start_server(None).await;
    let remote = RemoteExecutor::connect(&url, TOKEN).await.unwrap();

    let cancel = CancellationToken::new();
    cancel.cancel();
    let err = remote
        .exec(
            platform_command("sleep 5", "Start-Sleep -Seconds 5"),
            dir.path(),
            Duration::from_secs(10),
            &cancel,
        )
        .await
        .unwrap_err();
    assert!(err.contains("cancel"), "{err}");
}

#[cfg(unix)]
#[tokio::test]
async fn exec_timeout_kills_remote_descendants_and_returns_terminal_error() {
    let dir = tempfile::tempdir().unwrap();
    let url = start_server(None).await;
    let remote = RemoteExecutor::connect(&url, TOKEN).await.unwrap();

    let err = tokio::time::timeout(
        Duration::from_secs(2),
        remote.exec(
            "sleep 30 & echo $! > descendant.pid; wait",
            dir.path(),
            Duration::from_millis(150),
            &CancellationToken::new(),
        ),
    )
    .await
    .expect("remote executor must return after its own timeout")
    .unwrap_err();
    assert!(err.contains("timed out"), "{err}");

    let pid: u32 = std::fs::read_to_string(dir.path().join("descendant.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    for _ in 0..20 {
        let exists = std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !exists {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("remote descendant process {pid} survived command timeout");
}

#[tokio::test]
async fn remote_terminal_timeout_returns_a_terminal_error() {
    let dir = tempfile::tempdir().unwrap();
    let url = start_server(None).await;
    let remote = RemoteExecutor::connect(&url, TOKEN).await.unwrap();
    let err = tokio::time::timeout(
        Duration::from_secs(2),
        remote.exec_streaming_pty(
            platform_command("sleep 30", "Start-Sleep -Seconds 30"),
            dir.path(),
            Duration::from_millis(150),
            &CancellationToken::new(),
            &|_, _| {},
        ),
    )
    .await
    .expect("remote terminal must return after its own timeout")
    .unwrap_err();
    assert!(err.contains("timed out"), "{err}");
}

#[tokio::test]
async fn bad_token_is_rejected() {
    let url = start_server(None).await;
    let err = match RemoteExecutor::connect(&url, "wrong-token").await {
        Ok(_) => panic!("a bad token must be rejected"),
        Err(e) => e,
    };
    assert!(err.to_lowercase().contains("token"), "{err}");
}

#[tokio::test]
async fn version_mismatch_is_rejected() {
    let url = start_server(None).await;
    let mut ws = tokio_tungstenite::connect_async(&url).await.unwrap().0;
    let auth = Request::new(
        1,
        method::AUTH,
        serde_json::to_value(AuthParams {
            token: TOKEN.to_string(),
            protocol_version: PROTOCOL_VERSION + 99,
        })
        .unwrap(),
    );
    send(&mut ws, &auth).await;
    let resp = recv_response(&mut ws).await;
    let err = resp.error.expect("version mismatch should error");
    assert!(err.message.contains("protocol version"), "{}", err.message);
}

#[tokio::test]
async fn root_containment_rejects_escape() {
    let dir = tempfile::tempdir().unwrap();
    let url = start_server(Some(dir.path().to_path_buf())).await;
    let remote = RemoteExecutor::connect(&url, TOKEN).await.unwrap();

    // Inside the root: fine.
    let inside = dir.path().join("ok.txt");
    remote.write(&inside, b"hi").await.unwrap();

    // Outside the root (via ..): refused by the server's lexical containment.
    let outside = dir.path().join("../escape.txt");
    let err = remote.write(&outside, b"nope").await.unwrap_err();
    assert!(err.contains("escapes project root"), "{err}");
}

#[tokio::test]
async fn output_survives_reconnect_via_resume() {
    let dir = tempfile::tempdir().unwrap();
    let url = start_server(None).await;
    let process_id = "resume-test-proc".to_string();
    // Three chunks separated in time, so they get distinct sequence numbers.
    let command = platform_command(
        "printf 'A\\n'; sleep 0.2; printf 'B\\n'; sleep 0.2; printf 'C\\n'",
        r#"[Console]::Out.WriteLine("A"); Start-Sleep -Milliseconds 200; [Console]::Out.WriteLine("B"); Start-Sleep -Milliseconds 200; [Console]::Out.WriteLine("C")"#,
    );

    // --- Connection 1: start the process, read until the first chunk, then drop.
    let last_seq;
    {
        let mut ws = tokio_tungstenite::connect_async(&url).await.unwrap().0;
        authenticate(&mut ws).await;
        let start = Request::new(
            2,
            method::PROCESS_START,
            serde_json::to_value(ProcessStartParams {
                process_id: process_id.clone(),
                command: command.to_string(),
                cwd: dir.path().to_string_lossy().to_string(),
                timeout_ms: 10_000,
                pty: false,
            })
            .unwrap(),
        );
        send(&mut ws, &start).await;
        // Read frames until we observe the first "A" output chunk.
        last_seq = read_until_chunk(&mut ws, "A").await;
        assert!(last_seq >= 1, "saw the first chunk");
        // Drop ws here (process keeps running on the server).
    }

    // --- Connection 2: re-auth, resume from last_seq, collect the rest + exit.
    let mut ws = tokio_tungstenite::connect_async(&url).await.unwrap().0;
    authenticate(&mut ws).await;
    let resume = Request::new(
        3,
        method::PROCESS_RESUME,
        serde_json::to_value(ProcessResumeParams {
            process_id: process_id.clone(),
            after_seq: last_seq,
        })
        .unwrap(),
    );
    send(&mut ws, &resume).await;

    let mut resumed = String::new();
    let exit_code = loop {
        match recv_frame(&mut ws).await {
            Frame::Note(n) if n.method == method::PROCESS_OUTPUT => {
                let p: exec_protocol::ProcessOutputParams =
                    serde_json::from_value(n.params).unwrap();
                assert!(
                    p.seq > last_seq,
                    "resume must not replay already-seen output"
                );
                if p.stream == WireStream::Stdout {
                    resumed.push_str(&String::from_utf8_lossy(
                        &exec_protocol::b64_decode(&p.data).unwrap(),
                    ));
                }
            }
            Frame::Note(n) if n.method == method::PROCESS_EXIT => {
                let p: exec_protocol::ProcessExitParams = serde_json::from_value(n.params).unwrap();
                break p.code;
            }
            _ => {}
        }
    };

    // B and C arrived after the reconnect; A (already seen) was not replayed.
    assert!(
        resumed.contains('B') && resumed.contains('C'),
        "resumed tail = {resumed:?}"
    );
    assert!(
        !resumed.contains('A'),
        "A should not be replayed; got {resumed:?}"
    );
    assert_eq!(exit_code, Some(0));
}

// ---- raw-protocol test helpers ---------------------------------------------

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

enum Frame {
    Resp(Response),
    Note(Notification),
}

async fn send(ws: &mut Ws, req: &Request) {
    ws.send(Message::Text(serde_json::to_string(req).unwrap().into()))
        .await
        .unwrap();
}

async fn recv_frame(ws: &mut Ws) -> Frame {
    loop {
        let Some(Ok(msg)) = ws.next().await else {
            panic!("connection closed before a frame arrived");
        };
        if let Message::Text(t) = msg {
            if let Ok(r) = serde_json::from_str::<Response>(&t) {
                return Frame::Resp(r);
            }
            if let Ok(n) = serde_json::from_str::<Notification>(&t) {
                return Frame::Note(n);
            }
        }
    }
}

async fn recv_response(ws: &mut Ws) -> Response {
    loop {
        if let Frame::Resp(r) = recv_frame(ws).await {
            return r;
        }
    }
}

async fn authenticate(ws: &mut Ws) {
    let auth = Request::new(
        1,
        method::AUTH,
        serde_json::to_value(AuthParams {
            token: TOKEN.to_string(),
            protocol_version: PROTOCOL_VERSION,
        })
        .unwrap(),
    );
    send(ws, &auth).await;
    let resp = recv_response(ws).await;
    assert!(resp.error.is_none(), "auth failed: {:?}", resp.error);
}

/// Read frames until a stdout output chunk containing `needle` is seen; returns
/// the highest output `seq` observed so far (the resume cursor).
async fn read_until_chunk(ws: &mut Ws, needle: &str) -> u64 {
    let mut max_seq = 0;
    loop {
        if let Frame::Note(n) = recv_frame(ws).await {
            if n.method == method::PROCESS_OUTPUT {
                let p: exec_protocol::ProcessOutputParams =
                    serde_json::from_value(n.params).unwrap();
                max_seq = max_seq.max(p.seq);
                let text = String::from_utf8_lossy(&exec_protocol::b64_decode(&p.data).unwrap())
                    .to_string();
                if text.contains(needle) {
                    return max_seq;
                }
            }
        }
    }
}
