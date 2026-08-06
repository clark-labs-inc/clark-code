# Codex-to-Clark experience map

This document records the source-grounded comparison used to improve Clark
Desktop and the ClarkChat artifact handoff. It is intentionally organized by
authority and evidence boundaries rather than by UI screens.

## User-outcome map

```text
User request
├── Environment authority
│   ├── Which project/session is active?
│   ├── Which instruction and skill roots belong to it?
│   └── Which filesystem and account may the run access?
├── Agent work
│   ├── Observe authoritative state
│   ├── Plan from typed history
│   ├── Execute tools inside the selected containment boundary
│   └── Record ordinary failures as context for recovery
└── Completion evidence
    ├── Requested state exists
    ├── Requested external effect has a fresh receipt
    └── Requested file is delivered as an openable artifact
```

The product is complete only when the final branch is true. A file that exists
inside an inaccessible sandbox is work evidence, not delivery evidence.

## Read Millar production reconstruction

Read asked ClarkChat to build a working Penny Jar Voice Interviewer MVP using
ElevenLabs ElevenAgents and a Claude Desktop MCP integration, with transcript
and summary output, Penny Jar branding, and an easy Vercel deployment path.

The run:

1. Read the supplied product and brand material.
2. Built a Next.js project under the Clark sandbox.
3. Exercised mocked health, interview creation, participant link, token,
   completion, and summary flows.
4. Created a project ZIP under the sandbox output directory.
5. Did not complete real ElevenLabs credentials, Claude integration, or a
   deployed Vercel instance.
6. Told Read to use local Finder, Terminal, ZIP, or GitHub workflows that were
   unavailable from ClarkChat.
7. After Read clarified the environment twice, claimed the ZIP was attached and
   linked the internal sandbox path.

The terminal envelope carried only the sandbox path; it had no ready published
URL, content hash, size, or attachment receipt when the claim became visible.
The first broken contract was therefore:

```text
sandbox file exists
        X
user has a downloadable artifact
```

The visible confusion was downstream of that boundary error. Packaging the file
again, explaining Terminal more clearly, or changing the chat copy could not
solve it.

## Codex source map

The comparison used the current sibling checkout at `../codex`, not remembered
product behavior.

### Instructions

```text
Codex home instructions
        +
project AGENTS from root → cwd
        +
explicit override
        ↓
bounded, provenance-labelled instruction context
```

Relevant implementation:

- `../codex/codex-rs/codex-home/src/instructions/mod.rs`
- `../codex/codex-rs/core/src/agents_md.rs`
- `../codex/codex-rs/core/src/context/world_state/agents_md.rs`

Important properties:

- Project instructions are ordered by filesystem scope.
- Input has explicit byte budgets.
- Provenance and environment are preserved.
- World-state updates replace or remove prior derived instruction state instead
  of silently accumulating stale copies.

### Skills

```text
environment roots
├── project .agents/skills roots
├── user roots
├── system roots
└── plugin roots
        ↓
bounded discovery
        ↓
immutable skill snapshot
        ↓
context-budgeted catalog + typed invocation metadata
        ↓
watcher invalidation and refresh
```

Relevant implementation:

- `../codex/codex-rs/core-skills/src/loader.rs`
- `../codex/codex-rs/core-skills/src/loader/discovery.rs`
- `../codex/codex-rs/core-skills/src/service.rs`
- `../codex/codex-rs/core-skills/src/injection.rs`
- `../codex/codex-rs/core-skills/src/render.rs`
- `../codex/codex-rs/app-server/src/skills_watcher.rs`

Important properties:

- Discovery is bounded by depth, directory, entry, file, and content limits.
- Project/user/system/plugin scope remains explicit.
- Symlink traversal is deliberate rather than incidental.
- Cache identity includes the effective environment and policy.
- A watcher invalidates snapshots; it does not mutate a parallel hidden catalog.

### Approval and containment

```text
tool request
├── environment id
├── canonical command
├── cwd
├── tty
├── sandbox permission
└── additional permissions
        ↓
approval lookup
        ↓
deny/read-boundary checks
        ↓
sandboxed or explicitly elevated executor
```

