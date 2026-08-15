# Clark Code evaluations, benchmarks, and simulations

Last source-and-artifact audit: 2026-08-12.

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

## Current map

| Surface | Entrypoint | Evidence class | Claim boundary |
| --- | --- | --- | --- |
| Open-source boundary | `node --test harness/product-boundary.spec.mjs` | deterministic repository-wide text, dependency, and metadata contract | Rejects hardcoded hosted-service policy, commercial access rules, release credentials, and deployment-specific transports |
| Core domain and providers | `cargo nextest run -p agent-core -p provider-acp -p provider-local` with cargo-nextest `0.9.143` | deterministic Rust contracts | Proves provider-neutral projection, ACP translation, and local loop/tool behavior |
| Awaited background recovery | `env -u RUSTC_WRAPPER cargo nextest run -p provider-local eval_awaited_build_survives_provider_outage_without_keep_going` | deterministic real provider-loop simulation with local shell execution and a scripted SSE outage | Proves one user prompt can start a background build, time out while awaiting it, exhaust request-local transport retries, consume exactly one terminal build receipt during whole-run recovery, and finish with a typed `Done` outcome. Retained 2026-08-14 receipt: nextest `ec4649e9-ad89-4403-b8eb-f5a8b5eabf85`, 1/1 passed in 3.692s. It does not prove a live provider, packaged app, or deployed product. |
| Public live benchmark interface | sibling `../clark-public-evals` package, with Clark Managed and the downstream Clark Code public CLI as separate targets | externally owned, opt-in Free-tier live evaluation | Routes Finance Agent v2, Terminal-Bench, DeepSWE, BrowseComp, WebTailBench, Online-Mind2Web, and OSWorld-Verified without importing branded policy into this foundation. Scores and release claims belong to the external package and downstream Clark release; this repository owns only the provider contracts they exercise. |
| Native host | `cargo nextest run -p desktop-foundation --lib` | deterministic native command contracts | A packaged application still requires platform verification |
| WASM core | `cargo check -p agent-core --target wasm32-unknown-unknown` | compile contract | Proves the domain crate remains WASM-clean |
| Frontend | `pnpm --dir app typecheck`, `pnpm --dir app test`, `pnpm --dir app build` | deterministic TypeScript, component, and bundle contracts | Proves the checked Clark Code frontend bundle |
| Model picker UI | `node harness/model-picker-smoke.mjs` | deterministic browser-bound UI-only interaction with screenshot and typed receipt | Proves the composer model menu is portaled above the workspace, stays inside a compact viewport, and accepts pointer selection in both directions; packaged native WebKit behavior remains a separate platform receipt |
| SSH execution-target picker | `node harness/ssh-settings-smoke.mjs` | deterministic browser-bound UI-only interaction with SSH discovery/probe fixtures, screenshots, and a typed receipt | Proves a host can be saved before choosing its default folder, an add-host action from the composer selects that exact host as the remote execution target, and incomplete targets remain actionable without a premature Git connection; live SSH and packaged native behavior remain separate receipts |
| Pragmatic drag and drop UI | `node harness/pragmatic-dnd-smoke.mjs` | deterministic browser-bound UI-only interaction with screenshot and typed receipt | Proves pinned-project pointer reordering, the equivalent exact-position menu with focus restoration, desktop-file drop attachment, and the equivalent file picker; packaged native WebKit/OS drag behavior remains a separate platform receipt |
| Artifact delivery UI | `node harness/artifact-delivery-smoke.mjs` | deterministic browser-bound mock-provider journey with screenshot and download receipts | Proves inline image decoding, real PDF page rendering, visible artifact actions, image/PDF save-copy delivery, and artifact-workspace rendering; packaged native save dialogs remain a separate platform receipt |
| Cloud composer drafts | `pnpm --dir app test -- cloudComposerDraft.network.spec.ts cloudComposerDraft.spec.ts layoutPolicy.spec.ts composerDraft.spec.ts sessionStore.composerDraft.spec.ts`; downstream `cargo nextest run -p conversation-cloud`; `CLARK_REQUIRE_DESKTOP_DRAFT_DB=1 cargo nextest run -p clark-service-db --test desktop_draft_cas_e2e`; and `CLARK_REQUIRE_DESKTOP_DRAFT_DB=1 cargo nextest run -p clark-services --test desktop_draft_http_e2e` | deterministic frontend state-machine and native HTTP-codec checks plus real Axum/auth/Postgres CAS | Proves scoped keys, authoritative 204-to-revision-zero handling, conditional accepted-text clearing, bounded typed conflict handling, specialist-key URL encoding, create/update/payload-stable mutation replay/idempotent-clear/stale/concurrent CAS behavior, and authenticated HTTP response shapes; it does not prove the repaired service is deployed |
| Local sandbox | `cargo nextest run -p exec-sandbox` | deterministic policy and platform-adapter contracts | Packaged and signed binaries require a separate platform receipt |
| Durable worker | `cargo nextest run -p code-host -p code-worker -p code-remote -p provider-remote-worker` with cargo-nextest `0.9.143` | deterministic protocol and confinement contracts | Ignored live SSH lanes require explicit authorization |
| Orchestration | provider-local orchestration benchmark tests | deterministic fixture contracts | Does not claim live-model quality |
| Memory and goals | provider-local memory and goal eval tests | deterministic fixture contracts | Live-model quality is separate and opt-in |
| Scout human authority | `pnpm --dir app test -- ComposerContextBar.spec.ts localAgent.spec.ts sessionStore.scoutAuthority.spec.ts sessionStore.scoutStart.spec.ts` | deterministic renderer/session contracts | Proves explicit organization/workspace binding, enterprise-perimeter composer semantics, neutral checkout census roots, and refusal to start or reopen unbound Scout work; it does not prove a live census |
| Scout perimeter discovery | `cargo nextest run -p scout-adapter-runtime --lib` plus provider-local `census_reconciles_transport_equivalent_remotes_without_leaking_paths` | deterministic target-adapter, route-registry, and bounded local-checkout contracts | Proves GitHub organization and authenticated-user repository pagination, opaque local checkout identity, and bounded manifest inspection; it does not prove current live credentials or complete enterprise access |
| Enterprise feature context | `cargo nextest run -p agent-core`, provider-local planning/tool tests, and downstream `cargo check -p clark-services` | deterministic domain, permission, and compile contracts | Proves typed revision pinning, host-scoped bounded reads, and fresh-confirmation feedback wiring; it does not prove deployed graph coverage or production tenancy |

Changing a foundation eval contract or authoritative result requires updating
this file.
