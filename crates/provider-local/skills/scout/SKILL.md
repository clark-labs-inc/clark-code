---
name: scout
description: Map an organization's end-to-end technical business system and derive a simulation-ready model using high-signal control planes, target-bound adapters, immutable evidence, and one authoritative enterprise graph. Use for `/scout`, system cartography, business environment surveys, pre-simulation maps, or requests to prove infrastructure and repository claims with artifacts.
---

# Scout — evidence-first system cartography

Produce an adjudicated organization-system graph, not a host inventory or
narrative survey. Every finding must name a test and evidence artifact, carry a
typed interval when quantitative, or end as `UNFALSIFIABLE` with the missing
instrument. The final graph must be usable to design an end-to-end simulation.

## Non-negotiable boundaries

- Start only from a human pressing Start/Run/Rescan in an explicitly selected
  Clark organization and its Company Scout map. Never run on navigation, specialist
  switching, app launch, a timer, a stale conversation, or an enterprise-
  context read from normal Code. Never schedule or silently resume a scan.
- The currently open folder is execution context, not Scout scope. It may be
  one local checkout discovered during the census, but it must never become
  the charter, organization, Company Scout authority, or root of the enterprise map by
  inheritance.
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
- Raw shell or SSH execution is not an isolation receipt. Call it external
  containment unless an attested OS boundary proves otherwise. WASM is for pure
  transforms and parsers, not ambient host inspection.
- the host-configured system-cartography backend is the enterprise authority.
  Local SQLite, local trust manifests, exported bundles, and materialized
  graphs are staging caches and projections only. Never present them as shared
  enterprise state or completion.
- Tenant access comes only from explicit product organization membership and
  the host-bound Company Scout map id. Never group, merge, authorize, or share Scout discoveries by
  matching email domains. Public domains such as `gmail.com` provide no
  tenancy relationship.
- Collectors observe. Only the backend issues tasks, accepts evidence, advances
  the frontier, corrects, retracts, and determines completion.

## Start with the business charter and control-plane seeds

1. Write a provisional charter naming the organization or business unit,
   products and environments in scope, production read-only policy, known
   control planes, exclusions, and the simulation question.
2. Call `scout_capabilities` only as an adapter bootstrap over declared
   execution roots. Review its truncation flags and routing states. It returns known
   DevOps/cloud executable names, environment-variable names, scoped `.env` key
   names, and credential-source kinds without values.
   Explicitly report whether `gh` is present; verify GitHub access rather than
   inferring it from installation; enumerate every repository visible through
   each authorized forge organization/account; and reconcile those remote
   identities with locally checked-out repositories under declared roots.
   Local folders without exact remote identity remain unresolved candidates,
   not enterprise scope.
   Call `scout_repository_census` with action `census` to inspect every host-approved local read
   root. It returns canonical forge identities and opaque checkout ids without
   exposing absolute paths. Treat `unapproved_filesystem_locations_not_scanned`
   as an explicit coverage gap rather than crawling the user's disk. For each
   returned checkout, call action `inspect` with only its opaque id to obtain
   bounded manifest descriptions, dependency names, workflow names, and
   component markers. Use those facts to propose what the repository does and
   how it may fit the graph; mark runtime relationships unverified until an
   independent deployment, cloud, DNS, CI, or telemetry source confirms them.
   These actions are hints only. After the backend issues and the host claims
   the `clark/local-repository@1` task, run action `collect` with no checkout
   id. It reruns the bounded census and inspections under that task and returns
   a retained receipt id. Submit that receipt through `scout_enterprise
   submit_adapter_receipt`; only the accepted graph rows are durable evidence.
3. Turn the bounded capability census into an evidence-first discovery plan.
   Never include credentials, secret values, raw private source, or shell
   commands in planning artifacts.
4. Seed a business surface manifest from authoritative entry points: source
   forges, cloud organizations/accounts/subscriptions/projects, identity
   providers, DNS and certificate control planes, CI/CD and artifact systems,
   observability, data platforms, and declared business SaaS.
