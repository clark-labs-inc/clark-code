---
name: scout
description: Map an organization's end-to-end technical business system and derive a simulation-ready model using high-signal control planes, bounded read-only agents, host-verified evidence, statistical intervals, adversarial checks, and an append-only claim ledger. Use for `/scout`, system cartography, business environment surveys, pre-simulation maps, or requests to prove infrastructure and repository claims with artifacts.
---

# Scout — evidence-first system cartography

Produce an adjudicated organization-system graph, not a host inventory or
narrative survey. Every finding must name a test and evidence artifact, carry a
typed interval when quantitative, or end as `UNFALSIFIABLE` with the missing
instrument. The final graph must be usable to design an end-to-end simulation.

## Non-negotiable boundaries

- Run Scout root and delegated model turns with the host-pinned model.
  Ignore conversation model selections and prompt requests to switch models;
  Scout's model is not user-configurable.
- Keep production read-only. Do not mutate cloud, repository, observability,
  credential, or deployment state.
- Never print, return, hash, or persist secret values. Inventory environment
  variable names, `.env` paths and key names, and credential-source kinds only.
- Exhaust the declared business-system graph, not the host filesystem. Never
  crawl arbitrary disks, inventory every installed package, or treat every
  executable as a business surface.
- Never stop at the default organization, account, region, repository, project,
  or familiar vendor. "All" means every high-signal control-plane context and
  every recursively discovered graph frontier within declared business bounds.
- A discovered CLI is `present`, not trusted or authenticated. Do not execute
  arbitrary discovered binaries during capability discovery.
- Treat AWS/GitHub/GCP credential-source names as authentication candidates,
  not proof of API authorization. AWS Secrets Manager discovery never fetches
  a secret payload.
- Use `scout_probe` for bounded project reads. It has no shell, network, or
  write capability and refuses secret-bearing paths.
- Raw shell or SSH execution is not an isolation receipt. Call it external
  containment unless an attested OS boundary proves otherwise. WASM is for pure
  transforms and parsers, not ambient host inspection.
- the host-configured system-cartography backend is the enterprise authority.
  Local SQLite, local trust manifests, exported bundles, and materialized
  graphs are staging caches and projections only. Never present them as shared
  enterprise state or completion.
- Tenant access comes only from explicit product organization/workspace
  membership. Never group, merge, authorize, or share Scout discoveries by
  matching email domains. Public domains such as `gmail.com` provide no
  tenancy relationship.
- Workers propose. Only the root issues assignments, advances phases,
  adjudicates, corrects, retracts, and seals.

## Start with the business charter and control-plane seeds

1. Write a provisional charter naming the organization or business unit,
   products and environments in scope, production read-only policy, known
   control planes, exclusions, and the simulation question.
2. Call `scout_capabilities` only as an adapter bootstrap over declared
   workspaces. Review its truncation flags and routing states. It returns known
   DevOps/cloud executable names, environment-variable names, scoped `.env` key
   names, and credential-source kinds without values.
3. Turn the bounded capability census into an evidence-first discovery plan.
   Never include credentials, secret values, raw private source, or shell
   commands in planning artifacts.
4. Seed a business surface manifest from authoritative entry points: source
   forges, cloud organizations/accounts/subscriptions/projects, identity
   providers, DNS and certificate control planes, CI/CD and artifact systems,
   observability, data platforms, and declared business SaaS.
5. Resolve one explicitly authorized product organization, system-cartography
   workspace, charter, run, registered source, and enrolled machine. Backend
   ids are authoritative and must never be inferred from an email address,
   domain, mutable display name, credential value, or unverified imported
   bundle. A personal user stays in a private workspace unless an explicit,
   audited membership or share grants otherwise. Call `scout_enterprise
   enroll`; the trusted host supplies the exact tenant binding, Platform
   credential, application-private identity root, and platform metadata.
   The tool accepts none of them from the model.
6. Start `scout_ledger` with the returned census id. The host pins its
   fingerprint into the charter and rejects invented or stale ids.
7. On every local, SSH, or VM execution target, call `scout_adapter census`.
   Treat its target identity, opaque credential candidates, and registered
   routes as target-bound: never reuse a candidate or auth handle on another
   machine. Call `verify_auth` for every candidate and exact declared authority,
   not only the default or first successful context. A candidate is not evidence
   until verification succeeds.
