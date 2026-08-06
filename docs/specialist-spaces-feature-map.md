# Clark specialist spaces

Status: implementation contract
Visual target: Specialist Lens
Products in scope: Clark Desktop, Clark cloud, Scout, Clark Security, Clark Scientist, Clark Simulator

## Product definition

Scout, Security, Scientist, and Simulator are specialist spaces, not skills,
plugins, models, or ordinary chat presets. Each space combines:

1. a durable cloud-owned domain;
2. recurring and background work;
3. domain-specific views;
4. contextual Clark conversations;
5. evidence and artifacts that outlive any one conversation.

Conversation is how a person steers and interprets a specialist. It is not the
specialist's system of record.

## Shared information architecture

The Clark Desktop sidebar keeps coding projects and general conversations.
Below them, a first-class `Specialists` section exposes:

- Scout
- Security
- Scientist
- Simulator

The specialist shell has:

- a specialist switcher;
- an organization/workspace selector;
- lifecycle and freshness status;
- a left contextual conversation pane;
- a right tabbed domain canvas;
- cloud-save, permission, and subscription state that remain visible without
  interrupting reading.

## Shared identities

Every specialist conversation carries a cloud-owned context:

```text
specialist_kind
organization_id
scope_kind
scope_id
object_kind
object_id
```

The context is nullable for general coding conversations. It is immutable after
the first specialist run starts. A conversation can be re-scoped only by
creating a new conversation and preserving a source link.

Supported contexts:

| Specialist | Scope | Optional focused object |
| --- | --- | --- |
| Scout | workspace | entity, edge, claim, journey, simulation, run |
| Security | organization or repository | finding, campaign, scan, remediation |
| Scientist | organization or research program | campaign, study, experiment, run, evidence, claim |
| Simulator | organization or target project | scenario, simulation run, counterexample, coverage family |

The server, not the WebView, binds conversations to the signed-in user and
checks current organization/object access.

## Entitlement contract

Specialist access requires current Clark subscription coverage.

Covered:

- active personal subscription;
- trialing personal subscription;
- active covered organization with a seat assigned to the current user.

Not covered:

- Free;
- promotional or free-model credits;
- purchased credits without subscription coverage;
- BYOK;
- canceled/expired subscription after its effective period;
- past-due or `action_needed` coverage;
- organization membership without a covered assigned seat;
- stale, missing, or unverifiable billing state.

Free remains a coding tier. Scout and Security navigation stays visible so the
product is understandable, but specialist data and execution are unavailable.

### Access modes

| State | Destination visible | Domain data | Existing conversations | New work | Background work |
| --- | --- | --- | --- | --- | --- |
| Signed out | Yes | No | No | No | Continues server-side if already admitted |
| Free | Yes, locked | No | Preserved, locked | No | Paused before new admission |
| Trial | Yes | Yes | Yes | Yes | Yes |
| Paid personal | Yes | Yes | Yes | Yes | Yes |
| Covered workspace seat | Yes | Membership-filtered | Yes | Yes | Yes |
| Workspace member, no seat | Yes, locked | No | Preserved, locked | No | Paused |
| Past due/action needed | Yes, locked | No | Preserved, locked | No | Paused |
| Cancel at period end | Yes until period end | Yes | Yes | Yes | Yes |
| Coverage just ended | Yes, locked | No | Preserved, locked | No | Already-running work may seal |
| Billing unavailable | Yes, checking | Cached chrome only | Do not reveal cached content | No | Server decides |

The client gate is explanatory. Server admission and data authorization are
authoritative.

## Subscription transitions

### Upgrade

1. Billing refresh observes coverage.
2. Locked specialist shell transitions in place to loading.
3. The selected destination, tab, draft, and intended action are preserved.
4. Domain data loads after authoritative coverage and membership checks.
5. A pending intent may run only after the person presses its explicit action
   again; checkout return never starts work silently.

### Downgrade