5. Resolve one explicitly authorized product organization, Company Scout map
   storage id, charter, run, registered source, and enrolled machine. Backend
   ids are authoritative and must never be inferred from an email address,
   domain, mutable display name, credential value, or unverified imported
   bundle. A personal user stays in a private Scout context unless an explicit,
   audited membership or share grants otherwise. Call `scout_enterprise
   enroll`; the trusted host supplies the exact tenant binding, Platform
   credential, application-private identity root, and platform metadata.
   The tool accepts none of them from the model.
6. For this explicit human Start/Run action, call `scout_enterprise start_run`
   with the human's objective. The host supplies the stable idempotency binding
   created by the Start-run UI; the model cannot see or replace it. The backend
   atomically issues the charter, starts one run, and seeds fenced GitHub
   authority plus host-approved local-checkout tasks; it returns the only
   authoritative run id. Never invoke start_run from navigation, a timer, a
   background continuation, or a context read.
7. On every local, SSH, or VM execution target, call `scout_adapter census`.
   Treat its target identity, opaque credential candidates, and registered
   routes as target-bound: never reuse a candidate or auth handle on another
   machine. Call `verify_auth` for every candidate and exact declared authority,
   not only the default or first successful context. A candidate is not evidence
   until verification succeeds.
   For GitHub, first verify the candidate against authority `global`, claim the
   backend-seeded task, and exhaust `list_organizations`; do not ask the human
   to name a GitHub organization that the authenticated control plane can
   enumerate. Then exhaust the backend-issued `list_accessible_repositories`
   task for owner, collaborator, and organization-member visibility as well as
   the repository tasks for each discovered organization. Reconcile duplicates
   by provider-native repository identity; never equate organization membership
   with the complete repository perimeter.
8. For registered GitHub, AWS, and GCP routes, use `scout_adapter fetch_page`
   only after `scout_enterprise claim_task` returns an `adapter_page` task. For
   the backend-authored `clark/local-repository@1` route, use
   `scout_repository_census collect` instead; it produces the same retained
   adapter-receipt boundary without giving the target service ambient project
   filesystem access.
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
9. Use only the backend-issued discovery charter. It declares the exact
   required coverage cells, a maximum pass age, and pinned critical journey and
   runtime ids. The model may populate its tasks but never create, shrink,
   supersede, or issue a second charter.

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
the host. Every batch binds the backend-issued organization, Company Scout map id, run,
source, machine, task, and fence to provider-native entities, edges, claims,
coverage, or explicit retractions. Agents must not invent hashes. Host code
signs each batch with a protected target-local key. The host loads or creates
that key through `scout-machine-identity` below an application-owned private
data directory, bound to the exact host origin, organization, and Company Scout map.
Private key bytes never enter tool arguments, results, evidence, logs, or
exports. The model never chooses the tenant binding, private directory, seed,
public key, signer id, or coordinator key.

Before adjudicating an enterprise claim, finish the backend acceptance
sequence: claim a backend task lease; request a signed evidence-upload
authorization bound to the organization, Company Scout map id, run, source, machine,
task, and fence; upload the exact bytes to the server-generated S3 key; commit
and verify its SHA-256, size, content type, KMS encryption, and immutable
version id; submit the signed observation/completion batch; then verify the
backend coordinator receipt against the map-pinned public key. A stale
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
possession public key to one exact Company Scout map, and returns the backend-issued
machine id plus the map coordinator key. Repeating enrollment is
idempotent only for the same active key and platform metadata. A revoked key
cannot self-reenroll.

Concurrent collectors share nothing directly. Each claims fenced backend
tasks, uploads immutable evidence to server-generated S3 keys, and submits
signed batches to the host. Only backend-accepted batches under the same explicit
organization and Company Scout map contribute to the graph. Never exchange mutable
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
to an exact organization and Company Scout map, effective time, knowledge time, and graph
filter. Its membership rows distinguish graph coverage (`covered`, `partial`,
`outside_contract`, `unknown`) from execution result (`not_run`, `passed`,
`failed`, `diverged`, `blocked`). Never infer simulation coverage from adapter
scan coverage; they answer different questions.