Relevant implementation:

- `../codex/codex-rs/core/src/tools/approvals.rs`
- `../codex/codex-rs/core/src/tools/exec_policy.rs`
- `../codex/codex-rs/core/src/tools/sandboxing.rs`
- `../codex/codex-rs/protocol/src/approvals.rs`

The command string alone is not an authorization identity. Containment and
additional capabilities are part of the grant.

## Clark Desktop map

### Environment-owned project state

```text
session id
    ↓
native AppState session
    ↓
SessionEnvironment.checkout_root
    ├── project memory
    ├── project instructions
    ├── skill discovery
    └── local tool root
```

Project memory now follows this chain in
`src-tauri/src/commands/local.rs`. A caller-provided cwd is no longer read
authority for the memory viewer.

### Skills and instructions

Clark Desktop mirrors the useful Codex structure:

- Project, user, managed, and plugin roots have explicit precedence.
- Discovery is bounded and environment-aware.
- Skill catalogs are immutable revisions shared by UI and provider sessions.
- Managed packs use staged validation and content-addressed revisions.
- Local and fake-remote empty-user journeys exercise the same lifecycle.
- Instruction provenance records origin, path, precedence, and truncation.

Primary implementation is under:

- `crates/provider-local/src/skills/`
- `crates/provider-local/src/instructions.rs`
- `crates/provider-local/src/provider/`
- `crates/provider-local/examples/skill_experience_benchmark/`

### Shell authority

```text
command request
├── parse every shell segment
├── apply hard deny rules to every segment
├── classify network/host capability
├── consult remembered approval only for sandboxed offline execution
└── offer one-time approval at network/host boundaries
```

Primary implementation:

- `crates/provider-local/src/permissions.rs`
- `crates/provider-local/src/permissions_tests/command_scope.rs`

This prevents both a benign-prefix denylist bypass and reuse of a sandboxed
approval for unsandboxed execution.

### Sandbox selection

```text
Auto or Required
    ↓
construct sandbox manager
    ├── Enforced → SandboxedExecutor
    └── unavailable/setup incomplete → explicit failure

Disabled or DangerFullAccess
    ↓
LocalExecutor
```

Primary implementation:

- `crates/provider-local/src/provider/isolation_setup.rs`

The executor no longer silently changes from managed containment to host access.

### Clark cloud authority

```text
endpoint
├── parse exact URL
├── require approved Clark host and secure production scheme
├── reject credentials, query, fragment, and unexpected path
└── disable redirects
        ↓
bearer accepted by Clark
        ↓
validated JWT subject
        ↓
native account authority
        ↓
account-scoped conversation cache
```

Primary implementation:

- `src-tauri/src/commands/cloud_authority.rs`
- `src-tauri/src/commands/cloud_conversations.rs`
- `src-tauri/src/commands/cloud.rs`
- `src-tauri/src/state.rs`

The WebView no longer chooses either the bearer destination or the local
conversation owner partition.

## ClarkChat terminal delivery map

The Clark backend implementation lives in the sibling `../clark` checkout.

```text
workspace file
    ↓
message(action="result").artifacts
    ↓
normalized terminal artifact metadata
    ↓
artifact mirroring/proxy resolution
    ↓
openable chat artifact card
```

Terminal validation rejects:

- A visible `/home/user/workspace` path.
- A claim that a file is attached or downloadable when no concrete terminal
  artifact survived validation.

It still accepts:

- Ordinary text-only answers.
- Honest statements that delivery failed.
- Attachment language when backed by typed artifact metadata.
- References to source material attached by the user.

Primary implementation:

- `../clark/crates/clark-agent-bridge/src/tools/message.rs`
- `../clark/crates/clark-agent-bridge/src/tools/message_delivery_guard.rs`
- `../clark/crates/clark-agent-bridge/src/tools/message_tests.rs`

## Benchmark map

### Deterministic pre-release gate

`scripts/run-pre-release-benchmarks.sh` samples:

