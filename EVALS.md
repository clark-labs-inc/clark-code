# Clark Code evaluations, benchmarks, and simulations

Last source-and-artifact audit: 2026-08-16.

This catalog covers Clark Code's deterministic open-source checks. Hosted or
paid provider runs are opt-in and require explicit authorization.

## Evidence rules

1. Unit and integration tests prove only their named contracts.
2. Scripted simulations prove harness mechanics, not model quality or live use.
3. Live provider tests require explicit authorization for the exact model and
   route; no live provider test is part of the default gate.
4. Host checks do not prove a packaged application or guest-VM journey.
5. Ignored `target/` and disposable `/tmp` artifacts are not durable evidence.
6. Keep typed receipts and first failures; never infer success from UI state or
   a process exit alone.

## Natural assistant completion — 2026-10-20

The October 20 desktop incident returned successful provider streams with no
structured tool calls at final delivery. An offline ordinary-response regression
reproduced the old transport-local repair loop and timed out after 60 seconds
(nextest `73ce4dde-9701-44b3-b5c1-e1d27bfedfb2`). The same test now completes
on its first response. No private incident text is retained in the fixtures.

The desktop now finishes through ordinary assistant responses in the canonical
loop. The required terminal tool, named-tool repair, and duplicate answer-stream
projection are removed. The nonterminal `deliver_files` producer validates all
requested paths before emitting typed artifacts. Effect verification remains a
canonical follow-up and tool gate: premature answers stay hidden, premature file
delivery returns an ordinary error, and rejected answer text remains in model
history for repair. Existing approved-plan and goal lifecycle guards remain.

The foundation gate passed **789 tests, eight skipped** across agent-core,
provider-acp, and provider-local (nextest
`9299111e-f4c0-4556-bc3c-96e72553afdb`). The seven completion/termination
regressions also passed against published `clark-agent =0.4.0`, with the local
path override excluded (`74569adf-68bb-4814-be04-bb4abdf1d4cd`). They cover
post-work prose, single answer projection, saved follow-up context, automatic
tool selection, validated file delivery, pending-effect rejection and repair,
and 160 productive work turns. Core WASM compilation and all-targets provider
compilation (including examples) pass against the published dependency.

The real stdio worker replay also used the retired terminal tool in its fake
provider. Both scenarios stalled on that stale fixture; after switching it to
ordinary assistant SSE responses, the working-day/cancellation/replay and
saturation/oversized-input journeys pass in 4.2 seconds (2/2, nextest
`0509869c-66f6-4fce-b28d-e3469f04295d`). The oversized fixture is split into
helper, journey, and saturation modules. No hosted provider was called.

Strict Clippy remains blocked by the existing Rust 1.99 dependency-upgrade
warnings: fixed-size `chunks_exact` in exec-sandbox-protocol, a pre-existing
needless borrow in the engine, and large provider error variants. No lint
suppression or unrelated migration fix is included. This is deterministic
source-path evidence; no hosted model call, installed-app replacement,
packaged acceptance, deployment, or release was performed.

## Rendering flicker follow-up — 2026-10-19

Storage update: October 19–22 raw frame/transcript traces and generated media
remain local and ignored. The versioned README conclusions and `validation.json`
receipts retain counts, hashes and limitations; raw traces are not available
from a fresh checkout and must be regenerated with the named harnesses.

No new flicker was reproduced on the existing rendering fixes with the current
uncommitted dependency candidate. All 22 prior rendering/harness source hashes
match. [Fresh frames, recordings, and validation](docs/element-presence/2026-10-19/README.md)
retain 2,037 samples across 20 dialog cases in Chromium/WebKit with both motion
preferences: zero entry-opacity reversals, competing CSS opacity transitions,
rapid-reopen replacements, settled displacement, or page errors; every exit has
intermediate frames. The four transcript configurations retain 12 commands
without overlap, replacement, duplicates, or status/finish height shift.

Frozen frontend installation, typecheck, 848 tests (3 skipped), and production
build pass. No app or harness source changed. Existing dirty work is preserved
apart from this additive catalog entry. Historical failing traces still identify
destroyed presence ownership and the WebKit WAAPI-to-CSS opacity handoff as the
original boundaries. Installed-native/OS-window creation, arbitrary popovers,
packaged acceptance, and the pending Rust upgrade Clippy migration are outside
this follow-up. No active app restart, paid tests, commit, or release occurred.

2026-10-20 replay: the same 25 rendering/dependency source hashes still match.
[Fresh retained observations](docs/element-presence/2026-10-20/README.md)
pass all 20 dialog cases (2,041 frame samples) and four transcript configurations
with zero entry reversals, competing opacity transitions, node replacements,
overlap, or status/finish height shift. Frozen install, typecheck, 856 frontend
tests (3 skipped), and production build pass. Current mobile-project work and
all other existing changes are preserved; only evidence and this catalog entry
were added. No new app/harness patch or native-app interaction occurred; the
same native, packaged, untested-popover, and Rust-migration limits apply.

2026-10-21 replay at `921f564a`: all 32 sources in the previous rendering
receipt remain identical. [Fresh frame evidence](docs/element-presence/2026-10-21/README.md)
passes 20 dialog cases (2,044 samples) and four transcript configurations with
zero opacity reversals, competing CSS fades, node replacements, overlap, or
status/finish height shift. Frozen install, typecheck, 856 frontend tests
(3 skipped), and production build pass. No app/harness patch was needed; all
29 existing changed/untracked files are preserved apart from this additive
catalog update. The prior native/packaged/paint-coverage limits still apply.

2026-10-22 replay at `fde7f2db`: the same 32 monitored sources and all existing
frontend/harness dirty files match the previous receipt; no intervening commits
change those trees. [Fresh observations](docs/element-presence/2026-10-22/README.md)
pass 20 dialog cases (2,041 samples) and four transcript configurations, with
zero opacity reversals, competing CSS fades, replacements, overlap, or
status/finish height shift. Full frontend gates were not repeated on unchanged
sources; the prior 856-test/build receipt remains separately identified.
No app/harness patch was made. All 33 prior changed/untracked files are
preserved except for this additive catalog entry; native/packaged/paint limits
remain unchanged.

## Dependency upgrade candidate — 2026-10-18

The [upgrade review](docs/dependency-upgrades/2026-10-18.md) and
[source-bound receipt](docs/dependency-upgrades/2026-10-18/validation.json)
record Rust 1.99.1, React 19.3.1, Tauri 2.12.4 with matching frontend/native
plugins, Vite 8.3.4, and 32 compatible direct Rust dependency release updates.
Frozen frontend installation, typecheck, 848 frontend tests (3 skipped), and
production build pass; a downstream extension composition passes 873 tests
(3 skipped) and its build. Isolated CI-pinned pnpm 10.34.5 frozen installation
passes with lifecycle scripts disabled.

All 20 dialog presence cases and all four transcript geometry configurations
pass in Chromium/WebKit with normal/reduced motion: no entry-opacity reversal,
command overlap/replacement, or status/finish height shift. Rust format, 802
Nextest tests (8 skipped), core WASM check, macOS native all-targets compilation,
and 13 public-boundary checks pass. Existing local agent path overrides apply.

**This candidate is not fully qualified:** strict Rust 1.99.1 Clippy fails on
new fixed-chunk iteration warnings in the protocol and capture crates. A diagnostic probe
also identifies provider needless-borrow and large-error warnings. Minimal
source migration permission remains pending under the shared-file rules; no
lint suppression is committed. This is not exact-source hosted CI, Linux or
Windows native qualification, a packaged app, or a release. Downstream source
pins and compiler/cache identities still require coordinated consumption.

## Dialog appearance and presence — 2026-10-18