Use `scout_enterprise_query changes` after a snapshot or overlay page to poll
the Company Scout map's monotonic change sequence. Resume from exactly
`next_after_sequence`; a `batch_accepted` change means the temporal graph may
have advanced, while `simulation_overlay_published` means a new immutable
overlay version is available. The host's website may consume the equivalent SSE
stream with `Last-Event-ID`. The change feed is an invalidation/replay signal,
not a substitute for fetching the pinned snapshot, delta, or overlay rows.

Lease exactly one backend-authored page task at a time. Claims are
target-affine and fenced; continuation creation and terminal gaps are backend
transitions. A collector process may restart on one target, but a provider
cursor or credential handle never moves to another target. A stale fence must
produce neither graph evidence nor a continuation. If the target vault is
lost, record `target_unavailable` and restart that coverage cell from page zero
after reauthorization.

Submit a page only through the host acceptance workflow. The accepted batch
must bind the leased task and fence to the exact target adapter receipt, page
digest, authenticated tenant, and immutable evidence version. Backend run
advancement then verifies that exact accepted receipt before issuing any
continuation or child tasks. Retrying either step must be idempotent; no local
scheduler completion or model statement may advance the frontier.

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
- declared execution roots and discovered repository/deployment roots;
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

If a CLI is missing, prefer an available typed adapter fallback:

- GitHub organizations plus all repositories visible to the authenticated user:
  `scout_adapter` uses target-native Rust HTTPS when a target token candidate
  verifies, with a fixed-argument `gh` fallback.
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

## Authoritative run lifecycle

There is exactly one run id, charter, task queue, evidence acceptance log, and
graph lifecycle: the system-cartography backend created by `start_run`. Do not
create a local run, local claim ledger, parallel phase machine, or report store.
Target-local state may retain opaque authorization handles, provider cursors,
and unsubmitted adapter receipts only long enough to complete a backend-issued
task. It never establishes enterprise truth.

For each backend task:

1. Claim it through `scout_enterprise claim_task`.
2. Execute exactly its allowlisted adapter page through `scout_adapter`.
3. Submit only the retained receipt id through `scout_enterprise
   submit_adapter_receipt`.
4. Require immutable evidence upload, backend batch acceptance, and backend run
   advancement before claiming another task.
5. Continue until the backend returns no claimable task. A denial, truncation,
   missing adapter, or unavailable target is a coverage gap, never an empty or
   completed surface.

Local repository census and inspect results are discovery hints. The collect
action is authoritative only after its retained receipt passes immutable upload
and backend acceptance under the backend-issued local-checkout task. Use the
accepted checkout and canonical-remote entities to reconcile local state with
forge repositories. Runtime relationships inferred from manifests remain gaps
until an independent deployment, cloud, DNS, CI, or telemetry source confirms
them.

### Synthesize and seal

Before a complete seal:

- require every enterprise observation used by the report to have a verified
  S3 evidence version and the host backend acceptance receipt under the exact
  organization and Company Scout map; local-only batches cannot satisfy this gate;
- Call `scout_enterprise_query status` and require enterprise completion with
  no unresolved graph, source-position, or retraction conflicts;
- require a coordinator-issued charter, a current verified pass, and an
  identical verified predecessor (`fixed_point: true`);
- every expected business control plane and recursively discovered frontier row
  has a terminal status;
- capability and coverage gaps are visible.

A partial seal must name at least one limitation or requested follow-up.

The final report order is: TL;DR with backend object refs; charter and run id;
business graph and control-plane coverage; findings by domain; simulation
model; corrections and retractions; evidence locators and acceptance receipts.

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

Retain bounded raw receipts only in target-local tool state. The final response
references backend evidence and acceptance receipts rather than copying raw
provider data or inventing a second digest or completion signal.
