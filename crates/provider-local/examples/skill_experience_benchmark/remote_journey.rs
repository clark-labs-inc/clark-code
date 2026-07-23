use std::collections::HashSet;
use std::path::Path;

use provider_local::{
    discover_skill_catalog_snapshot, install_skill_pack, list_skill_packs, uninstall_skill_pack,
    InstallSkillPackRequest, RemoteExecutor, SkillCatalogEntry, SkillOrigin, SkillPackAction,
    SkillPackScope, SkillScope,
};
use serde_json::json;

use crate::fixture;
use crate::model::{error, evidence, require, DynError, Recorder};
use crate::provider_harness::{self, RemoteSpec};

const REMOTE_TOKEN: &str = "read-benchmark-remote-capability";

pub async fn run(source: &Path, output: &Path, recorder: &mut Recorder) -> Result<(), DynError> {
    let home = output.join("fake-empty-remote-user");
    let project = output.join("remote-project");
    let fixture_root = project.join("fixtures/superpowers");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&project)?;

    let (remote, spec) = recorder
        .step(
            "remote_empty_home_and_connect",
            "Start a loopback remote host with its own empty target home",
            || async {
                require(
                    std::fs::read_dir(&home)?.next().is_none(),
                    "remote home was not empty",
                )?;
                fixture::copy_tree(source, &fixture_root)?;
                std::fs::create_dir_all(home.join(".clark"))?;
                std::fs::write(
                    home.join(".clark/AGENTS.md"),
                    "READ_BENCH_REMOTE_PERSONAL_INSTRUCTION\n",
                )?;
                std::fs::write(
                    project.join("AGENTS.md"),
                    "READ_BENCH_REMOTE_PROJECT_INSTRUCTION\n",
                )?;
                let server = exec_server::bind(exec_server::Config {
                    token: REMOTE_TOKEN.to_string(),
                    root: Some(project.clone()),
                    home: Some(home.clone()),
                    addr: "127.0.0.1:0".into(),
                })
                .await?;
                let address = server.local_addr()?;
                tokio::spawn(server.serve());
                let ws_url = format!("ws://{address}");
                let remote = RemoteExecutor::connect(&ws_url, REMOTE_TOKEN).await?;
                let target_home = provider_local::Executor::home_dir(&remote, &project).await?;
                require(
                    target_home == home.canonicalize()?,
                    "remote executor resolved the desktop home instead of target home",
                )?;
                let spec = RemoteSpec {
                    ws_url,
                    token: REMOTE_TOKEN.into(),
                    cwd: project.to_string_lossy().into_owned(),
                };
                Ok((
                    (remote, spec),
                    evidence([
                        ("targetHome", json!(target_home)),
                        ("projectRoot", json!(project)),
                        ("transport", json!("loopback_websocket")),
                    ]),
                ))
            },
        )
        .await?;

    let old_skill = recorder
        .step(
            "remote_install_discover",
            "Install the same user pack on the remote target and reconnect to discover it",
            || async {
                let receipt = install_skill_pack(
                    &remote,
                    &project,
                    InstallSkillPackRequest {
                        pack_id: "superpowers".into(),
                        source_path: fixture_root.to_string_lossy().into_owned(),
                        scope: SkillPackScope::User,
                    },
                )
                .await?;
                require(
                    receipt.action == SkillPackAction::Installed,
                    "remote pack not installed",
                )?;
                let reconnected = RemoteExecutor::connect(&spec.ws_url, &spec.token).await?;
                let catalog =
                    remote_snapshot(&reconnected, &project, "remote:read-benchmark").await;
                let selected = remote_brainstorming(&catalog.skills)?.clone();
                let packs = list_skill_packs(&reconnected, &project).await?;
                require(
                    packs.len() == 1,
                    "remote registry did not survive reconnect",
                )?;
                Ok((
                    selected.clone(),
                    evidence([
                        ("packRevision", json!(receipt.revision)),
                        ("skillCount", json!(receipt.skill_count)),
                        ("selectedId", json!(selected.id)),
                        ("selectedRevision", json!(selected.revision)),
                        ("reconnected", json!(true)),
                    ]),
                ))
            },
        )
        .await?;

    let (mut provider, remote_turn) = recorder
        .step(
            "remote_provider_boundary_v1",
            "Load the remote skill body through the real provider boundary",
            || async {
                let (active, turn) = provider_harness::launch_and_prompt(
                    &project,
                    Some(&spec),
                    &old_skill,
                    &[
                        "# Brainstorming Ideas Into Designs",
                        "READ_BENCH_REMOTE_PERSONAL_INSTRUCTION",
                        "READ_BENCH_REMOTE_PROJECT_INSTRUCTION",
                    ],
                )
                .await?;
                Ok((
                    (active, turn),
                    evidence([
                        ("selectedId", json!(old_skill.id)),
                        ("selectedRevision", json!(old_skill.revision)),
                        ("scriptedModelCalls", json!(1)),
                    ]),
                ))
            },
        )
        .await?;
    require(
        remote_turn.event_count > 0,
        "remote provider emitted no events",
    )?;

    let updated_skill = recorder
        .step(
            "remote_update_and_stale_rejection",
            "Update remotely, preserve identity, and reject the old revision before dispatch",
            || async {
                fixture::append_update_marker(&fixture_root, "READ_BENCH_REMOTE_V2")?;
                let reconnected = RemoteExecutor::connect(&spec.ws_url, &spec.token).await?;
                let receipt = install_skill_pack(
                    &reconnected,
                    &project,
                    InstallSkillPackRequest {
                        pack_id: "superpowers".into(),
                        source_path: fixture_root.to_string_lossy().into_owned(),
                        scope: SkillPackScope::User,
                    },
                )
                .await?;
                require(
                    receipt.action == SkillPackAction::Updated,
                    "remote update not detected",
                )?;
                let catalog =
                    remote_snapshot(&reconnected, &project, "remote:read-benchmark").await;
                let updated = remote_brainstorming(&catalog.skills)?.clone();
                require(updated.id == old_skill.id, "remote managed id changed")?;
                require(
                    updated.revision != old_skill.revision,
                    "remote revision did not change",
                )?;
                let rejection = provider_harness::expect_binding_rejected(
                    &mut provider,
                    &old_skill,
                    "select it again before sending",
                )
                .await?;
                Ok((
                    updated.clone(),
                    evidence([
                        ("stableId", json!(updated.id)),
                        ("previousRevision", json!(old_skill.revision)),
                        ("currentRevision", json!(updated.revision)),
                        ("rejection", json!(rejection)),
                        ("modelCallsForRejectedTurn", json!(0)),
                    ]),
                ))
            },
        )
        .await?;

    let (mut restarted, _) = recorder
        .step(
            "remote_restart_updated_body",
            "Reconnect and restart against the updated remote revision",
            || async {
                let reconnected = RemoteExecutor::connect(&spec.ws_url, &spec.token).await?;
                let catalog =
                    remote_snapshot(&reconnected, &project, "remote:read-benchmark-restart").await;
                let selected = remote_brainstorming(&catalog.skills)?;
                require(
                    selected.id == updated_skill.id && selected.revision == updated_skill.revision,
                    "remote restart discovered the wrong revision",
                )?;
                let (active, turn) = provider_harness::launch_and_prompt(
                    &project,
                    Some(&spec),
                    selected,
                    &["READ_BENCH_REMOTE_V2"],
                )
                .await?;
                Ok((
                    (active, turn),
                    evidence([
                        ("catalogRevision", json!(catalog.revision)),
                        ("updatedMarkerLoaded", json!(true)),
                    ]),
                ))
            },
        )
        .await?;

    recorder
        .step(
            "remote_uninstall",
            "Deactivate and remove the remote pack, then reject its last binding",
            || async {
                let reconnected = RemoteExecutor::connect(&spec.ws_url, &spec.token).await?;
                let receipt = uninstall_skill_pack(
                    &reconnected,
                    &project,
                    "superpowers",
                    SkillPackScope::User,
                )
                .await?;
                require(
                    receipt.action == SkillPackAction::Uninstalled,
                    "remote uninstall failed",
                )?;
                require(
                    receipt.warnings.is_empty(),
                    "remote uninstall had cleanup warnings",
                )?;
                let packs = list_skill_packs(&reconnected, &project).await?;
                require(packs.is_empty(), "remote registry retained the pack")?;
                let after = remote_snapshot(&reconnected, &project, "remote:read-benchmark").await;
                require(
                    !after
                        .skills
                        .iter()
                        .any(|skill| skill.id == updated_skill.id),
                    "remote catalog retained the managed skill",
                )?;
                let rejection = provider_harness::expect_binding_rejected(
                    &mut restarted,
                    &updated_skill,
                    "is not available",
                )
                .await?;
                Ok((
                    (),
                    evidence([
                        ("remainingManagedPacks", json!(packs.len())),
                        ("rejection", json!(rejection)),
                        ("modelCallsForRejectedTurn", json!(0)),
                    ]),
                ))
            },
        )
        .await
}

async fn remote_snapshot(
    remote: &RemoteExecutor,
    project: &Path,
    environment: &str,
) -> provider_local::SkillCatalogSnapshot {
    discover_skill_catalog_snapshot(remote, project, environment, &HashSet::new(), &[]).await
}

fn remote_brainstorming(skills: &[SkillCatalogEntry]) -> Result<&SkillCatalogEntry, DynError> {
    skills
        .iter()
        .find(|skill| {
            skill.name == "brainstorming"
                && skill.scope == SkillScope::User
                && skill.origin == SkillOrigin::Clark
        })
        .ok_or_else(|| error("remote managed brainstorming skill was not discoverable"))
}