`node harness/element-presence.mjs` samples first appearance, close, and rapid
reopening of the command palette, session chooser, Settings, MCP, and SSH
dialogs. The initial Chromium probe found all five removed in 3–14 ms with
zero exit-fade frames; rapid reopening replaced their DOM nodes. A separate
WebKit reduced-motion probe then caught a full-opacity → zero-opacity reversal
on every dialog's first appearance. Its frame records show a CSS opacity
transition starting when Motion commits the completed WAAPI fade.

The lazy modal shells now retain a keyed outer presence owner and propagate
inner exits; the palette keeps its presence owner mounted while closed.
Reduced-motion CSS gives nonzero durations only to explicit CSS transition
utilities, preserving narrower property sets instead of adding a second opacity
transition to Motion surfaces. All 20 engine/motion/dialog combinations pass
with zero entry reversals, no duplicate/replaced nodes on rapid reopening,
continuous exits, and zero settled displacement or page errors.

Retained [frames, recordings, source digests, and scope](docs/element-presence/2026-10-18/README.md)
use synthetic local state in Chromium and WebKit. Frontend frozen install,
typecheck, 848 tests (3 skipped), and production build pass. Foundation format,
strict Clippy, 802 Nextest tests (8 skipped), and core WASM check pass.
Transcript geometry/identity, workspace navigation, image/attachment motion,
and permission/toast exits also pass both engines and both motion preferences.
Installed-native, packaged-release, OS-window creation, and arbitrary popover
paint behavior are outside this browser qualification.

Release cleanup removes the obsolete reply-reserve helper, estimator, styles,
and tests. Duplicate conversation-motion GIFs and their encoder are removed;
MP4 evidence remains. Routine motion captures now default to ignored `target/`
output, with an explicit `OUTPUT` override for durable exports.

## Transcript geometry and command identity — 2026-10-18

`node harness/rendering-stability.mjs` now covers narration/tool handoffs,
short-answer and run completion, and rapid appends while output opens/closes.
Chromium and WebKit passed with normal and reduced motion: zero status/finish
height shift, zero command overlap, stable identities/titles for 12 visible
commands, and 58–71 sampled frames per configuration. Existing disclosure,
scrollback, and retained-Markdown checks passed. Four-line transcript skeleton
geometry is removed; a fixed activity line and opacity-only assistant entrances
replace it. This supersedes the older reply-placeholder motion contract below.

Retained [receipt, source digests, recording, and screenshot](docs/transcript-stability/2026-10-18/README.md)
use synthetic local state. Frontend tests pass 848 (3 skipped), typecheck,
frozen install, and production build pass. Rust format, strict foundation
Clippy, 802 Nextest tests (8 skipped), and the core WASM check pass. These checks
do not qualify the installed native application or the user's live conversation.

## Conversation motion — 2026-10-17

`node harness/conversation-motion.mjs` exercises the actual UI with synthetic
local conversations in Chromium and WebKit, each with normal and reduced
motion. All four configurations passed: image hover policy, full-viewport
viewer geometry, keyboard focus containment/return, viewer and final-attachment
exit fades, Markdown/generated-image viewing, zero transcript displacement,
and zero page errors. Retained [before/after videos, receipts, and review
notes](docs/conversation-motion/README.md) document the visual conclusions.
The baseline exposed a viewer trapped by a transformed message row; the body
portal fixes that boundary.

Rendering-stability and workspace-navigation acceptance passed all four
browser/motion configurations. The existing artifact-delivery lane passed its
six image/PDF/download checks. The focused motion/message suite passed 43 tests;
frozen install, typecheck, and production build passed. The full frontend suite
initially passed 844 tests (3 skipped); a subsequent run during concurrent auth
edits passed 843 and failed the unrelated cloud-lifecycle reconnect-state
assertion. Foundation strict Clippy, 802 nextest tests (8 skipped; run
`8188b912-bc33-4eee-ae29-8a8c539f27e7`), and core WASM check passed.
Workspace formatting remains blocked by missing `src-tauri/src/product/tests.rs`.

The canonical unsigned launcher built and started the exact local development
executable. Native motion interaction and packaged/Windows/Linux behavior were
not qualified. These are deterministic visual and interaction receipts, not
frame-rate benchmarks or hosted-model evidence.

## Mobile permission validation — 2026-10-11

The focused permission validation and Desktop-store parity gate passed 15/15
tests. The frontend gate passed 828 tests with three skipped, typecheck,
production build, and frozen-lockfile installation. Rust format, strict Clippy,
the three-crate nextest gate (802 passed, eight skipped), and agent-core WASM
check passed. The unsigned local desktop launcher built and started. These
receipts prove the source permission handoff; authenticated cloud delivery and
released phone/desktop behavior require downstream verification.

## Current map

Streaming-to-phone source verification (2026-10-11): six initial regressions
failed across first-text cadence, held-delta timing, already sealed active
history, long-chat publication, and the native active-head boundary. The final
frontend gate passed 841 tests with three skipped, typecheck, production build,
and frozen-lockfile installation. Native-host nextest passed 153/153, including
the repaired active-head publication contract. Core strict Clippy, format,
802 nextest tests (eight skipped), and WASM check passed. The unsigned desktop
launcher built and started. Downstream iOS and Android fixtures visibly render
the first public text at chunk 1/5 before completion; the mobile gate passed
1,217 tests, typecheck, and zero lint errors. These are scheduling, native
boundary, and synthetic renderer receipts, not measured production WAN latency.