1. Billing refresh removes coverage.
2. New specialist requests stop immediately.
3. Visible sensitive domain data is replaced by the locked state.
4. Drafts and specialist conversation data remain stored in Clark.
5. Already-admitted work may finish and seal, but its result is not exposed
   until access returns.
6. Schedules remain configured but do not admit new runs.

### Organization seat changes

- Seat assignment grants access after both billing and membership refresh.
- Seat removal revokes specialist access without deleting data.
- Organization removal also revokes object access and must not be restored by a
  personal subscription.
- Personal coverage does not override missing organization/repository access.

## Slash-command and prompt handoff

`/scout`, `/security`, `/security-diff`, `/security-deep`, `/scientist`, and
`/simulate` remain accepted as legacy entry points but no longer execute as
ordinary coding skills.

The handoff:

1. Parse the specialist intent and retain all user-authored text.
2. Open the matching specialist shell.
3. Resolve organization plus workspace/repository.
4. Create or select a specialist conversation.
5. Show the proposed run mode and target.
6. Require an explicit `Start`, `Scan`, or `Run` action.

On Free, the same handoff opens the locked specialist destination with the
value proposition and preserved draft. It never inserts a hidden skill token
or starts a coding-model run.

Direct skill tokens are defense-in-depth gated in the local provider and cannot
bypass specialist coverage.

## Scout feature map

### Domain hierarchy

```text
Organization
└── Scout workspace
    ├── charter
    ├── sources and authenticated contexts
    ├── enrolled machines
    ├── discovery runs and tasks
    ├── temporal graph
    │   ├── entities
    │   ├── edges
    │   ├── claims
    │   └── coverage cells
    ├── business journeys
    ├── simulation overlays
    ├── evidence
    └── conversations
```

### Tabs

- **Map** — current or historical observed graph, search, selected-object
  inspector, explicit unknowns.
- **Changes** — accepted source batches, timeline replay, and explicit
  reconciliation events.
- **Simulations** — overlays, memberships, coverage, result and confidence.
- **Evidence** — source, machine, run, adapter, time, classification, receipt,
  and conflicts.
- **Runs** — expedition health, connected sources, machines, and sealed output.
- **Conversations** remain nested under Scout in the product sidebar rather
  than consuming a canvas tab.

### Primary journeys

- Create a workspace and charter.
- Resume an existing map at its latest accepted state.
- Start or monitor an expedition.
- Replay accepted changes across time.
- Inspect an entity, edge, claim, or missing relationship.
- Ask Scout about the selected object without losing canvas state.
- Compare observed and simulated coverage.
- Resolve or explicitly retain an unknown boundary.
- Receive a background-run completion and deep-link to the changed map.

### Scout edge cases

- No organizations.
- Multiple organizations.
- No covered seat in the selected organization.
- No Scout workspace.
- Workspace exists but has no charter.
- Charter changed while a run is active.
- No enrolled machine.
- Machine revoked or offline.
- No usable source/auth context.
- Partial, denied, unreachable, unsafe, stale, truncated, or untested coverage.
- Empty authoritative result versus incomplete enumeration.
- Graph conflict or tentative reconciliation.
- Historical replay while new changes arrive.
- Simulation points at an older graph snapshot.
- Selected object disappears at another timeline position.
- Result pagination/truncation.
- Change stream disconnect and polling fallback.
- Secret-canary or classification rejection.
- Run deferred by subscription, rate, cost, concurrency, or unavailable worker.
- User loses membership while viewing.
- Another device changes workspace, selection, or conversation.

## Security feature map

### Domain hierarchy

```text
Organization
├── repositories and access projections
├── policies and schedules
├── posture
├── scan requests and scan runs
├── findings and occurrences
├── PoC and prior-art evidence
├── campaigns and assignments
├── remediation workspaces and verification
└── conversations
```

### Tabs

- **Posture** — repository coverage, staleness, risk, policy state.
- **Findings** — validated findings with repository and severity filters.
- **Zero-day lab** — evidence-bounded novelty workflow.
- **Campaigns** — multi-repository remediation and ownership.
- **Scans** — queued, deferred, running, sealing, completed, incomplete, and
  failed histories.