8. For registered GitHub, AWS, and GCP routes, use `scout_adapter fetch_page`
   only after `scout_enterprise claim_task` returns an `adapter_page` task.
   Copy its backend-authored adapter, coverage, query, page, cursor-handle, and
   limit scope exactly; add only the target-bound verified authorization
   context. It executes only allowlisted read operations and returns a
   target-bound normalized receipt retained by the host. Immediately call
   `scout_enterprise submit_adapter_receipt` with only that retained `task_id`
   and `receipt_id`. The host recovers the stored run, source, fence, and
   source-sequence allocation; uploads retry-stable immutable evidence;
   translates schema-v2 observations; signs the batch; and verifies the host's
   coordinator receipt. Local append and sweep operations are retired. Do not
   advance to another page unless submission returns both an S3 version id and
   backend batch receipt. Follow backend-issued continuation tasks within
   explicit page/record/time bounds. Never request or expose the provider
   cursor.
   GCP CLI keyset cursors do not prove snapshot isolation, so retain that
   limitation. Record missing adapters and denied, unreachable, unsupported,
   unsafe, stale, or truncated authorities as first-class coverage gaps.
   Treat query authority as an authorization boundary, not an entity id.
   Normalized entities use a stable provider namespace, canonical identity
   authority, and provider-native id; versioned adapter builds remain
   provenance. Do not merge or fork entities merely because an adapter version,
   credential, traversal path, or worker changed.
9. Append one root/coordinator-issued discovery charter. It declares the exact
   required coverage cells, a maximum pass age, and pinned critical journey and
   runtime ids. Workers may populate the charter but never shrink, supersede,
   or issue it.

## Organization graph contract

Build typed entities for products, teams/owners, repositories, artifacts,
services/jobs, environments, cloud tenants/accounts/regions/resources,
deployments, domains/endpoints, identities/roles, data stores/streams,
pipelines, monitors/alerts/runbooks, clients, external vendors, and business
actors. Build typed edges such as `owns`, `builds`, `deploys_to`, `exposes`,
`calls`, `reads`, `writes`, `authenticates_via`, `configured_by`,
`monitored_by`, `alerts_to`, and `depends_on`.

Use a recursive frontier:

1. Enumerate every context and object in each seed control plane.
2. Extract references to other graph entities from structured metadata, IaC,
   deployment manifests, workflow definitions, runtime configuration schemas,
   DNS/TLS, telemetry, and dependency declarations.
3. Normalize identities and enqueue every new reference.
4. Reconcile the same entity across independent sources.
5. Stop only when every frontier row is terminal and a full pass adds no new
   entities or edges, or when a declared bound creates an explicit gap.

Cloud hierarchy is a frontier, not a label. Enumerate AWS organization/account
and region scopes and recursively enumerate GCP organizations, folders,
folder-nested projects, and project/organization Cloud Asset scopes. Persist
provider-backed ownership and membership edges. Never infer a cross-account or
cross-project authorization path from hierarchy alone; verify that exact
target/auth context or record `authorization_required`.

Stage discovery as immutable schema-v2 observation batches submitted only to
the host. Every batch binds the backend-issued organization, workspace, run,
source, machine, task, and fence to provider-native entities, edges, claims,
coverage, or explicit retractions. Agents must not invent hashes. Host code
signs each batch with a protected target-local key. The host loads or creates
that key through `scout-machine-identity` below an application-owned private
data directory, bound to the exact host origin, organization, and workspace.
Private key bytes never enter tool arguments, results, evidence, logs, or
exports. The model never chooses the tenant binding, private directory, seed,
public key, signer id, or coordinator key.

Before adjudicating an enterprise claim, finish the backend acceptance
sequence: claim a backend task lease; request a signed evidence-upload
authorization bound to the organization, workspace, run, source, machine,
task, and fence; upload the exact bytes to the server-generated S3 key; commit
and verify its SHA-256, size, content type, KMS encryption, and immutable
version id; submit the signed observation/completion batch; then verify the
backend coordinator receipt against the workspace-pinned public key. A stale
fence, revoked machine, expired upload, unverified object, tenant mismatch, or
missing receipt leaves the row unaccepted regardless of local state.

For every charter coverage cell, terminal frontier page facts must carry the
exact discovered entity and edge ids. The final coverage fact records matching
entity and edge counts. Raw provider pagination tokens remain in target-local
vault state; replicated facts may contain only a host-issued
`cursor:<uuid>` handle. If an exhaustive sweep reaches a host bound before
cursor exhaustion, retain its signed nonterminal frontier as an explicit gap
and resume it; never reinterpret the bound as empty or complete coverage.