| Surface | Entrypoint | Evidence class | Claim boundary |
| --- | --- | --- | --- |
| Native integrations / read-only iMessage tool | `cargo nextest run -p desktop-integrations -p exec-sandbox`; provider-local `native_data_stays_private_even_with_full_access_file_tools_and_symlinks`; host `renderer_cannot_add_write_or_widen_read_arguments`; frontend `integrations.spec.ts` | deterministic registry, ToolPack schema, synthetic read-only SQLite, real macOS sandbox process, and IPC contracts | Compiled in debug and release profiles. Proves exact-selection and task-scope contracts plus sandboxed file boundaries. It does not prove full host isolation, actual privacy grant/revocation, a live Messages read, or a packaged release. See the retained conclusion below and `crates/desktop-integrations/README.md`. |
| Open-source boundary | `node --test harness/product-boundary.spec.mjs` | deterministic repository-wide text, dependency, and metadata contract | Rejects hardcoded hosted-service policy, commercial access rules, release credentials, and deployment-specific transports |
| Core domain and providers | `cargo nextest run -p agent-core -p provider-acp -p provider-local` with cargo-nextest `0.9.143` | deterministic Rust contracts | Proves provider-neutral projection, ACP translation, and local loop/tool behavior |
| Phone text publication | frontend `snapshotPublicationCadence.spec.ts`, `snapshotPersistence.spec.ts`, `cloudLiveCheckpoint.spec.ts`, `cloudHistory.streaming.spec.ts`, and `historyFingerprint.spec.ts`; native `cloud_publication::pages::tests` | deterministic first-public-text scheduling, trailing flush, monotonic revision, immutable-prefix, and bounded native head contracts | Queues the first public text immediately, coalesces growth with a 250ms trailing flush, and flushes terminal state immediately. New active deltas replace a held busy checkpoint; a terminal generation cannot be overwritten. Already sealed history is excluded from active heads without sealing new mutable rows. Genuine oversized unsealed payloads retain the bounded idle backstop. Authenticated phone/cloud delivery and actual network latency require downstream acceptance. |
| Mobile permission execution | frontend `mobileRemotePermission.spec.ts` and `MobileRemoteAgent.parity.spec.ts` | deterministic permission validation and real desktop-store bridge handoff | Accepts the pending tool's exact permission for a `running` or `awaiting_input` run, prefers one-time decisions, and rejects stale actions, terminal runs, different conversations/runs, ambiguous legacy ownership, and missing options. Native phone, hosted command delivery, and released app behavior require downstream evidence. |
| Full deterministic Rust workspace CI | `cargo nextest run --workspace -E 'not test(attachment_benchmark_local)' --no-fail-fast` with cargo-nextest `0.9.143` | deterministic Rust unit, integration, native-host, sandbox, worker, orchestration, and computer-use contracts | Required on every push and pull request; the host-load-sensitive attachment benchmark remains in the benchmark lane rather than making integration CI timing-dependent |
| Root-loop termination | provider-local `productive_turn_runs_past_128_model_tool_iterations_without_global_cap`, `provider_owns_stream_silence_and_the_user_can_still_cancel`, and agent-orchestration `root_recovery_has_no_implicit_attempt_ceiling` | deterministic provider-loop contracts | Proves productive root work has no implicit step, cumulative-token, stream-idle, wall-clock, empty-outcome, or recovery-attempt lifetime stop; user cancellation remains immediate |
| Provider progress streaming and completion | provider-local `natural_completion`, `loop_termination`, and `agent_adapter::projection_tests` | deterministic provider-stream, completion, and desktop-projection contracts | Ordinary assistant responses finish through the canonical loop and remain in follow-up history. Tools always use automatic selection. Generated files are validated by the nonterminal `deliver_files` producer; pending effects block premature answer/file publication until canonical verification. Ordinary tool deltas still stream as activity. These fixtures do not prove a hosted provider or packaged application. |
| Provider reasoning continuity | provider-local reasoning receipt, retry, accumulator, and full-history translation contracts; downstream ignored `live_reasoning_continuity` | deterministic hash-only capture/replay contracts plus an explicitly authorized live Clark Code source-path sample | The focused deterministic gate passed 19/19 (nextest `db3fb4d3-5a49-4666-b347-995846f82e4e`). The 2026-08-26 Ox Alpha sample passed 2/2 turns after four provider responses, retained exact structured reasoning across the full canonical history, and matched capture to replay at SHA-256 `78cc3a2d6c33dcb467b61dec4a5ea0c8d3c275205b42518de8b076119a3b344f`; safe traces contain hashes and counts, never raw private reasoning. Receipt: `/Users/stan/Documents/evals/clark-client-shape-reasoning-20260826_045507/clark-code/clark-code-summary.json`. This proves the current source path, not a packaged or released Clark Code build. |
| Delegated-run lifecycle | agent-orchestration `unbounded_provider_harness_remains_cancellable_while_the_stream_is_idle` and `default_budget_accounts_without_imposing_a_lifetime_ceiling`, plus provider-local `production_harness_path_is_parallel_isolated_reviewed_and_replayed` | deterministic child-provider and isolated-worktree contracts | Productive local, ACP, writer, reviewer, and integrator attempts have no foundation-owned response deadline or default shared-token lifetime ceiling; explicit cancellation wakes an idle stream and reaches the provider. Optional caller-owned deadlines and budgets remain available for bounded tests, evals, and brokered services. This does not prove live delegated-model quality. |
| Tool-result retention and overflow recovery | provider-local `giant_tool_output_is_preserved_before_the_next_model_call`, `bounds_broad_results_and_tells_the_model_how_to_refine`, `broad_grep_reports_complete_count_without_flooding_the_next_prompt`, `hierarchical_compaction_covers_every_oversized_source_byte`, and `oversized_context_recovers_through_hierarchical_compaction`; sibling `../clark-agent` `clips_old_results_but_preserves_the_entire_fresh_batch`, `useful_excerpt_keeps_head_and_tail_within_cap`, and `bounded_excerpt_never_exceeds_a_tiny_cap` | deterministic provider-loop, tool-output, and history-transform contracts | Proves every accepted tool result in the newest batch reaches the next model request verbatim; broad grep itself returns a complete match count with at most 64 KiB of result rows and explicit refinement guidance; compaction partitions an oversized transcript into provider-safe UTF-8 segments covering every source byte, sends summarization policy in the system role with each segment isolated as quoted evidence, and recursively reduces ordered handoffs instead of resending the rejected prompt or silently dropping a prefix. The public provider-loop reproduction completes an overflowed turn through two map calls and one reduction. Older oversized results retain bounded head/tail evidence with the middle omitted. It does not prove live summarization quality or a packaged recovery journey. |
| Awaited background recovery | `env -u RUSTC_WRAPPER cargo nextest run -p provider-local eval_awaited_build_survives_provider_outage_without_keep_going` | deterministic real provider-loop simulation with local shell execution and a scripted SSE outage | Proves one user prompt can start a background build, time out while awaiting it, exhaust request-local transport retries, consume exactly one terminal build receipt during whole-run recovery, and finish with a typed `Done` outcome. Retained 2026-08-14 receipt: nextest `ec4649e9-ad89-4403-b8eb-f5a8b5eabf85`, 1/1 passed in 3.692s. It does not prove a live provider, packaged app, or deployed product. |
| Public live benchmark interface | sibling `../clark-public-evals` package, with Clark Managed and the downstream Clark Code public CLI as separate targets | externally owned, opt-in Free-tier live evaluation | Routes Finance Agent v2, Terminal-Bench, DeepSWE, BrowseComp, WebTailBench, Online-Mind2Web, and OSWorld-Verified without importing branded policy into this foundation. Scores and release claims belong to the external package and downstream Clark release; this repository owns only the provider contracts they exercise. |
| Full-stack worker simulation | `cargo nextest run -p code-worker --test full_stack_simulation` | deterministic end-to-end simulation: the real `agent-code-worker` binary over its real stdio JSONL protocol, the real provider-local loop against a scripted loopback OpenAI-SSE model, real tools in a real temporary Git repository | Proves the seam no per-crate contract covers: boot receipt with execution residency, bounded `fs/walk` truncation, path confinement without content leakage, a streamed multi-turn run whose permission gate is answered over the wire mid-stream, files really written and a checkpoint ref really created, tool results round-tripped to the model, monotone progress sequences with exactly one run start/finish, terminal frames without embedded snapshots, mid-run cancellation as a typed terminal with the session surviving, SIGKILL + restart replaying a byte-identical durable receipt, same-id/different-params as `request_id_conflict`, stale sessions as typed `invalid_input`, health pings answered and extra work refused `busy` on a saturated single-permit worker, an oversized request line surviving as `request_too_large`, and clean shutdown. It does not prove SSH transport, live-model quality, or the packaged WebView. First run discovered and fixed the host collapsing every non-cancel plugin error into `plugin_failed`; caller errors are now typed (`invalid_input`, `unknown_plugin`, `unsupported_operation`) on the wire. |
| Native host | `cargo nextest run -p desktop-foundation --lib` | deterministic native command contracts | A packaged application still requires platform verification |
| Cloud auth refresh replay | desktop-foundation `expired_token_refreshes_and_replays_the_durable_outbox_end_to_end`; frontend `productBridge.spec.ts`, `sessionStore.cloudLifecycle.spec.ts`, `authPersistence.spec.ts`, `useProductAccess.spec.ts`, and `SpecialistAccessGate.spec.ts` | deterministic native-host simulation with real SQLite and local HTTP transport plus renderer request/recovery and access-state contracts | Proves an expired bearer produces one auth-refresh event, a delayed same-account credential generation wakes native delivery, and the byte-equivalent logical batch retries durably; renderer product calls join one refresh flight, wait for native-event recovery, replay the exact rejected operation at most once, publish only the still-active account, preserve auth-cache ownership, fence stale access results across account switches, make refresh failure manually reconnectable, and replace an indeterminate Security access check with an explicit retry state. It does not prove the packaged WebView, deployed gateway, or live identity provider. |
| Update/relaunch conversation durability | frontend `cloudHistory.spec.ts`, `cloudArtifacts.spec.ts`, and `sessionStore.updater.spec.ts`; desktop-foundation outbox `newer_pending_snapshot_survives_a_superseded_publication_ack`, `pending_snapshot_is_not_discarded_when_an_uncertain_put_advanced_cloud_revision`, and `staged_artifact_bytes_and_upload_receipt_survive_database_reopen` | deterministic renderer/native integration and real FULL-synchronous SQLite reopen contracts | Proves the updater gates on native durability rather than cloud latency; the newest coalesced tail can commit while an older cloud PUT remains unresolved; a late acknowledgement is mutation-fenced in the same transaction and cannot overwrite or clear the newer durable snapshot; pending snapshots retain their mutation identity through reopen and an uncertain response that advanced cloud revision; generated Markdown bytes and digest-fenced upload receipts survive process death; an idle snapshot revalidates an in-place rewrite, and neither an older upload nor a replaced SHA can acknowledge the newer bytes. Cloud replay, packaged relaunch, deployed gateway, and paid-provider behavior remain separate live gates. Retained packaged/live boundary receipt: `docs/testing/2026-08-20-update-relaunch-durability.md`; its package hash predates the atomic-ack cleanup, whose exact source currently has deterministic coverage only. |
| WASM core | `cargo check -p agent-core --target wasm32-unknown-unknown` | compile contract | Proves the domain crate remains WASM-clean |
| Frontend | `pnpm --dir app typecheck`, `pnpm --dir app test`, `pnpm --dir app build` | deterministic TypeScript, component, and bundle contracts | Proves the checked Clark Code frontend bundle |
| Queued follow-up steering | frontend `sessionStore.steer.spec.ts`; native host `adjacent_user_prompts_remain_distinct_after_replay` | deterministic renderer/session and durable projection contracts | Proves explicit steering cancels the current run while retaining the message in the ordinary follow-up queue, so the existing idle drain sends it once instead of hiding it in a provider-owned mid-run queue; every accepted user turn carries an explicit message boundary, so adjacent prompts remain separate through live projection and durable replay. It does not prove packaged cancellation latency or live-model behavior. |
| Hyper-realistic recovery matrix | `pnpm --dir app test -- src/core-bridge/resilienceBenchmark.spec.ts` plus `pnpm --dir harness test:resilience` | deterministic nine-fault power set (512 combinations) plus representative Chromium product journeys | Proves typed rate-limit, timeout, upstream, duplicate-tool-id, event-stream, tool-host, provider-process, cloud-sync, and user-cancel combinations settle truthfully; recovered incidents stay out of product UI, terminal interruption offers resume, cancellation requires an explicit stop, and raw diagnostics do not leak. It does not prove a live provider or packaged WebView. |
| Specialist runtime matrix | `pnpm --dir harness test:specialists`; `SECURITY_E2E_WORKFLOW=deep pnpm --dir harness test:specialists`; frontend `specialistNavigation.spec.ts`, `conversationProjectBinding.spec.ts`, `sessionStore.unavailableConversation.spec.ts`, `UnavailableConversation.spec.tsx` | deterministic Chromium integration against a product composition and mock provider, with typed receipts and screenshots, plus navigation and recovery state checks | Checks the Security start heading, dedicated new-session action, split-pane heading/action clearance, folder chooser, three starters, absence of organization selector and Insights canvas, general assistant and explicit deep-scan skill routing, paid/free access, failed-start draft recovery, terminal settlement, detach/reattach, and mobile chat without overflow. Cross-device checks keep all cloud-listed Security rows reachable, retain a read-only saved transcript when the original machine's project path cannot attach, block sending from that preview, and bind an explicitly selected local folder without changing shared cloud project metadata. It does not prove hosted-model quality, live billing, the other machine's cloud index, native folder-picker interaction, or packaged WebView behavior. |
| Model picker UI | `node harness/model-picker-smoke.mjs` | deterministic browser-bound UI-only interaction with screenshot and typed receipt | Proves the composer model menu is portaled above the workspace, stays inside a compact viewport, and accepts pointer selection in both directions; packaged native WebKit behavior remains a separate platform receipt |
| SSH execution-target picker | `node harness/ssh-settings-smoke.mjs` | deterministic browser-bound UI-only interaction with SSH discovery/probe fixtures, screenshots, and a typed receipt | Proves a host can be saved before choosing its default folder, an add-host action from the composer selects that exact host as the remote execution target, and incomplete targets remain actionable without a premature Git connection; live SSH and packaged native behavior remain separate receipts |
| Pragmatic drag and drop UI | `node harness/pragmatic-dnd-smoke.mjs` | deterministic browser-bound UI-only interaction with screenshot and typed receipt | Proves pinned-project pointer reordering, the equivalent exact-position menu with focus restoration, desktop-file drop attachment, and the equivalent file picker; packaged native WebKit/OS drag behavior remains a separate platform receipt |
| Artifact delivery UI | `node harness/artifact-delivery-smoke.mjs`; frontend `newSession.spec.ts`, `ArtifactJsonPreview.spec.tsx`, `ArtifactWorkspaceEmpty.spec.tsx`, `AuthenticatedWorkspace.artifacts.spec.ts`, `artifactPresentation.spec.ts`, `Sidebar.actions.spec.ts`; native host `json_preview_is_limited_to_the_session_root_and_size` | deterministic browser-bound mock-provider journey with screenshot and download receipts, plus renderer and native-host contracts | Proves inline image decoding, real PDF page rendering, visible artifact actions, image/PDF save-copy delivery, and artifact-workspace rendering. Scoped checks cover fresh specialist navigation clearing the previous session's artifacts, session-only artifact entry points and empty copy, selecting the newest usable artifact ahead of a metadata-only record, retaining a selected paged-history file in the workspace when the live snapshot lacks its URI, escaped JSON text, and native JSON preview confined to the active session's roots with a size limit. The added navigation checks do not prove packaged WebView interaction; packaged native save dialogs remain a separate platform receipt. |
| Attachment composer UI | `node harness/attachment-smoke.mjs` | deterministic WebKit mock-provider journey with delayed attachment admission and a typed receipt | Proves image and large-paste staging, immediate atomic composer clearing, non-repeatable sending feedback, expanded-paste delivery, and settled admission; packaged native attachment delivery and upload timing remain separate platform receipts |
| Cloud composer drafts | `pnpm --dir app test -- cloudComposerDraft.network.spec.ts cloudComposerDraft.spec.ts layoutPolicy.spec.ts composerDraft.spec.ts sessionStore.composerDraft.spec.ts`; downstream `cargo nextest run -p conversation-cloud`; `CLARK_REQUIRE_DESKTOP_DRAFT_DB=1 cargo nextest run -p clark-service-db --test desktop_draft_cas_e2e`; and `CLARK_REQUIRE_DESKTOP_DRAFT_DB=1 cargo nextest run -p clark-services --test desktop_draft_http_e2e` | deterministic frontend state-machine and native HTTP-codec checks plus real Axum/auth/Postgres CAS | Proves scoped keys, authoritative 204-to-revision-zero handling, conditional accepted-text clearing, bounded typed conflict handling, specialist-key URL encoding, create/update/payload-stable mutation replay/idempotent-clear/stale/concurrent CAS behavior, and authenticated HTTP response shapes; it does not prove the repaired service is deployed |
| Size-independent transcript history | `cargo nextest run -p agent-core -p desktop-foundation`; `pnpm --dir app test -- src/lib/historyFingerprint.spec.ts src/lib/cloudHistory.spec.ts`; downstream Postgres/page/client checks | deterministic native immutable-page, bounded renderer fingerprint, outbox, and IPC contracts | Rust seals only model-compacted, idle history into bounded pages while retaining 160 live items; the renderer hashes only that head for no-op detection. The former TypeScript 100 MiB/1,024 MiB paging benchmark measured a path removed when publication moved native, so its 2026-08-17 timings are historical and do not qualify native performance. A native large-history performance receipt, deployed migration, and real WAN throughput remain unverified. Legacy inline and segmented reads remain supported. |
| Paged-history provider replay | `cargo nextest run -p agent-core` (`paged_resume_transcript_matches_shared_frontend_fixture`); `pnpm --dir app test` (`history.spec.ts` and native bridge delegation) | shared deterministic Rust/TypeScript snapshot fixture with an absolute checkpoint index and retained timeline offset | Proves replay includes the checkpoint and only the retained tail after it, while edit prefixes preserve absolute coordinates; the desktop bridge delegates replay to agent-core. Packaged WebView reopen and a live provider continuation remain separate checks. |
| Native history publication | desktop-foundation `cloud_publication::tests` and `publication_reads_only_the_exact_durable_mutation`; frontend `cloudHistory.spec.ts` | deterministic native page preparation and retry classification, renderer/native IPC scheduling, and durable outbox contracts | The native publication command reads the exact staged mutation from SQLite, derives transcript pages from `agent-core`, publishes them before the bounded head, and returns typed retry/conflict/deleted/rejected/superseded outcomes. The renderer commits the full pending projection once; native retains the cloud's acknowledged page cursor after recovery and does not accept a renderer-selected cursor or a second snapshot payload at publication. Tests do not prove a packaged WebView, live cloud transport, or restart convergence across a real network outage. |
| Local sandbox | `cargo nextest run -p exec-sandbox` | deterministic policy and platform-adapter contracts | Packaged and signed binaries require a separate platform receipt |
| Durable worker | `cargo nextest run -p code-host -p code-worker -p code-remote -p provider-remote-worker` with cargo-nextest `0.9.143`, plus the composed product frontend contract | deterministic protocol, confinement, and renderer/native ownership contracts | Remote sessions carry only an opaque worker binding plus bounded typed recipes. The generic worker rejects unregistered product extension ids; downstream compositions may register compile-time session extensions in their own worker binary, while credentials remain native-owned and outside every recipe. Ignored live SSH lanes require explicit authorization |
| Orchestration | provider-local orchestration benchmark tests | deterministic fixture contracts | Does not claim live-model quality |
| Memory and goals | provider-local memory and goal eval tests | deterministic fixture contracts | Live-model quality is separate and opt-in |
Changing a foundation eval contract or authoritative result requires updating
this file.

