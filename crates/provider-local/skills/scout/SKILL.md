---
name: scout
description: Exhaustively map a technical system with a secret-safe capability census, bounded read-only agents, host-verified evidence, statistical intervals, adversarial checks, and an append-only claim ledger. Use for `/scout`, system cartography, environment surveys, pre-simulation maps, or requests to prove infrastructure and repository claims with artifacts.
---

# Scout — evidence-first system cartography

Produce an adjudicated map, not a narrative survey. Every finding must name a
test and evidence artifact, carry a typed interval when quantitative, or end as
`UNFALSIFIABLE` with the missing instrument.

## Non-negotiable boundaries

- Keep production read-only. Do not mutate cloud, repository, observability,
  credential, or deployment state.
- Never print, return, hash, or persist secret values. Inventory environment
  variable names, `.env` paths and key names, and credential-source kinds only.
- Exhaust every discovered capability family and safe authentication context;
  never stop at the default profile, first successful CLI, or familiar tools.
  "All" means all entries in the pinned surface manifest within declared
  bounds, with denials, omissions, and truncation recorded as findings.
- A discovered CLI is `present`, not trusted or authenticated. Do not execute
  arbitrary discovered binaries during capability discovery.
- Treat AWS/GitHub credential-source names as authentication candidates, not
  proof of API authorization. AWS Secrets Manager discovery never fetches a
  secret payload.
- Use `scout_probe` for bounded project reads. It has no shell, network, or
  write capability and refuses secret-bearing paths.
- Raw shell or SSH execution is not an isolation receipt. Call it external
  containment unless an attested OS boundary proves otherwise. WASM is for pure
  transforms and parsers, not ambient host inspection.
- Workers propose. Only the root issues assignments, advances phases,
  adjudicates, corrects, retracts, and seals.

## Start with capability discovery

1. Call `scout_capabilities` over the intended scope.
2. Review its truncation flags and routing states. It returns executable names,
   environment-variable names, `.env` key names, and known credential surfaces
   without values.
3. Derive and pin a surface manifest from the entire census. Include every
   detected capability family, tool, credential source, account/profile/host
   context, and declared project or infrastructure surface. Give credential
   and authentication contexts opaque ids. Include non-secret resource
   identifiers only when the declared scope and output classification permit
   them; otherwise retain counts and digests.
4. Start `scout_ledger` with the returned census id. The host pins its
   fingerprint into the charter and rejects invented or stale ids.
5. Declare objective, snapshot, scopes, exclusions, network policy, production
   read-only policy, denied capabilities, and the minimum quantitative power.

## Exhaustive sweep contract

Partition the pinned manifest into independent workstreams and inspect every
entry with the safest typed read-only operation available. At minimum classify
detected surfaces for source control and forges, cloud providers, containers
and orchestration, virtualization and sandboxes, databases, networking,
observability, infrastructure-as-code, build/package systems, language
toolchains, browsers/mobile tooling, local model tooling, operating-system
services, shells, SSH, environment names, `.env` schemas, and other executables
that do not fit a known family.

For each manifest entry record `present`, `configured`, `authenticated`,
`supported`, `denied`, `unreachable`, `empty`, or `untested`, plus the exact
safe probe, bound, and limitation. Every pinned manifest row has a terminal status.
Enumerate all discoverable profiles, accounts, hosts, regions,
projects, organizations, repositories, clusters, contexts, registries, and
analogous namespaces when the adapter can return metadata safely. Keep
authentication-context identities opaque. Paginate to completion within
declared item, page, region, context, time, and cost limits. A hit limit,
permission denial, login requirement, unsupported adapter, or unsafe endpoint
is a visible coverage gap, never implicit success.

Do not read secret payloads, key material, token text, credential values,
process environments belonging to other processes, or arbitrary configuration
contents. Do not switch active accounts, refresh interactive login, install
tools, start services, call paid models, or mutate a target merely to improve
coverage. Require separate authorization for those actions.

If a CLI is missing, prefer an available typed Rust fallback:

- JSON parsing/counts and source receipts: `scout_probe`.
- Binomial Wilson intervals and seeded bootstrap mean/median intervals:
  `scout_measure`.
- GitHub REST or AWS SDK adapters: use only when a separately installed,
  explicitly authorized network tool exists. The census describes these as
  design-ready, not enabled.
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
Translate accepted rows into `scout_ledger submit_worker` envelopes. Candidate
worker artifacts remain untrusted.

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

- every headline claim is adjudicated;
- every supported headline has independently checked reproduction;
- all quantitative findings have typed uncertainty;
- counterevidence is addressed;
- capability and coverage gaps are visible.

A partial seal must name at least one limitation or requested follow-up.

The final report order is: TL;DR with claim refs; charter and census
fingerprint; sources swept and exclusions; findings by domain; corrections and
retractions; claim ledger; evidence locators, digests, and replay recipes.
Retain bounded raw receipts in tool results when safe; for secret-bearing or
minimized inputs, retain the source locator and input digest instead of copying
raw data. Include the ledger fingerprint so replay can detect drift.
