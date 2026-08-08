use std::collections::HashSet;
use std::path::Path;

use agent_core::domain::ContentBlock;
use provider_local::{
    discover_instructions, discover_skill_catalog_snapshot, install_skill_pack, list_skill_packs,
    uninstall_skill_pack, InstallSkillPackRequest, InstructionScope, LocalExecutor,
    SkillCatalogEntry, SkillOrigin, SkillPackAction, SkillPackScope, SkillScope,
};
use serde_json::json;

use crate::fixture::{self, HomeGuard};
use crate::model::{error, evidence, require, DynError, Recorder};
use crate::provider_harness;

pub async fn run(source: &Path, output: &Path, recorder: &mut Recorder) -> Result<(), DynError> {
    let local_home = output.join("fake-empty-user");
    let repository = output.join("local-repository");
    let selected = repository.join("worktree");
    let local_source = output.join("fixtures/local-superpowers");
    std::fs::create_dir_all(&local_home)?;
    std::fs::create_dir_all(&selected)?;
    let _home = HomeGuard::enter(&local_home)?;

    recorder
        .step(
            "fixture_real_superpowers",
            "Copy the supplied Superpowers checkout into an isolated mutable fixture",
            || async {
                fixture::copy_tree(source, &local_source)?;
                let count = skill_file_count(&local_source)?;
                require(
                    count >= 10,
                    format!("expected at least 10 skills, found {count}"),
                )?;
                Ok((
                    (),
                    evidence([
                        ("skillFiles", json!(count)),
                        ("fixture", json!(local_source)),
                    ]),
                ))
            },
        )
        .await?;

    let baseline = recorder
        .step(
            "empty_user_baseline",
            "Start with no personal setup and no project skills",
            || async {
                require(
                    directory_empty(&local_home)?,
                    "fake user home was not empty",
                )?;
                require(
                    directory_empty(&selected)?,
                    "selected project folder was not empty",
                )?;
                fixture::init_repository(&repository)?;
                let snapshot = snapshot(&selected, "local:read-benchmark").await;
                let external = snapshot
                    .skills
                    .iter()
                    .filter(|skill| skill.scope != SkillScope::Bundled)
                    .count();
                require(
                    external == 0,
                    format!("baseline leaked {external} external skills"),
                )?;
                require(
                    snapshot.diagnostics.is_empty(),
                    "baseline catalog had diagnostics",
                )?;
                Ok((
                    snapshot.clone(),
                    evidence([
                        ("home", json!(local_home)),
                        ("catalogRevision", json!(snapshot.revision)),
                        ("nonBundledSkills", json!(external)),
                    ]),
                ))
            },
        )
        .await?;

    recorder
        .step(
            "legacy_symlink_discovery",
            "Reproduce Read's legacy symlink-style Superpowers discovery",
            || async {
                let mechanism = fixture::create_legacy_link(&local_home, &local_source)?;
                let linked = snapshot(&selected, "local:read-benchmark").await;
                let count = linked
                    .skills
                    .iter()
                    .filter(|skill| {
                        skill.name == "brainstorming"
                            && skill.scope == SkillScope::User
                            && skill.origin == SkillOrigin::Compatible
                    })
                    .count();
                require(
                    count == 1,
                    format!("expected one linked brainstorming skill, got {count}"),
                )?;
                require(
                    linked.diagnostics.is_empty(),
                    "linked catalog reported diagnostics",
                )?;
                fixture::remove_legacy_link(&local_home)?;
                let removed = snapshot(&selected, "local:read-benchmark").await;
                require(
                    !removed.skills.iter().any(|skill| {
                        skill.name == "brainstorming" && skill.scope == SkillScope::User
                    }),
                    "legacy skill remained after removing the link",
                )?;
                Ok((
                    (),
                    evidence([
                        ("mechanism", json!(mechanism)),
                        ("linkedCatalogRevision", json!(linked.revision)),
                        ("canonicalSkillCount", json!(count)),
                    ]),
                ))
            },
        )
        .await?;

    recorder
        .step(
            "instruction_provenance",
            "Load personal, project, and nested instructions in explicit precedence order",
            || async {
                fixture::seed_instructions(&repository, &selected, &local_home)?;
                let instructions = discover_instructions(&LocalExecutor, &selected)
                    .await?
                    .ok_or_else(|| error("instruction discovery returned no sources"))?;
                let scopes = instructions
                    .sources
                    .iter()
                    .map(|source| source.scope)
                    .collect::<Vec<_>>();
                require(
                    scopes
                        == [
                            InstructionScope::Personal,
                            InstructionScope::Project,
                            InstructionScope::Nested,
                        ],
                    format!("unexpected instruction precedence: {scopes:?}"),
                )?;
                Ok((
                    (),
                    evidence([
                        ("sourceCount", json!(instructions.sources.len())),
                        (
                            "precedence",
                            json!(instructions
                                .sources
                                .iter()
                                .map(|source| source.path.clone())
                                .collect::<Vec<_>>()),
                        ),
                    ]),
                ))
            },
        )
        .await?;

    let (installed, old_skill) = recorder
        .step(
            "managed_install_and_collision",
            "Install a validated user pack and preserve a project name collision",
            || async {
                let receipt = install_skill_pack(
                    &LocalExecutor,
                    &selected,
                    InstallSkillPackRequest {
                        pack_id: "superpowers".into(),
                        source_path: local_source.to_string_lossy().into_owned(),
                        scope: SkillPackScope::User,
                    },
                )
                .await?;
                require(
                    receipt.action == SkillPackAction::Installed,
                    "pack was not installed",
                )?;
                require(
                    receipt.skill_count >= 10,
                    "managed pack omitted Superpowers skills",
                )?;
                fixture::seed_collision(&selected)?;
                let catalog = snapshot(&selected, "local:read-benchmark").await;
                let managed = managed_brainstorming(&catalog.skills)?.clone();
                let collisions = catalog
                    .skills
                    .iter()
                    .filter(|skill| skill.name == "brainstorming")
                    .collect::<Vec<_>>();
                require(
                    collisions.len() == 2,
                    "same-name collision was not preserved",
                )?;
                require(
                    collisions.iter().all(|skill| skill.has_name_collision),
                    "collision entries were not marked",
                )?;
                require(
                    collisions
                        .iter()
                        .map(|skill| &skill.invocation_name)
                        .collect::<HashSet<_>>()
                        .len()
                        == 2,
                    "collision-safe invocation names were not unique",
                )?;
                let packs = list_skill_packs(&LocalExecutor, &selected).await?;
                require(
                    packs.len() == 1,
                    "managed pack registry did not list one pack",
                )?;
                Ok((
                    (receipt.clone(), managed.clone()),
                    evidence([
                        ("packRevision", json!(receipt.revision)),
                        ("skillCount", json!(receipt.skill_count)),
                        ("selectedId", json!(managed.id)),
                        ("selectedRevision", json!(managed.revision)),
                        ("exactInvocation", json!(managed.invocation_name)),
                        (
                            "collisionInvocations",
                            json!(collisions
                                .iter()
                                .map(|skill| skill.invocation_name.clone())
                                .collect::<Vec<_>>()),
                        ),
                    ]),
                ))
            },
        )
        .await?;
    require(
        installed.previous_revision.is_none(),
        "first install unexpectedly had a previous revision",
    )?;

    recorder
        .step(
            "typed_binding_round_trip",
            "Round-trip the exact id, revision, and display name through typed history",
            || async {
                let blocks = vec![
                    ContentBlock::text("Use the selected skill."),
                    ContentBlock::skill_reference(
                        old_skill.id.clone(),
                        old_skill.revision.clone(),
                        old_skill.invocation_name.clone(),
                    ),
                ];
                let encoded = serde_json::to_vec(&blocks)?;
                let decoded: Vec<ContentBlock> = serde_json::from_slice(&encoded)?;
                require(
                    decoded == blocks,
                    "typed skill binding changed during history replay",
                )?;
                Ok((
                    (),
                    evidence([
                        ("serializedBytes", json!(encoded.len())),
                        ("id", json!(old_skill.id)),
                        ("revision", json!(old_skill.revision)),
                    ]),
                ))
            },
        )
        .await?;

    let (mut provider, first_turn) = recorder
        .step(
            "provider_boundary_v1",
            "Send the exact selection through the product's real provider request boundary",
            || async {
                let (active, turn) = provider_harness::launch_and_prompt(
                    &selected,
                    &old_skill,
                    &[
                        "# Brainstorming Ideas Into Designs",
                        "READ_BENCH_PERSONAL_INSTRUCTION",
                        "READ_BENCH_PROJECT_INSTRUCTION",
                        "READ_BENCH_NESTED_INSTRUCTION",
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
        !first_turn.request_text.is_empty(),
        "provider request capture was empty",
    )?;
    require(
        first_turn.request.get("tools").is_some(),
        "provider request did not include the production tool schema",
    )?;

    let updated_skill = recorder
        .step(
            "live_update_and_stale_rejection",
            "Update the pack, refresh at the next run, and reject the stale binding before dispatch",
            || async {
                fixture::append_update_marker(&local_source, "READ_BENCH_LOCAL_V2")?;
                let receipt = install_skill_pack(
                    &LocalExecutor,
                    &selected,
                    InstallSkillPackRequest {
                        pack_id: "superpowers".into(),
                        source_path: local_source.to_string_lossy().into_owned(),
                        scope: SkillPackScope::User,
                    },
                )
                .await?;
                require(receipt.action == SkillPackAction::Updated, "pack update was not detected")?;
                let catalog = snapshot(&selected, "local:read-benchmark").await;
                let updated = managed_brainstorming(&catalog.skills)?.clone();
                require(updated.id == old_skill.id, "managed skill identity changed on update")?;
                require(
                    updated.revision != old_skill.revision,
                    "managed skill revision did not change",
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
                        ("previousRevision", json!(old_skill.revision)),
                        ("currentRevision", json!(updated.revision)),
                        ("stableId", json!(updated.id)),
                        ("rejection", json!(rejection)),
                        ("modelCallsForRejectedTurn", json!(0)),
                    ]),
                ))
            },
        )
        .await?;

    let (mut restarted, restarted_turn) = recorder
        .step(
            "restart_and_updated_body",
            "Restart the desktop app, rediscover the pack, and load the updated body",
            || async {
                let fresh = snapshot(&selected, "local:read-benchmark-restart").await;
                let selected_after_restart = managed_brainstorming(&fresh.skills)?;
                require(
                    selected_after_restart.id == updated_skill.id
                        && selected_after_restart.revision == updated_skill.revision,
                    "restart did not rediscover the active revision",
                )?;
                let (active, turn) = provider_harness::launch_and_prompt(
                    &selected,
                    selected_after_restart,
                    &["READ_BENCH_LOCAL_V2"],
                )
                .await?;
                Ok((
                    (active, turn),
                    evidence([
                        ("catalogRevision", json!(fresh.revision)),
                        ("selectedRevision", json!(selected_after_restart.revision)),
                        ("updatedMarkerLoaded", json!(true)),
                    ]),
                ))
            },
        )
        .await?;
    require(
        restarted_turn.final_text.contains("SIMULATED_SKILL_ACK"),
        "restarted provider did not project the scripted response",
    )?;

    recorder
        .step(
            "uninstall_and_removed_rejection",
            "Uninstall atomically, retain the project collision, and reject the removed binding",
            || async {
                let receipt = uninstall_skill_pack(
                    &LocalExecutor,
                    &selected,
                    "superpowers",
                    SkillPackScope::User,
                )
                .await?;
                require(
                    receipt.action == SkillPackAction::Uninstalled,
                    "pack was not uninstalled",
                )?;
                require(
                    receipt.warnings.is_empty(),
                    "uninstall produced cleanup warnings",
                )?;
                let packs = list_skill_packs(&LocalExecutor, &selected).await?;
                require(packs.is_empty(), "pack remained active in the registry")?;
                let after = snapshot(&selected, "local:read-benchmark").await;
                require(
                    !after
                        .skills
                        .iter()
                        .any(|skill| skill.id == updated_skill.id),
                    "managed skill remained in the catalog",
                )?;
                require(
                    after.skills.iter().any(|skill| {
                        skill.name == "brainstorming" && skill.scope == SkillScope::Project
                    }),
                    "uninstall incorrectly removed the independent project collision",
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
                        ("previousRevision", json!(receipt.previous_revision)),
                        ("remainingManagedPacks", json!(packs.len())),
                        ("projectCollisionPreserved", json!(true)),
                        ("rejection", json!(rejection)),
                    ]),
                ))
            },
        )
        .await?;

    require(
        baseline.skills.iter().all(|skill| skill.id != old_skill.id),
        "managed skill identity leaked into the empty baseline",
    )?;
    Ok(())
}

async fn snapshot(project: &Path, environment: &str) -> provider_local::SkillCatalogSnapshot {
    discover_skill_catalog_snapshot(&LocalExecutor, project, environment, &HashSet::new(), &[])
        .await
}

fn managed_brainstorming(skills: &[SkillCatalogEntry]) -> Result<&SkillCatalogEntry, DynError> {
    skills
        .iter()
        .find(|skill| {
            skill.name == "brainstorming"
                && skill.scope == SkillScope::User
                && skill.origin == SkillOrigin::Bundled
        })
        .ok_or_else(|| error("managed user brainstorming skill was not discoverable"))
}

fn skill_file_count(source: &Path) -> Result<usize, DynError> {
    Ok(walkdir::WalkDir::new(source.join("skills"))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == "SKILL.md")
        .count())
}

fn directory_empty(path: &Path) -> Result<bool, DynError> {
    Ok(std::fs::read_dir(path)?.next().is_none())
}