## Local tree exploration — 2026-09-25

Security's broad-investigation skill now uses optional `research_tree` /
`research_tree_status` tools. The standalone Apache-2.0 `research-tree` crate
contains pure typed graph transitions, semantic-key deduplication with shared
crosslinks, prerequisites, deterministic clark-autoresearch ranking, bounded
branch context, explicit terminal outcomes, evidence-required reassessment /
reopening, and journal replay. The provider owns local storage and permissions;
there is no new backend service, dashboard, model client or scanner.

Local artifacts use atomic replacement, process file locks and expected
revisions. Tests cover cancellation before persistence, competing/stale updates,
corrupt replay, source changes/missing sources, task-scope storage, symlink/FIFO
rejection, budget headroom for terminal outcomes, and read-only status.
Bindings hash only explicitly listed source files; an empty list is unbound.
All tree evidence and outcomes remain caller assertions, not security seals.

The scripted provider test `tests/research_tree.rs` loads the actual Security
assistant skill and performs tool discovery, begin, proposal, deterministic
selection, source read, refutation, two child proposals, higher-priority child
selection and a blocked outcome. A fresh provider/session restores revision 8,
seven events, two spent steps, the refuted parent, blocked child and pending
sibling without rewriting the journal. This proves local execution and restart,
not live-model research quality.

