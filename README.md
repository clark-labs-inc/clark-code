# Clark Code

Public, cross-platform desktop client for agentic work. One UI that talks to
many agent backends through a single provider abstraction — **ACP** local CLI
agents and the **Clark** runtime — with the
performance-critical engine written once in Rust and shared across desktop,
mobile, and web.

## Standalone Clark CLI and TUI

Clark Desktop is not required. The standalone installer places the Clark-owned
`clark` TUI and its paired native specialist worker in the user's command path:

```bash
# macOS or x86_64 Linux
curl -fsSL https://www.clarkchat.com/install.sh | sh

# Open the human-facing TUI in the current project
clark
```

On Windows PowerShell:

```powershell
irm https://downloads.clarkchat.com/desktop/cli/install.ps1 | iex
clark
```

On first interactive use, `clark` offers browser sign-in, device-code sign-in,
or an existing Clark API key. A remote or headless machine can be configured
without opening any GUI:

```bash
clark login --device-code       # approve from another device
clark login --api-key           # reads the key from a hidden prompt or stdin
clark auth status
clark doctor
```

`CLARK_API_KEY` is also accepted for non-interactive CI. It is the cloud
credential; the Clark server resolves the account, paid specialist
entitlements, and normally the organization. `--organization` is needed only
when the credential can use more than one eligible paid organization.
Interactive login persists the credential in an app-owned ChaCha20-Poly1305
envelope and adjacent random key under `CLARK_HOME` (normally `~/.clark`). It
does not use Keychain, Credential Manager, Secret Service, or another OS vault.
With a TTY, `clark` opens the TUI. Supplying a prompt runs one headless turn,
including the paid specialist workspaces:

```bash
clark code "fix the failing test"
clark scout "map this system"
clark security --workflow scan "scan this repository"
clark scientist --workflow discover "test this hypothesis"
clark rsi --workflow create-evals "build a regression eval"

# Reopen the same account-scoped conversation from a script or another machine
clark --conversation CONVERSATION_ID scientist --workflow replicate "continue"
```

The TUI offers matching Clark Cloud conversations after workspace selection.
Desktop, TUI, and headless runs persist the same typed snapshot and specialist
binding; the CLI has no separate local session database. Every run fails before
starting a worker or model when account-scoped conversation synchronization is
unavailable. Specialist execution additionally requires live entitlement,
organization resolution, and specialist preflight synchronization. Scientist
and RSI completion also requires a verified cloud journal/artifact receipt.
Upgrade the verified paired binaries with `clark update`.

## Why

- **Lean by construction.** Tauri 2 (native WebView, no bundled Chromium) + a
  Rust core.
- **One engine, three targets.** Transport codec, event projection, and run
  lifecycle live once in the `agent-core` Rust crate and compile to native
  (desktop/mobile) and WASM (web) — instead of being re-implemented per platform.
- **Agent-agnostic, Clark-focused.** A trait-based `Provider` abstraction fronts
  every backend; the ACP and Clark adapters ride the same trait.
- **Beautiful & complete.** React 19 + Tailwind v4 UI: streaming chat, tool-call
  timeline, plan, permission gates, and an agent "computer" surface.

## Architecture at a glance

```
React (Tauri WebView)  ──invoke/emit──►  agent-core (Rust, native + WASM)
   surfaces, store            │              domain · projection · Provider trait
                              ▼
           provider-acp (JSON-RPC/stdio)   provider-local (native tool loop)   provider-clark (HTTP + SSE)
           local CLI agents via sidecar     local/remote project execution      remote Clark runtime
```

The same provider boundary can run without the desktop host:

```
JSONL control ──► code-host (projects · plugins · cancellation · trajectory)
                         │
                         ▼
                  code-worker / coding plugin
                         │
                         ▼
             agent-core::Provider + LocalExecutor

SSH control ──► code-remote (artifact + checksum + credential bootstrap)
                         │
                         ▼
              remote code-worker (model + tools + project)
```

`code-host` is provider-neutral and reusable. Workers register typed
`HeadlessPlugin` implementations at their composition root, so research,
evaluation, or GPU-specific capabilities can be added without changing the
control protocol. See [`crates/code-worker`](crates/code-worker/README.md) for
the standalone JSONL worker and its fail-closed permission configuration.
For a complete CPU/GPU-resident run, [`crates/code-remote`](crates/code-remote)
launches that worker over system SSH, verifies the uploaded binary, relays a
credential only through encrypted SSH stdin, and exposes correlated
ping/catalog/invoke/cancel/shutdown requests. The account-partitioned native
runtime registry is the only remote mode; conversations and project inspection
attach to its opaque worker capabilities.