```text
all authoritative deterministic lanes from the feature map
├── frontend typecheck + tests
├── provider and local-agent Rust contracts
├── computer-use simulator + security contracts
├── macOS / Linux / Windows sandbox contracts
├── WebKit startup + attachment rendering (where supported)
└── UI resilience sample
core/provider contracts
native commands + account boundaries + local persistence
local tools + permissions + memory + planning + recovery
scripted conversations + continuation
remote execution + git + worktrees
frontend state + projection + surfaces
UI resilience fault matrix
16-stage empty local/remote user skill journey
```

The release workflow runs this gate before native builds and uploads its receipt
and logs.

### Default cheapest-paid model lane

The consolidated runner executes the live lane after deterministic contracts
pass. Provider, endpoint, and model are pinned to
`clark-platform` / `clark-code:minimax_m3`; it fails closed without a
credential. `--offline` is the explicit opt-out. It samples:

- Basic text response.
- Managed skill and linked resource use.
- Directory listing, glob, grep, and file read.
- Permissioned write, edit, shell, and readback.
- Project memory write and recall.
- Explicit compaction and continuation.

### UTM real-use environments

Windows 11 ARM and Ubuntu 24.04 Desktop are tested in UTM only. The checked-in
inventory pins their exact VM names, graphical-session evidence, guest-agent
requirements, Clark Code installation, native sandbox prerequisite, and real
chat/job scenario contract.

`node harness/utm-real-use.mjs` is a read-only preflight that emits an
owner-only receipt. It never changes VM state and never upgrades readiness into
a feature pass. The consolidated gate accepts `--utm-preflight` or
`--utm-observation-receipt PATH`; an unready requested guest blocks the paid
lane before any provider call. Real Windows and Ubuntu passes require receipts
exported from the guests after executing the mapped scenarios.

Inside each ready guest, `node harness/platform-real-use.mjs` generates the
exact observation template, validates fresh GUI assertions and SHA-256-pinned
evidence before any provider call, and then runs the authoritative matrix.
The cheapest-paid MiniMax M3 route is the default. A deterministic-only
`--offline` run remains visibly incomplete because the paid real-use scenario
is skipped. A separate
`--verify-receipt` path replays the evidence, coverage, model, cost, and matrix
integrity checks without trusting the guest's claimed status.

The top-level pre-release runner accepts repeatable `--real-use-receipt` inputs.
Once any is supplied, it requires the exact macOS, Windows, and Ubuntu set,
independently verifies and copies every package, rechecks current UTM readiness,
and records all platform coverage and paid cost summaries in the one root
receipt.

### Read/Penny Jar production regression

`../clark/evals/scenarios/custom/penny_jar_downloadable_handoff.yaml` starts in a
fresh sandbox and reproduces the three-turn ClarkChat experience:

1. Build and verify the starter project.
2. Ask what to download and how to test and integrate it.
3. Clarify that internal paths, Finder, Terminal, and GitHub are not acceptable
   substitutes for a chat download.

Its hard artifact contract requires:

- The project manifest and README.
- A nonempty verified ZIP.
- A terminal structured ZIP artifact.
- No visible workspace path.
- No unsupported “attached” claim.

It is discovered by the normal Clark `make smoke` suite and can be run alone as
a fast paid pre-release replay.

## Completion invariants

| Boundary | Required invariant | Evidence |
|---|---|---|
| Project memory | Root comes from the native live session | Native unit test |
| Bearer forwarding | Destination is an exact Clark origin; redirects disabled | Origin and redirect tests |
| Command deny | Every shell segment is checked | Chained-command tests |
| Remembered approval | Network/host capability cannot inherit sandbox approval | Permission tests |
| Conversation cache | Owner derives from validated native account authority | Cross-account test |
| Sandbox fallback | Auto/Required fail closed | Isolation policy test |
| File delivery | Attachment/download claim has typed terminal artifact | Bridge tests |
| Read experience | Fresh sandbox produces an actually downloadable ZIP | Hard artifact eval |

Cheapest-paid MiniMax M3 execution is the default consolidated release
decision. Provider, endpoint, model, maximum iterations, and the inter-test cost
ceiling are pinned; `--offline` is the explicit no-network/no-credit exception.