Validation:
- `cargo nextest run -p research-tree -p agent-core -p provider-acp -p provider-local`:
  **811 passed, 8 skipped**, run `e279bdcf-6419-47b2-8915-b22c62968828`.
- `cargo clippy -p research-tree -p agent-core -p provider-acp -p provider-local -p devbridge --all-targets -- -D warnings`,
  workspace format check, and agent-core WASM build passed.
- Frontend frozen install, typecheck, **809 passed / 5 skipped**, and production
  build passed. No renderer changes were necessary.
- `cargo package -p research-tree --allow-dirty --offline` packaged and independently
  verified the library against published dependencies; nothing was published.
- Real standard and working-tree diff inventory tests prove journal/lock/temp
  updates at root and nested task scope do not change scan identity; actual
  source edits still invalidate both.
- Final branch-context provider replay passed **1/1**, run
  `a0391823-f144-4899-91bf-a8f1cff58ab2`, including selected-child and refuted-parent
  context in the actual model continuation.
- The unsigned native dev build passed in 22.29 seconds. The default launcher
  encountered occupied port 1420, so the same unsigned Tauri path was built on
  isolated port 1437 with `--no-watch`. The new process exited while an existing
  older app instance remained; this is build evidence, not a fresh WebView smoke.

The existing coverage, deep-pass, PoC and finalization suites remain intact.
This run made no paid model calls, live target scans, commits or deployments.
No improved vulnerability-detection rate or cost claim is established.

## Native integrations / iMessage — 2026-09-01

The foundation has a compiled native integration registry in Settings →
Integrations and one eager local-provider tool named
`read_imessage_selection`. The tool has no arguments and can return only the
1–20 exact message IDs a user enabled for the bound task after native approval
and conversation selection. Every call rechecks account, task, account
generation, live session instance, 15-minute expiry, sleep/lock epoch, current
OS access, and exact text equality. Tool output labels Messages text as
untrusted quoted data.

No draft, send, Apple Events, Automation permission, send ledger, background
polling, inbound task trigger, or model-selectable conversation/query surface
exists. The adapter is compiled in release builds. No Messages content was
read, no privacy permission was granted, and no hosted model call was made
during implementation checks.

Retained deterministic conclusions:

- Full workspace nextest `10471735-ada7-4131-a8b3-85a05c5812d4`:
  **1,478 passed, 11 skipped**. The focused integration/sandbox run
  `5a36b2a1-0071-4977-a9c6-89a9cc9d863e` passed **27/27**, including the
  fixed argument-free ToolPack schema, no send tool, explicit enablement,
  task/account/generation/session isolation, changed-text and OS-revocation
  failures, read-only synthetic SQLite, and real macOS sandbox read/write
  denial through direct and symlink paths.
- Native IPC `acd332e0-8db2-4565-a5e2-d58889701006` passed **1/1**, rejecting
  send, draft, Automation, owner override, and widened read arguments.
- Frontend passed **815 tests with 5 skipped** across 173 passing and 2 skipped
  files; typecheck and production build passed. Browser previews still refuse
  native integration access instead of simulating Messages.
- Scoped Clippy passed with warnings denied for agent-core, provider-acp,
  provider-local, devbridge, desktop-integrations, and exec-sandbox. The full
  native-host Clippy command remains blocked by 12 warnings in unrelated files;
  they were not modified. Repository formatting, core WASM, product boundary
  **14/14**, native debug compilation, and optimized release compilation all
  passed. Release compilation retained the existing unused
  `TrajectoryOutbox::acknowledge` warning outside this change.
- Local validation uses this checkout's existing `.cargo/config.toml` path
  override for the sibling `clark-agent`. This is not exact pinned-dependency
  CI or a packaged/released product receipt.
- The unsigned `./script/build_and_run.sh` launcher built and started the
  current `target/debug/clark-code`. Startup did not connect iMessage, request
  Full Disk Access, read Messages, use a signing identity, or touch Keychain.