Before a new machine writes, call `scout_enterprise enroll`. The host requires
active organization-administrator authority, binds the host's proof-of-
possession public key to one exact workspace, and returns the backend-issued
machine id plus the workspace coordinator key. Repeating enrollment is
idempotent only for the same active key and platform metadata. A revoked key
cannot self-reenroll.

Concurrent collectors share nothing directly. Each claims fenced backend
tasks, uploads immutable evidence to server-generated S3 keys, and submits
signed batches to the host. Only backend-accepted batches under the same explicit
organization/workspace contribute to the graph. Never exchange mutable
materialized graphs, trust roots, private signing state, local SQLite files, or
email-domain-derived tenant hints.

Use `scout_enterprise_query snapshot` for bounded entity, edge, claim, and
coverage pages. Supply effective and knowledge times when reconstructing
history, and pass the exact returned cursor to continue a pinned view. Follow
`next_cursor` until absent. A possibly truncated page is a prompt to continue,
not proof of completeness. Use `scout_enterprise_query delta` to compare two
independently pinned bitemporal cuts. Change effective time to measure business
system evolution; change knowledge time to measure how concurrent Scout runs
expanded or corrected the host's understanding. Treat `added`, `changed`, and
`removed` as temporal graph facts; request `include_unchanged` only when a
simulation overlay needs an explicit denominator. Pass only the returned
`delta_cursor` when continuing that exact comparison. If the host
ingestion or retrieval is unavailable, the enterprise run cannot seal; report
the backend path as a missing instrument.

Use `scout_enterprise_query simulation_overlay` to retrieve a versioned
simulation boundary. Every overlay is immutable, content-addressed, and pinned
to an exact organization/workspace, effective time, knowledge time, and graph
filter. Its membership rows distinguish graph coverage (`covered`, `partial`,
`outside_contract`, `unknown`) from execution result (`not_run`, `passed`,
`failed`, `diverged`, `blocked`). Never infer simulation coverage from adapter
scan coverage; they answer different questions.

Use `scout_enterprise_query changes` after a snapshot or overlay page to poll
the workspace's monotonic change sequence. Resume from exactly
`next_after_sequence`; a `batch_accepted` change means the temporal graph may
have advanced, while `simulation_overlay_published` means a new immutable
overlay version is available. The host's website may consume the equivalent SSE
stream with `Last-Event-ID`. The change feed is an invalidation/replay signal,
not a substitute for fetching the pinned snapshot, delta, or overlay rows.

When a central scheduler is available, lease exactly one manifest-owned page
task at a time. Claims are target-affine and fenced; heartbeats, retry/backoff,
quota consumption, continuation creation, and terminal gaps are coordinator
transitions. A worker may move between processes on one target, but a provider
cursor or credential handle never moves to another target. A stale fence must
produce neither graph evidence nor a continuation. If the target vault is
lost, record `target_unavailable` and restart that coverage cell from page zero
after reauthorization.

Commit a terminal page only through the coordinator's atomic page boundary.
It must bind the leased task and fence to the exact target adapter receipt,
page digest, signed enterprise batch, authenticated tenant, and central ingest
receipt in one transaction. Never acknowledge the batch and advance the
frontier in separate best-effort writes. Retry completions may update
scheduler backoff without graph evidence; success, empty, and terminal gaps
may not use an unlinked scheduler-only completion.

The target-side SQLite index is a disposable bounded-retrieval projection, not
evidence. Warm reads report an index receipt and must read zero batch bodies;
an event-root or filter change invalidates the cursor. Index corruption may be
quarantined and rebuilt, but a corrupt or unauthenticated immutable batch must
stop the run.

Discovery epochs are coordinator-issued adapter-authority snapshots, not
worker-chosen counters or wall-clock guesses. After every declared cell is
terminal, the host backend recomputes the requirement, scope-membership,
semantic-topology, and pass roots and refuses incomplete or forged seals.
Charters, pass seals, enrollment changes, and retractions are backend
administration actions. Completion requires two consecutive verified passes
for the same charter with identical roots.

A later verified authoritative membership snapshot may retire an entity or
edge absent from that scope while retaining its immutable history. Denied,
truncated, stale, corrupt, or partial scans never imply deletion and block a
new pass seal. Explicit evidence-backed retractions correct individual bad
observations. Same-transition disagreement and reused source positions are
conflicts that block completion.

For every row record `present`, `configured`, `authenticated`, `supported`,
`denied`, `unreachable`, `empty`, `stale`, or `untested`, plus its safe probe,
evidence, bound, and limitation. Paginate to completion within declared
account, region, project, page, time, rate, and cost limits. A hit limit,
permission denial, login requirement, unsupported adapter, stale source, or
unsafe endpoint is a visible coverage gap.

