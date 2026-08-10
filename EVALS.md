# Clark Code evaluations, benchmarks, and simulations

Last source-and-artifact audit: 2026-08-07.

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
| Core domain and providers | `cargo test -p agent-core -p provider-acp -p provider-local` | deterministic Rust contracts | Proves provider-neutral projection, ACP translation, and local loop/tool behavior |
| Native host | `cargo test -p desktop-foundation --lib` | deterministic native command contracts | A packaged application still requires platform verification |
| WASM core | `cargo check -p agent-core --target wasm32-unknown-unknown` | compile contract | Proves the domain crate remains WASM-clean |
| Frontend | `pnpm --dir app typecheck`, `pnpm --dir app test`, `pnpm --dir app build` | deterministic TypeScript, component, and bundle contracts | Proves the checked Clark Code frontend bundle |
| Local sandbox | `cargo test -p exec-sandbox` | deterministic policy and platform-adapter contracts | Packaged and signed binaries require a separate platform receipt |
| Durable worker | `cargo test -p code-host -p code-worker -p code-remote -p provider-remote-worker` | deterministic protocol and confinement contracts | Ignored live SSH lanes require explicit authorization |
| Orchestration | provider-local orchestration benchmark tests | deterministic fixture contracts | Does not claim live-model quality |
| Memory and goals | provider-local memory and goal eval tests | deterministic fixture contracts | Live-model quality is separate and opt-in |

Changing a foundation eval contract or authoritative result requires updating
this file.