**Isolation limitation:** Full Disk Access belongs to the whole Clark Code app,
not a task or conversation. Sandboxed commands and file tools deny Messages
paths after symlink resolution, but Full Access/elevated execution, MCP,
external agents, terminals, computer use, another same-user process, and a
compromised renderer are not comprehensively contained by this grant. The
read tool itself is task-scoped; the macOS app permission is not. A separately
identified authenticated broker is still required for strong app-level
isolation.

Actual-app permission owner attribution, grant/deny/revoke, a live selected
conversation read through the tool, real sleep/lock/restart, and alternate-tool
isolation remain **not run**. User-assisted acceptance steps and schema limits
are in [`crates/desktop-integrations/README.md`](crates/desktop-integrations/README.md).
Do not promote deterministic receipts into a live iMessage claim.

## Sidebar refresh and local project removal — 2026-10-17

The sidebar now uses one neutral New chat action, on-demand search, a separate
Add project chooser, plain folder/chat rows, hover/focus actions, and a compact
288px default width (400px cap, including older persisted oversized widths).
Worktree actions are grouped in the project menu.

Remove project is an account-scoped local navigation preference. It no longer
starts an unawaited cloud archive operation: active chats and files are retained,
removed projects stay hidden when cloud history refreshes, and search can recover
their chats. Choosing a folder or opening a chat restores its project. Archive
all chats remains a separate cloud mutation.

Retained [design QA and interaction conclusions](design-qa.md) and
[desktop](docs/sidebar-refresh/01-desktop.png) /
[narrow-window](docs/sidebar-refresh/02-narrow.png) captures use synthetic sidebar
fixtures and the mock bridge in Codex's in-app browser. Verified populated and
active-project removal, persistence after reload, search/reopen recovery,
creation controls, rail search, menu keyboard focus, and the 600px drawer.
The projectSidebar preference tests cover removal persistence despite unarchived
chats and remembered folders, account isolation, exact SSH-root identity,
recovery, and legacy/malformed preference loading.

Frontend frozen install, typecheck, production build, and 844 tests passed
(3 skipped). Rust format, strict four-crate Clippy, 802 nextest tests (8 skipped;
run 8eacf5fc-ce8b-43a5-aa35-c81845a222fb), and core WASM check passed.
The unsigned canonical launcher compiled and started the local debug executable.
Native interaction, production auth failure, packaged/branded release, and live
SSH were not qualified. No hosted-model calls or release deployment occurred.

## Sidebar navigation audit — 2026-09-04

The local foundation sidebar was audited and repaired in the Codex in-app browser
using disposable projects and the mock bridge. The retained [audit and screenshots](docs/sidebar-audit/README.md)
record folder-based session creation, visible project/conversation actions,
explicit rename, keyboard menu navigation and focus return, archive/search/restore,
and a dismissible narrow-window navigation drawer. Search indexes project paths
and aliases; archived matches appear in the search region. Empty specialist
catalogs no longer expose an empty disclosure.

Frontend typecheck, production build, and the full deterministic frontend suite
passed (835 passed, 5 skipped). These checks and the recorded browser interactions
do not prove a packaged native build, OS folder picker, live SSH, paid providers,
full accessibility compliance, or branded specialist/account flows. Failure and
pending-start cancellation handling in the chooser was reviewed in source, not
fault-injected in the browser.

Browser-preview Quick Chat follow-up: `mockBridge.sidebar.spec.ts` now verifies
that default-generated workspace IDs are distinct UUIDs and that two new Quick
Chats form one `quick-chats` sidebar group. The three focused mock-sidebar tests
and frontend typecheck passed. This corrects preview IDs only; it does not migrate
previously created `mock-workspace-*` fixture history or change native workspaces.

Additional sidebar papercuts (same audit): `sidebarProjectTarget.spec.ts` pins
exact remote-project folder selection even when a saved host's default changed,
and search text matching the visible Quick chats and project-alias labels.
`newProjectDialog.spec.ts` pins retention of an edited or cleared folder when
saved hosts refresh. `hotkeys.spec.ts` pins consumed-event ownership and rejects
modal Tab navigation as a background global shortcut. The folder chooser also
filters hidden controls from its focus loop and includes disclosure summaries.

In-app browser checks confirmed Quick chats search, Quick Chat composer focus,
Command-backslash opening the narrow-window drawer, and reverse/forward Tab
wrapping inside the session chooser. The first keyboard check exposed background
Shift-Tab changing approval policy; after the fix, repeated modal Shift-Tab left
Approve for me unchanged. The preview policy was restored before continuing.
A development hot-reload hook-order error during editing was recovered with a
fresh page load; subsequent interactive checks passed. Exact remote-folder and
host-refresh contracts are deterministic checks, not live SSH acceptance.
Final frontend typecheck/build and full suite passed: 843 passed, 5 skipped.

## Transcript rendering stability — 2026-10-04

`node harness/rendering-stability.mjs` (or `pnpm --dir harness test:rendering-stability`)
drives the real UI with deterministic local snapshots.
It covers saved execution/research receipts, disclosure scroll ownership,
streaming while reading, returning to the live tail, code-fence identity on
completion, and the reply reserve's height. It runs Chromium and WebKit, each
with normal and reduced motion; no hosted providers are called.

The initial Chromium reproduction found all 60 saved execution rows entering
with opacity below 1, a 224 px upward displacement of the clicked tool header,
a replaced code-fence DOM node on completion, and 8 intermediate height frames
during normal-motion completion (3 with reduced motion). These are deterministic
DOM/geometry observations, not latency benchmark results.

The corrected implementation keeps one Markdown block renderer through answer
completion, scopes row identity to its conversation, renders tool receipts
without mount animations, and gives explicit disclosure interaction ownership
of the viewport. The reserve collapses once on completion. Reduced-motion CSS
limits transitions to paint properties: the previous universal duration applied
to the default `all` property and accidentally animated otherwise static layout.

All four browser configurations passed: 60 mounted receipts with zero faded
rows; zero header, reading, or scrollback displacement; retained code-fence
identity; zero intermediate completion-height frames; and zero page errors.
The harness also verifies that Jump to latest restores following as output grows.

Related acceptance: workspace navigation passed all four browser/motion
configurations, highlighting replacement/growth passed both engines, and
permission/toast exit behavior passed Chromium with both motion preferences.
The workspace harness's Settings locator now targets the labeled toolbar
button because the current remote-agent notice also contains a Settings button.
Frontend frozen install, typecheck, production build, and 816 full-suite tests
passed (3 skipped); the subsequently added Markdown settlement regression and
updated research-motion test passed in the focused 7-test run. Rust formatting,
strict Clippy, 802 nextest tests (8 skipped), and the agent-core WASM check passed.

The local streaming profiler failed machine quiescence and used development
instrumentation. Its timings are not a before/after performance baseline.
Packaged-native interaction latency remains unmeasured for this change.

Release-scope verification on 2026-10-04: an archive of the staged UI-only
candidate, excluding unrelated local history/cloud changes, passed 812 frontend
tests (5 skipped), the production build, and all four rendering-stability browser
configurations. The working-tree counts above include other in-progress work.

## UI highlighting and motion — 2026-09-05

Syntax highlighting now uses a module worker in browsers. The worker loads
language grammars on demand; the production worker entry is about 190 kB
uncompressed rather than an inline bundle of all grammars. Browser callers
share duplicate requests, match replies by request identity, and retain plain
code if worker initialization, messaging, or a 15-second request deadline fails.
The existing source-identity guard still prevents stale highlighted markup from
covering a replacement code fence. Server-side rendering uses the same engine
without a worker.