## Autoregressive prompt & schema design

LLMs generate left-to-right, so *order is part of the prompt*. The local coding
agent (`crates/provider-local`) is designed around that:

- **Tool schemas are ordered prompts.** The model emits tool-call arguments in
  the property order the schema advertises, so every schema is authored
  locate-before-payload (`edit_file`: `path → old_string → new_string`),
  decide-before-write (`memory`: `action → scope → title → content`), and
  rationale-first (`update_plan`: `explanation → plan`). The workspace enables
  serde_json's `preserve_order` feature so authored order actually reaches the
  wire — stock serde_json alphabetizes `json!{}` maps, which had silently
  reversed `edit_file` into `new_string → old_string → path`. A regression
  test pins the wire order of the sensitive schemas.
- **The system prompt is position-aware.** Hard rules (the shared-worktree git
  rules) sit in the primacy slot directly after the identity line; volatile
  per-turn facts (a fresh `git status` snapshot, output style) are injected at
  the recency end of each turn message — which also keeps the cached system
  prompt prefix stable for prompt caching.

## Layout

| Path | What |
| --- | --- |
| `crates/agent-core` | Domain model, projection reducers, `Provider` trait, codecs. Native + WASM. |
| `crates/provider-acp` | Agent Client Protocol adapter (JSON-RPC over stdio). |
| `crates/provider-clark` | Clark runtime adapter: HTTP command writes (`/api/conversation-sync/commands`) + resumable SSE event stream, with a WS for realtime session binding. |
| `crates/provider-local` | Local coding loop with brokered Clark Cloud tools and a default-on native command sandbox. |
| `crates/code-host` | Reusable headless control plane: strict JSONL protocol, project registry, plugin interfaces, cancellation, and durable trajectory. |
| `crates/code-worker` | Standalone `clark-code-worker` composition root with the first-party coding plugin over `provider-local`. |
| `crates/code-remote` | Reusable SSH launcher/control client for worker-resident model/tool execution, versioned artifacts, checksum verification, credential bootstrap, and reconnect-safe request correlation. |
| `crates/exec-sandbox*` | Cross-platform policy, Seatbelt/bubblewrap adapters, and the Windows restricted-token privilege boundary. See [sandbox design](docs/sandboxing.md). |
| `crates/devbridge` | Dev-only WebSocket bridge that drives the real providers + projection from a browser (headless UI testing, video capture). Not shipped. |
| `src-tauri` | Tauri 2 host: commands, event bridge, sidecar, state. |
| `app` | Vite + React + TS + Tailwind v4 frontend. |
| `harness` | Playwright scripts for local smoke runs, diagnostics, and screen capture against the running app. |

## Develop