## Host and file discovery boundary

Host inspection exists to locate adapters, checked-out repositories, deployment
roots, and configuration references—not to describe the business. Inspect only:

- common DevOps/cloud/identity/observability/database CLIs and their
  credential-source kinds;
- declared workspaces and discovered repository/deployment roots;
- conventional configuration locations required by an active adapter;
- `.env` and configuration schemas inside mapped roots, returning names and
  references only.

Represent other PATH entries with a count and digest. Do not enumerate arbitrary
system files, package databases, services, listeners, user documents, or
unrelated home directories unless a discovered business-system edge and the
charter require that exact surface.

For SSH/VM qualification, the host may run the portable
`scout_capability_census` binary from the independent
`scout-capability-census` crate with one or more explicit mapped `--root`
values. Do not make sensor deployment depend on `provider-local`, a local
Scout store, or a host checkout. The binary must return only the curated
executable matrix, relevant environment names, credential-surface names,
dotenv paths/key names, coverage, truncation, redaction, Rust-fallback gaps,
and a semantic digest. It never executes a discovered program or follows a
symlink root/entry. Increase an explicit bound or schedule another scoped root
when a truncation flag is true; never call a truncated receipt exhaustive.
Use `harness/scout-census-utm-qualify.mjs` for UTM transfer, execution,
receipt-integrity, redaction, endpoint-security, and scratch-cleanup evidence.

Do not retrieve secret payloads, key material, token text, or credential values.
Do not inspect other processes' environments, switch active accounts, refresh
interactive login, install tools, start services, call paid models, or mutate a
target merely to improve coverage. Require separate authorization for those
actions.

If a CLI is missing, prefer an available typed Rust fallback:

- JSON parsing/counts and source receipts: `scout_probe`.
- Binomial Wilson intervals and seeded bootstrap mean/median intervals:
  `scout_measure`.
- GitHub organization repositories: `scout_adapter` uses target-native Rust
  HTTPS when a target token candidate verifies, with a fixed-argument `gh`
  fallback.
- AWS Organizations accounts and Resource Explorer resources:
  `scout_adapter` uses fixed allowlisted AWS CLI operations after target
  authorization verifies. A pure-Rust AWS API fallback without the CLI remains
  a missing instrument.
- GCP organizations, recursive folders, direct-child projects, and Cloud Asset
  resources: `scout_adapter` uses fixed allowlisted `gcloud` operations after
  active target authorization verifies. Folder discovery creates new
  folder/project frontier work; it is not permission to reuse an organization
  credential blindly. Broader GCP surfaces and a native Google API/OAuth
  fallback remain missing instruments.
- GitLab.com group projects: `scout_adapter` uses target-native bounded HTTPS
  after the exact group and target token verify. It follows only host-owned
  opaque page handles. Self-managed origins and broader group/CI/runner/registry
  surfaces remain missing instruments.
- Untrusted candidate-record normalization may use the versioned
  `scout-capsule-core` pure transform. The qualifying `scout-capsule-host`
  admits only administrator-approved module digests, rejects every import,
  creates fresh instances, and enforces memory/table/instance/fuel/deadline/
  concurrency and input/output bounds. Its deadline returns to the caller but
  is not a hard Wasmi interrupt; the timed-out worker retains its slot until
  finite fuel stops it. `scout_capsule` may call only the authenticated
  target-native registry configured by the host: first run an exact
  `scout_adapter` census to pin target identity, then census the requested
  enterprise and select only a returned logical capsule id. Never ask the
  model to supply or approve module bytes, paths, digests, signers, tenants,
  target identities, or limits; if the administrator registry/policy is
  absent, report the capsule instrument as unavailable.
- Generic shell replacement is not a safe fallback. Report the missing
  capability or instrument.

## Ledger phases

Advance serially: `charter → map → measure → check → prove → adjudicate →
synthesize → sealed`.

### Map

Issue bounded `mapper` assignments with exact snapshot and scopes. When there
are at least two genuinely independent surfaces, call `delegate_read_only`
once with those workstreams. Agents are read-only sensors and must return:

- candidate claim rows;
- exact artifact locators or Scout replay recipes;
- coverage;
- limitations;
- requested follow-ups.