Validation on the current working tree: frontend typecheck/build and 847 tests
passed (5 skipped). `highlight.worker.spec.ts` covers duplicate and out-of-order
requests, separate line/block output, worker failure, and stalled requests.
`node harness/highlight-staleness-probe.mjs` passed in Chromium and with
`ENGINE=webkit` in the WebKit proxy. The cold-worker step waits for actual
highlighted output, bounded at 15 seconds, instead of assuming a 900 ms load.

The two 12-turn Chromium streaming probes (301 snapshot pushes each) retained
under `target/perf/20260905T075115Z-ad5abaee-streamA1-chromium` and
`target/perf/20260905T075346Z-ad5abaee-streamA1-chromium` provide local attribution:
Shiki regex tokenization appeared in the first main-thread profile and left the
main-thread leaderboard after worker isolation. Both runs failed machine
quiescence and used Vite development mode plus a profiler. Their timing and
frame-loss figures are not authoritative performance baselines; the disposable
traces are not release evidence. Controlled packaged-native timing remains
unmeasured; navigation/streaming acceptance is recorded below.

Navigation acceptance on 2026-09-05: `node harness/workspace-motion.mjs`
passed in Chromium and WebKit, each with normal and reduced motion. It proves
stable workspace-node identity across chat switches, per-conversation draft
separation, preserved draft/focus during a streaming update, no repeated
navigation animation on that update, no blank-opacity navigation phase, and
settings scroll reset. Normal navigation uses the shared 200 ms transform cue;
reduced motion uses a 120 ms opacity cue. These are deterministic browser
fixtures, not hosted-model or cross-platform packaged-release receipts.

`./script/build_and_run.sh` compiled and launched the unsigned native debug
application successfully in 28.67 seconds, and the WebView loaded the frontend.
This is native startup evidence, not a measured native interaction benchmark.

Final regression checks for this UI change: frontend typecheck, production
build, 847 frontend tests (5 skipped), Rust formatting, Clippy for the four
foundation crates, 816 nextest tests (8 skipped), and the agent-core WASM
compile check passed. No hosted-provider calls or release deployment were run.


## Autoregressive prompt and tool audit — 2026-09-05

Scope: foundation system and per-turn prompt assembly, steering, local tool
schemas, computer-action schemas, delegated review receipts, and the local
schema-to-wire adapter. The audit inspected existing planning, file-edit,
memory, image, device, and deferred-tool ordering as well. Product-owned
prompt overrides and third-party MCP schema authorship are outside this result.

The governing rule is dependency order: generate the concrete inputs needed
for a judgment before its verdict, and receive a tool result before generating
arguments that depend on it. This is distinct from sorting every field as
"rationale first" or "action first". Schema order guides model generation;
it does not enforce JSON output order or establish correctness.

| Finding | Change |
| --- | --- |
| `verify_effect` chose status before read-back evidence and comparison values | `effect_id → evidence → expected → observed → status` |
| Delegated review resolution chose accept/rework before feedback | Nested decisions now use `task_id → feedback → decision` |
| Enterprise feedback chose outcome before evidence and summary | `evidence_refs → summary → outcome` |
| Final delivery composed prose before enumerating deliverables | `files → content`; files remain optional and independently validated |
| All eight computer-action preparation schemas classified risk before selecting the observed target and action | Observed application/window identity and action arguments precede reason and advisory risk; trusted backend classification is unchanged |
| PoC generation selected its success exit code after writing the script | Expected observation and expected exit code precede script |
| In-flight steering appended attached text after the user's request | Attachments precede the explicit final user-request block, matching ordinary turn assembly |
| System communication rules did not state the cross-call generation dependency | Independent batching is explicit; dependent arguments wait for a subsequent model turn containing the prerequisite result; effect verification instructions precede completion instructions |
| Implementation guidance forced an edit after eight inspection calls | Edits follow inspection of the target, existing contract, and required dependencies; no arbitrary read quota forces premature implementation |

Preserved: field names, required-field membership, invocation parsing, runtime
permissions, approval requirements, and effect/delivery validation. The optional
feedback and artifact fields remain optional, so order alone cannot compel
evidence disclosure. Existing edit locate-before-payload and planning
contract-before-prose rules remain. MCP schemas are passed through unchanged;
we do not alphabetize or rewrite third-party contracts. The adapter clones
schema parameters into the model request.

Regression coverage now checks actual serialized `properties` keys rather than
searching the entire schema text, which could accidentally match descriptions
or `required` entries. Existing prompt and schema tests were moved into separate
modules to keep source files below the repository's size thresholds.

Validation on the current local source:

- Nextest `88a3484d-bf10-417a-9735-23b43bd35739`: **819 passed, 8 skipped**
  across agent-core, provider-acp, and provider-local. Includes schema order,
  nested review ordering, steering attachment order, prompt dependency order,
  computer actions, and existing effect/delivery contracts.
- Workspace formatting check passed. Required clippy gate passed for agent-core, provider-acp, provider-local,
  and devbridge; agent-core WASM check passed.
- Frontend frozen install, typecheck, **847 tests passed / 5 skipped**, and
  production build passed.
- The unsigned `./script/build_and_run.sh` path built and launched `clark-code`
  successfully. No hosted conversation or visual UI qualification was performed.
- These checks use the existing local Cargo configuration, which redirects the
  pinned agent dependency to the sibling checkout. They are not a clean CI or
  packaged-release qualification.

No hosted-model calls were made. Improved live-model reliability, provider-side
schema ordering, and behavioral effect sizes remain unmeasured.


## Security agent simplification — 2026-09-25

Security now uses `security:assistant` by default in the private composition;
standard/diff/deep scans remain explicit skills. The foundation removes the
mandatory Git start gate, organization-bound Insights canvas, cloud scan client,
and implicit scan-history uploads. Local evidence history and PDF export remain.
Saved Security context is reduced to kind/workflow. The downstream repositories
remove the web dashboard, organization scan backend, cloud exporter and Security
advisor while retaining ordinary access, account conversation sync and reusable
security tools. See [architecture and ownership](docs/security-agent-simplification.md).

The optional `research_rank` tool delegates ranking to published
`clark-autoresearch =0.2.0`, accepts 1–64 caller-described opportunities, preserves
references/rationale, rejects invalid/duplicate input and filters out-of-scope
items. It requires no organization, network call, persistent graph or scan state.
Four tests cover the real ranking library, input validation, registry discovery
and wire ordering. Rankings do not verify findings or establish scan quality.

Current local receipts:

- Rust nextest `8be7f428-e4e3-4cf9-95ff-9da628275055`: **789 passed, 8 skipped**
  across agent-core, provider-acp and provider-local. The new scripted-provider
  assistant test creates and reads a real file in a non-Git directory using the
  ordinary model, completes through `final_answer`, and creates no scan bundle.
  Existing explicit scan, evidence seal and PoC contracts also pass.
- Required formatting, Clippy (four foundation crates/all targets), and core
  WASM compile checks pass. The installed nextest is the pinned 0.9.143.
- Frozen frontend install and composed production build pass. Foundation plus
  product frontend suite: **834 passed, 5 skipped**, across 175 passing files.
  Includes migrated saved scope, account isolation, and local PDF invocation.
- Foundation product-boundary contracts: **13/13 passed**.
- Chromium matrices pass for the neutral fixture's general assistant/deep scan
  and the actual private product entry's general assistant/deep scan. They check
  desktop 1440×1000 and mobile 375×812, successful and failed starts, exact skill
  delivery, no org/repository/advisor recipe, preserved transcripts, and paid/free
  access. Browser errors and failed requests: zero. Local run directories under
  `target/specialist-matrix-smoke` are disposable; these are the retained conclusions.