Prerequisites: Rust (stable), Node 24+, pnpm 10+, and the
[Tauri 2 system deps](https://v2.tauri.app/start/prerequisites/).

```bash
# Rust engine: test + lint
cargo test -p agent-core -p provider-acp -p provider-clark -p provider-local
cargo clippy -p agent-core -p provider-acp -p provider-clark -p provider-local -p devbridge --all-targets -- -D warnings
cargo fmt --all --check

# Frontend (browser preview uses a mock provider)
cd app && pnpm install && pnpm dev      # http://localhost:1420
pnpm test                               # vitest
pnpm typecheck

# Run the desktop app with the isolated, stably signed development identity
./script/build_and_run.sh
```

### Owner-local capture

The native host and React error boundaries write the shared Clark Capture v1
event format when `CLARK_CAPTURED_LOGS` points to an absolute, non-symlinked
directory. The development launcher enables it at `$CLARK_LOGS/captured` by
default and restricts the directory to the current owner (`0700`). Normal
release launches remain opt-in through the environment variable.

```bash
export CLARK_CAPTURED_LOGS="$HOME/Library/Logs/ClarkCapture"
./script/build_and_run.sh

# From an installed clark-capture analyzer:
clark-capture latest --project clark-desktop --level error -n 20
clark-capture dedup --project clark-desktop --mode semantic --since 24h
```

Existing Rust `tracing` events and native panics are captured directly. Browser
errors, unhandled rejections, and contained React failures cross a typed Tauri
boundary as an opaque reference plus message-free stack frames; raw exception
messages, conversation content, credentials, and request bodies are not copied
to disk. Capture failures never interrupt the application.

In a plain browser the UI runs against a **mock provider** that plays a scripted
streaming run, so every surface is demonstrable without the native host.

## Resilience benchmark

The Playwright resilience benchmark drives the real Clark Code conversation UI
through every combination of six independent conditions: rate limiting,
duplicated provider tool-call IDs, event-stream disconnects, provider-process
loss, delayed cloud-history acknowledgment, and explicit user cancellation.
That is a deterministic 64-case power set, not 64 paid model calls. Each case
asserts that the conversation stays rendered, internal tool-call IDs remain
hidden, incident state settles, and interrupted/cancelled work can continue
from saved progress. Screenshots and a JSON receipt are written to a temporary
artifact directory printed at the end of the run.

```bash
cd harness
node resilience-benchmark.mjs
```

For a quick representative pass, `--smoke` runs the healthy baseline, each of
the six faults in isolation, and the all-faults interaction case:

```bash
cd harness
node resilience-benchmark.mjs --smoke
```

A separate control drives the real local provider through `devbridge` using
the included Clark-managed `clark-code:free` route backed by DeepSeek V4 Flash Latest. It is intentionally
opt-in because it makes live model calls against the account's included weekly
allowance. Set `CLARK_CODE_API_KEY` (or keep
it in the repository's gitignored `.env`) and run:

```bash
cd harness
node resilience-benchmark.mjs --live-only
```

The shared contract in
`app/src/core-bridge/resilienceBenchmark.json` pins the matrix dimensions,
provider, and model for both the TypeScript tests and Playwright harness so the
reported configuration cannot drift from the simulated one.

## Pre-release experience benchmark

Every release build is gated on a fast cross-product sample: core event and
provider contracts; native desktop commands, account boundaries, and local
persistence; local tools, permissions, memory, planning, and recovery; scripted
conversations; remote execution, git, and worktrees; frontend state and surfaces;
computer-use safety and simulator contracts; macOS, Linux, and Windows sandbox
contracts; WebKit startup; attachment rendering; an eight-case browser resilience
sample; and the full empty-home skill experience. The wrapper executes the
authoritative deterministic lanes from the checked-in feature map first, so a
new mapped lane cannot remain a documentation-only claim.
The skill lane covers legacy-compatible discovery, managed Superpowers install,
collision-safe selection, typed history, provider request projection,
update/stale-binding handling, restart, uninstall, and the same lifecycle against
a fake remote user home. It runs against both a self-contained fixture and a
pinned real Superpowers checkout, then uploads one suite receipt, the detailed
skill receipt, browser screenshots, and per-family logs.

```bash
./scripts/run-pre-release-benchmarks.sh \
  --superpowers /path/to/obra/superpowers
```

A representative paid real-model lane is part of the same gate by default. It
uses the pinned Clark Platform Qwen 3.7 Flash route and exercises a managed skill resource, basic response,
read/search tools, permissioned mutation, a memory round trip, and explicit
compaction/continuation. Provider, endpoint, and model are pinned; only the
credential is local:

```bash
./scripts/run-pre-release-benchmarks.sh \
  --superpowers /path/to/obra/superpowers
```

The runner also reads `CLARK_CODE_API_KEY` from the ignored repository `.env`
without executing the file. Use `--offline` for an explicit deterministic-only
run. Tag releases default to the paid lane and require the
`CLARK_CODE_PRERELEASE_API_KEY` repository secret; set the
`CLARK_CODE_PRERELEASE_LIVE` repository variable to `0` only for a deliberate
offline release gate. Missing or invalid live configuration fails before builds
or publishing instead of silently skipping the paid evidence.

Before cross-repository, UTM, or SSH work, use the redacted environment
preflight. It discovers the existing ignored Desktop and Clark env files,
checks their permissions, validates `utmctl`, exact QA guests, SSH reachability,
and the remote-worker configuration without printing credentials. Remote host,
root, trajectory, and HTTPS route have safe defaults; only the worker,
credential environment, paid model, and receipt path must be supplied for the
remote lane:

```bash
node harness/release-environment-preflight.mjs --all \
  --out target/release-preflight/all
```

`harness/clark-code-feature-map.json` and
`harness/clark-code-capability-inventory.json` are the checked-in coverage
contract. `node harness/feature-matrix.mjs --validate-only` derives model tools,
native commands, provider operations, models, settings, plugins, workspace
crates, permission classes, sandbox backends, computer actions, and incident
states from source; adding an unmapped item makes validation fail. Benchmark
directories and receipts are created owner-only.

### UTM guest readiness

Windows and Ubuntu Desktop real-use testing uses UTM only. VM provisioning,
login, guest transport, source staging, native product launch, Clark-owned QA
authentication, evidence capture, and recovery are autonomous release
prerequisites: every receipt must report `required_user_vm_actions: 0` and
`manual_vm_actions_allowed: false`. A physical-human input path is diagnostic
only and cannot satisfy a release gate.

The deterministic guest path is:

```bash
node harness/utm-autonomy.mjs ensure --platform all
node harness/utm-source-stage.mjs stage --platform all
node harness/utm-guest-provision.mjs ensure --platform all
node harness/utm-guest-benchmark.mjs run --offline --platform all
node harness/utm-windows-journey.mjs auth-smoke
node harness/utm-ubuntu-journey.mjs auth-smoke
```

The auth journeys mint a short-lived session for the ignored, owner-only
Clark QA identity, require the `clarkslabs.com` domain, verify that the
provisioned Clark Code key is bound to the same account, and never write the
email, password, JWT, or API key into a receipt.

The read-only
preflight never starts, stops, installs, or changes a VM; it verifies the exact
checked-in VM identities, guest-agent connectivity, graphical desktop state,
Clark Code installation, and each platform's sandbox prerequisite:

```bash
node harness/utm-real-use.mjs \
  --platform all \
  --observation-receipt target/utm-real-use-observations/DATE/receipt.json \
  --out target/utm-real-use/preflight
```

Pass the same observation receipt to the consolidated gate to make both guests
release-blocking:

```bash
./scripts/run-pre-release-benchmarks.sh \
  --utm-observation-receipt target/utm-real-use-observations/DATE/receipt.json
```

An environment-ready receipt is only permission to begin guest testing. It does
not count any Windows or Ubuntu chat, job, computer-control, permission, or
sandbox scenario as passed; those require separately exported real guest
receipts. If either guest is unready, the consolidated runner records the exact
blocker and skips its default cheapest-paid calls so no credits are spent
against a known-broken environment.

Once a guest is ready, generate its exact observation template inside that
guest, replace every blocker with fresh GUI assertions and evidence, and run
the platform benchmark:

```bash
node harness/platform-real-use.mjs \
  --write-observation-template target/real-use-observation.json

node harness/platform-real-use.mjs \
  --observation-receipt target/real-use-observation.json \
  --out target/platform-real-use
```

The second command must run on the platform it claims. It validates every
scenario and evidence hash before spending credits, then runs all applicable
deterministic lanes plus the pinned Clark Platform Qwen 3.7 Flash route by default. A complete pass requires
all mapped real-use scenarios, a positive cost below the checked-in ceiling,
and no stored credential. `--offline` remains useful for diagnosis but cannot
produce a complete guest pass. Exported packages can be independently verified
and copied without model calls:

```bash
node harness/platform-real-use.mjs \
  --verify-receipt target/platform-real-use/receipt.json \
  --copy-to target/verified-platform-real-use
```

To make cross-platform real use part of the single release receipt, pass one
verified package from each platform. Supplying any package makes the complete
three-platform set and the current UTM preflight release-blocking:

```bash
./scripts/run-pre-release-benchmarks.sh \
  --utm-observation-receipt target/utm-observation/receipt.json \
  --real-use-receipt target/macos-real-use/receipt.json \
  --real-use-receipt target/windows-real-use/receipt.json \
  --real-use-receipt target/ubuntu-real-use/receipt.json
```

## Repository Status

Clark Desktop is open source under the Apache-2.0 license. The local ACP and
provider-local paths can be developed without Clark Cloud access; Clark
provider, specialist, release, and hosted-worker paths require the
corresponding Clark services and credentials. Do not commit credentials,
customer data, screenshots from authenticated environments, or generated
release artifacts.

This is a clean-room implementation: no code from the main Clark repository is
copied into this client; shared behavior is reimplemented against provider
contracts. See [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md)
before opening a pull request or reporting a vulnerability.