- **Conversations** remain nested under Security in the product sidebar rather
  than consuming a canvas tab.

### Primary journeys

- Connect/register the current Git repository.
- Choose the correct organization when several are available.
- Start standard, diff, deep, or remediation-verification scan.
- Monitor queued/deferred/running phase progress.
- Inspect a finding and source-to-sink evidence.
- Review positive/negative PoC controls and prior-art evidence.
- Decide false positive, accepted risk, won't fix, or reopen.
- Create or join a campaign.
- Open a remediation workspace and verify the patch.
- Sync sealed local evidence to the cloud.
- Receive a completed scan/finding notification and deep-link to it.

### Security edge cases

- Current folder is not Git.
- Detached HEAD, shallow clone, dirty worktree, missing remote, or unsupported
  forge.
- Repository is not registered.
- Repository is registered in another organization.
- Multiple organizations or ambiguous repository ownership.
- GitHub inventory exists but per-user access projection is not ready.
- Repository access revoked while viewing or running.
- Selected revision changed before scan admission.
- Diff base cannot be resolved.
- Scan already exists for the immutable revision and mode.
- Request queued, automatically deferred, rate-limited, or concurrency-limited.
- Worker lease expires and work is retried with a new fence.
- Partial coverage, failed phase, missing receipt, or unsealed scan.
- Finding changes version before a decision.
- Finding disappears on complete rescan versus remains unresolved on partial
  coverage.
- PoC blocked, unsafe, timed out, or resource limited.
- Prior-art evidence is inconclusive.
- Vault grant expires or artifact access is revoked.
- Campaign owner loses access.
- Remediation patch no longer descends from the bound base revision.
- Local evidence upload is offline, duplicate, pending, or rejected.
- Organization seat, membership, or subscription changes mid-flow.

## Scientist feature map

### Domain hierarchy

```text
Organization
└── research program
    ├── authority and budgets
    ├── campaigns
    │   └── preregistered studies
    │       └── experiments
    │           ├── runs and effects
    │           ├── evidence
    │           ├── claims
    │           └── decisions
    ├── source snapshots
    ├── product-safe projections
    └── conversations
```

### Tabs

- **Programs** — objective, authority, budgets, campaign count, and supported
  claims.
- **Campaigns** — frozen objectives, studies, experiments, status, and
  unresolved gates.
- **Experiments** — hypotheses, replications, evidence, status, and decisions.
- **Evidence** — immutable observations, provenance, calibration, rights, and
  claims.
- **Runs** — live, terminal, interrupted, recovered, and failed effect
  histories.

### Primary journeys

- Turn broad intent into a bounded research program.
- Preregister a falsifiable study before launching work.
- Ask the Scientist model for one schema-valid discriminating experiment.
- Delegate implementation to headless Clark Code under an attenuated task
  contract.
- Run deterministic evaluators or instrument adapters.
- Inspect failed, rejected, and replicated experiments without losing lineage.
- Admit observations as evidence and support/refute/qualify claims.
- Pause, stop, or reauthorize interrupted work.
- Publish a versioned overview without exposing private reasoning or tools.

### Scientist edge cases

- Model returns malformed or schema-drifted output.
- Provider supports JSON but not native JSON Schema.
- Proposal tries to broaden tools, paths, commands, authority, or budget.
- Campaign starts before a study is preregistered.
- Primary metric improves while a protected metric regresses.
- Evidence has invalid provenance, calibration, time, confidence, or residency.
- Acceptance lacks required independent replications or linked supported
  claims.
- Worker crashes after dispatch-started but before a receipt.
- Two workers append at the same stream version.
- Effect requires a human gate or expired capability.
- Projection sequence is stale or conflicts at the same sequence.
- User can read the organization but is not owner/admin and cannot publish.

## Simulator feature map

### Domain hierarchy

```text
Organization
└── simulation program
    ├── pinned targets and evidence snapshots
    ├── candidate scenario bank
    ├── acquisition and coverage policy
    ├── versioned scenario IR
    ├── deterministic or external drivers
    ├── independent oracles
    ├── simulation runs
    ├── counterexamples
    ├── admitted evidence
    └── conversations
```