Call `resolve_delegation` for every report. Do not accept prose-only findings.
Translate accepted rows into `scout_ledger submit_worker` envelopes. Every
envelope must carry the required submit_worker fields `assignment_id`, `role`,
`snapshot_id`, and top-level `coverage` (plus any artifacts/claims/addenda);
see the scout_ledger tool schema for the exact shapes. Candidate worker
artifacts remain untrusted.

Translate root-verified graph observations into backend-fenced schema-v2
batches. A per-run claim ledger establishes who may assert and adjudicate; the
The host enterprise graph establishes durable system state across runs and
machines. Neither can substitute for the other, and a local ledger is never
shared authority.

Use `scout_probe record` for root-observed source slices, text counts, and JSON
array counts. Use `scout_probe verify` to replay a worker's Scout-owned recipe.
A worker-supplied hash or proof tier never verifies itself.

### Measure

First record the bounded JSON source with `scout_probe`. Then call
`scout_measure` with that verified evidence id, the same project-relative path
and scope, a JSON pointer to the observation array, and explicit confidence.
Use `wilson_proportion` for boolean or 0/1/null observations. Use
`bootstrap_mean` or `bootstrap_median` for numeric/null observations, with an
explicit seed and bounded resample count.

The Rust tool re-reads the array and computes missingness, the estimate, and
the interval. Never pass model-counted successes, trials, raw observations,
estimates, or intervals into the ledger.

Report `n`, missingness, method version, and interval. Do not call an
underpowered result a null. Name construct-validity gaps and Goodhart risks.

### Check

Re-run headline recipes. Use `scout_probe reproduce` for an independently
obtained reproduction artifact. Exact/equivalent checks must come from a
host-owned runner; changed and failed replays revoke trust.

For each worker, root-replay at least one load-bearing artifact. Disagreement
goes to a new `red_team` or `reproducer` assignment, never averaging.

### Prove

Do not claim above the highest verified tier:

- T1: source trace at the pinned snapshot.
- T2: live-state confirmation through an authorized read-only adapter.
- T3: offline PoC with typed, passing positive and negative controls.
- T4: benign staging-only reachability.

Counterexample labels alone do not grant T3. No production payloads.

### Adjudicate

Adjudication is serial and root-owned:

- `SUPPORTED`: name the test, attach verified evidence, and stay within tier.
- `UNSUPPORTED`: name the failed test. Quantitative nulls require adequate
  power.
- `UNFALSIFIABLE`: name the instrument that would change the verdict.

Address every counterevidence artifact explicitly. Corrections, retractions,
and supersessions append reasons; never erase prior rows.

### Synthesize and seal

Before a complete seal:

- require every enterprise observation used by the report to have a verified
  S3 evidence version and the host backend acceptance receipt under the exact
  organization/workspace; local-only batches cannot satisfy this gate;
- Call `scout_enterprise_query status` and require enterprise completion with
  no unresolved graph, source-position, or retraction conflicts;
- require a coordinator-issued charter, a current verified pass, and an
  identical verified predecessor (`fixed_point: true`);
- issue a coordinator checkpoint after the final merge and require it to cover
  the current ledger with zero uncheckpointed batches and events; distribute it
  to every participating replica with checkpoint export/observation;
- every expected business control plane and recursively discovered frontier row
  has a terminal status;
- every headline claim is adjudicated;
- every supported headline has independently checked reproduction;
- all quantitative findings have typed uncertainty;
- counterevidence is addressed;
- capability and coverage gaps are visible.

A partial seal must name at least one limitation or requested follow-up.

The final report order is: TL;DR with claim refs; charter and census
fingerprint; business graph and control-plane coverage; findings by domain;
simulation model; corrections and retractions; claim ledger; evidence locators,
digests, and replay recipes.

The simulation model must name business actors, entry points, workflows/events,
state stores, dependencies, trust boundaries, invariants, failure modes,
external effects, synthetic fixtures, mock/real boundaries, and observability
assertions. A production service without source/artifact provenance, deployment,
identity, dependencies, ownership, and observability is a simulation-readiness
gap, not a completed node.

Enterprise completion is always complete relative to the current charter, not
proof that an unknown control plane does not exist. Report the charter revision
and required-cell root with the result.

The host hash may be used only as an optional, versioned semantic candidate index
for entity reconciliation or retrieval. It is not an entity id, evidence
digest, graph digest, completeness signal, durable store, or conflict resolver.
Provider-native identity and the canonical event graph remain authoritative.

Retain bounded raw receipts in tool results when safe; for secret-bearing or
minimized inputs, retain the source locator and input digest instead of copying
raw data. Include the ledger fingerprint so replay can detect drift.