- Unsigned `./script/build_and_run.sh` built and launched the native debug binary
  with its WebView loaded. The task-owned watcher was stopped after startup. No
  credentialed signing, Keychain access, hosted calls or deployment occurred.

Tests use the existing local Cargo configuration and sibling source overrides;
this is not an exact pinned-source CI, packaged release, production migration, or
live-model scan-quality receipt. Improved live scanning quality remains unmeasured.


Final product browser follow-up (2026-09-25): the current
`specialist-ui-smoke.mjs` passed paid/free previews, starter prefill, explicit
absence of Security Insights/domain controls, and mobile overflow checks.
`security-ui-smoke.mjs` passed artifact history populated/expanded/empty/error,
Escape close, and remote-hidden states. These are deterministic browser checks,
not hosted-model quality measurements.

## Security split-pane and artifact recovery — 2026-10-02

The neutral fixture's Security matrix passed with a 1440×1000 split-pane
screenshot and a heading/action non-overlap assertion; the artifact delivery
browser journey passed its six image/PDF and copy checks. Frontend validation
passed 813 tests with 5 skipped, typecheck and production build. Rust format,
Clippy, agent-core WASM check and the three-crate nextest run passed (801 tests,
8 skipped). These checks do not prove the patched packaged macOS WebView, the
user's existing synced conversation, or a live hosted-model route. The
metadata-only file state is covered by renderer tests; the screenshot's actual
file bytes and provider failure were not reproduced in the mock fixture.

Security cross-device history follow-up: after removing the four-conversation
limit and separating saved history from local execution, frontend tests passed
820 with 5 skipped, TypeScript typecheck and the production build passed. The
Security browser matrix passed against both the neutral fixture and the Clark
Code branded product composition: it retained a cloud-like saved transcript in
a read-only recovery view after provider attach failed, then resumed the same
conversation under an explicitly selected device-local folder while preserving
its shared cloud project metadata. Rust format, strict Clippy, the 801-test three-crate
nextest run, and agent-core WASM check passed. This is deterministic mock
evidence; the other machine's account index, native picker, and packaged
WebView remain unverified.


## Native account recovery and phone presence — 2026-10-17

The account boundary now carries explicit native failure codes for expired
sessions, rejected credentials, service outages, cancellation, and account
changes. Same-account renewal uses a credential/generation compare-and-write;
it can complete inside an admitted cloud read without upgrading that read to
an account lifecycle write lock. A stale completion cannot overwrite a newer
session or restore an explicitly signed-out initial sign-in.

Renderer recovery remains single-flight and replays an expired operation only
once. Temporary outages retain offline local access without opening a browser;
interactive reconnect preserves the active conversation, running ids, and draft.
Pending, cancellation, and retry errors appear in the workspace and account
menu. Phone command polling owns its busy/backoff state per account connection,
so an old request's completion cannot change the replacement loop's state.

Validation: frontend tests pass (849 passed, 3 skipped), typecheck and production
build pass. Native host tests pass (155/155), provider/core Nextest passes
(802 passed, 8 skipped), strict foundation Clippy, format check, and agent-core
WASM check pass. Account ownership, outage, cancellation-before-browser,
concurrent reconnect, exact replay, and live-work preservation have deterministic
regressions in `sessionStore.authActions.spec.ts`, `productBridge.spec.ts`, and
`src-tauri/src/product/tests.rs`.

The [component preview](docs/account-recovery/cancelled.png) was exercised through
browser controls with synthetic state. It proves the visible cancel/retry state
and editable retained draft, not a live native browser sign-in. Branded identity
provider, session transport, and product validation are recorded in the private
product catalog. These local source checks do not verify a packaged desktop or
physical Android build.

Release qualification follow-up: hosted browser CI found that the specialist
matrix still clicked the retired `New quick chat` label. It now uses the visible
`New chat` action and retains the assertions that ordinary chat collapses the
specialist navigation and that subscription gates block dispatch. The complete
local specialist matrix passed all 12 checks, including desktop/mobile history
and provider continuity, with no console errors or failed requests.


Active-stream recovery follow-up: renderer and native account preflight leave
`voice.stream.send`, `voice.stream.finish`, and `voice.stream.cancel` independent
of account renewal. These operations use their existing authenticated native
connection; opening a new stream still renews account access. The renderer
regression completes all three operations while renewal remains pending.
Frontend validation passes 850 tests (3 skipped) and TypeScript typecheck.

## Stable prompt refresh and host-owned project containment — 2026-10-21

The private Clark Code DeepSeek investigation found that refreshing an unchanged
skill catalog grew newlines in the first system message each turn. Catalog
replacement now preserves its exact position and owned separators. A 1,000-cycle
regression pins byte stability, plus update/removal of the section.

The same investigation reproduced English-derived sandbox narrowing: permission
to add tests under `tests/` blocked source files once that directory existed.
Natural-language directory inference and the secondary scope state were removed;
host-selected project roots and explicit approved roots remain authoritative.
Subproject sibling reads/writes remain rejected. Research artifacts follow that
same project root. An offline real-provider replay (zero hosted calls) proves
successful source access after test-directory creation and append-only request
prefixes across the follow-up.

`cargo nextest run -p provider-local --lib`: 670 passed, 7 skipped, run
`d314b5a6-fb46-406c-af62-8e29c51a62b2` on the release toolchain 1.97.1.
Strict provider-local Clippy also passes on 1.97.1. The concurrent 1.99.1
dependency-upgrade experiment remains excluded from this release.
Live private product comparisons and their narrower claim boundaries belong to
`../clark-desktop-product/crates/clark-desktop-product/EVALS.md`, not this
foundation's public benchmark lanes. Detailed report in sibling backend:
`../clark/docs/testing/2026-10-21-clark-code-deepseek-migration-investigation.md`.


## Host-selected image relay billing identity — 2026-10-24

`ImageGenerationConfig` now requires the host-selected chat model. Initial
provider connection and idle model changes use the same configuration
constructor, and image relay requests send that selection as `source_model`.
The image-generation tool schema does not expose the billing selection; extra
model-authored arguments cannot replace it. Generic host-owned model exclusions
remain supported. The foundation does not select an upstream image provider,
model, resolution, tier, or billing price.

The deterministic loopback regression
`image_relay_source_model_tracks_initial_selection_and_model_changes` exercises
the real registered image tool before and after a model change, verifies both
HTTP bodies plus authentication/idempotency, writes returned raster bytes to
the workspace, and checks excluded-model disable/re-enable behavior. A separate
regression checks key absence and generic exclusions. The existing ignored
paid-image harness now also requires `IMAGE_GENERATION_E2E_MODEL` for the
host-selected chat model; it remains opt-in.

Focused Nextest run `0042b47d-b3a1-48ac-8e63-ae5aeb577311`: **9 passed**,
including the two relay regressions, existing raster/path helpers, mutating-tool
classification and serialized image schema order. This is offline source-path
evidence using the current local dependency overrides. No hosted image provider,
deployed billing boundary, installed application, or release was exercised.

Full provider-local library follow-up: `cargo nextest run -p provider-local
--lib` passed **672 tests, 7 skipped**, run
`d5e75406-d0f2-4449-978c-f532319b9688`. `cargo check -p provider-local
--all-targets` also passed. Strict all-targets Clippy on Rust 1.99.1 is blocked
by existing `chunks_exact`/`chunks_exact_mut` lints in untouched
`exec-sandbox-protocol/src/lib.rs:134,533` and
`computer-use/src/simulator.rs:512`; no unrelated source was changed.

The downstream backend separately verified live 1K generation and reference
editing with included settlement and zero customer-credit debits. See the
[backend acceptance evidence](../clark/docs/testing/2026-10-24-free-flux-images.md).
This does not qualify an installed or packaged Desktop client.