### Tabs

- **Scenarios** — candidate family, target, acquisition score, severity,
  coverage gap, and status.
- **Runs** — pinned target revision, seed, driver, oracle outcome, and
  terminal state.
- **Counterexamples** — minimized reproducible failure states and
  perturbations.
- **Coverage** — executed and evidential coverage by failure family and
  invariant.
- **Evidence** — accepted observations only; construction and execution are
  not automatically scientific evidence.

### Primary journeys

- Define a product, environment, training, or organizational target.
- Freeze invariants, safety constraints, drivers, oracles, and scenario budget.
- Acquire severe unknown and under-covered scenarios.
- Execute reproducible simulations through the Scientist research runtime.
- Distinguish successful driver execution from a passing product invariant.
- Minimize and retain counterexamples as future regression cases.
- Compare scenario acquisition policies under a fixed bank and budget.
- Admit validated outcomes as evidence and publish a safe overview.

### Simulator edge cases

- Candidate is severe but unrealistic, ungrounded, or out of authority.
- Target revision or source snapshot is missing or changed.
- Driver ID is unavailable or produces divergent/non-deterministic output.
- Oracle is malformed, self-referential, or not independent of construction.
- Simulation executes successfully while the product invariant fails.
- Counterexample cannot be reproduced or minimized.
- Scenario count rises while failure-family coverage remains unchanged.
- Multiple generators or drivers share a shortcut and produce false confidence.
- Simulation outcome lacks calibration and must remain outside evidence.
- Physical or external simulator cannot compensate, cancel, or emergency-stop.

## Shared UI states

Every specialist route must render intentional states for:

- loading;
- refreshing while retaining safe chrome;
- empty;
- ready;
- offline;
- stale;
- partial;
- permission denied;
- subscription required;
- billing action required;
- service unavailable;
- rate limited with retry timing;
- conflict;
- background work active;
- background work completed;
- archived/deleted object;
- unsupported older server contract.

Motion uses Clark's shared timing tokens and respects reduced motion. Pane,
specialist, tab, selected object, and entitlement transitions preserve focus and
do not animate sensitive data after access is revoked.

## Deep-link contract

Conceptual route:

```text
clark://specialist/{kind}
  ?organization={id}
  &scope={id}
  &tab={tab}
  &objectKind={kind}
  &object={id}
  &conversation={id}
```

Opening a link resolves in this order:

1. signed-in account;
2. specialist coverage;
3. organization membership;
4. object access;
5. object existence/version;
6. requested tab;
7. contextual conversation.

Failure never falls back to a different organization or similarly named object.

## Data and privacy

- Free/locked screens do not load specialist records into the WebView.
- Signing out clears specialist state and cached selections.
- Account changes cannot inherit another account's organization, repository,
  workspace, draft, or conversation selection.
- Local selection preferences are account and scope bound.
- Cloud is authoritative for specialist conversations and drafts.
- Evidence classifications and repository permissions apply independently of
  subscription coverage.
- Telemetry contains opaque ids and state transitions, never repository text,
  evidence bodies, secret values, or prompts.

## Verification matrix

Minimum automated coverage:

- entitlement truth table;
- upgrade/downgrade transition reducer;
- account and organization switching;
- slash-command handoff and bypass prevention;
- specialist conversation context serialization;
- cloud conflict and offline recovery;
- deep-link resolution failures;
- Scout loading/empty/partial/conflict/live states;
- Security repository/access/scan/finding/remediation states;
- reduced motion and keyboard focus;
- narrow and desktop layouts;
- server rejection for uncovered reads and mutations;
- in-flight completion plus new-admission pause after downgrade.

Visual QA states:

- Scout ready;
- Scout partial/offline;
- Scout Free locked;
- Security finding selected;
- Security scan running;
- Security Free locked;
- upgrade transition;
- downgrade transition;
- 1440 × 1024 desktop;
- narrow desktop and mobile companion layout.
