---
name: security-deep
description: Run a bounded exhaustive repository security scan with independent GLM 5.2 discovery passes, parent-verified evidence, semantic candidate reduction, saturation, complete coverage, and a sealed result.
---

# Clark Security deep scan

This is a source-read-only assessment unless the user separately and explicitly
asks to fix a finding. Do not edit the checkout, install dependencies, create
tickets, publish results, or run live provider tests. Writing the canonical
bundle under `.clark/security-scans/` is permitted. `security_poc_execute` may
write and execute controls only in its automatically provisioned disposable
offline copy.

This skill explicitly authorizes bounded read-only delegation for independent
security discovery and verification passes. Do not use coding delegates, do not
recurse, and do not widen repository scope or permissions. Clark pins the root
and delegated turns to the exact `z-ai/glm-5.2` production model.

## Start and inventory

1. Call `security_scan_contract(action="schema")`.
2. Choose a unique scan id and call
   `security_scan_contract(action="deep_begin", scan_id="<id>", scope=".")`.
3. Preserve the returned `deep.runId` and `inventoryId`.
4. Call `security_scan_contract(action="inventory", scope=".")`, paging until
   `nextCursor` is null.

Use a narrower scope only when explicitly requested. Record every inventoried
path exactly once in final `coverage` as `reviewed` or `excluded`; exclusions
need concrete reasons. Read applicable `SECURITY.md` files from repository root
toward each file.

## Independent-pass loop

Run one `delegate_read_only` orchestration at a time. Each orchestration is one
independent pass and needs a distinct, non-empty focus. Use purpose `explore`,
`review`, or `verify`, bounded scopes, and explicit acceptance criteria. Useful
focuses include:

- entrypoints, assets, boundaries, and attacker-input census;
- authorization, tenant isolation, and sensitive state changes;
- parsers, deserialization, injection, filesystem, process, and network sinks;
- sandbox, plugin, update, credential, and protocol boundaries;
- adversarial challenge of accumulated candidates and missing controls.

Workers report source/control/sink evidence; they do not decide the canonical
candidate ledger. Inspect every cited repository file yourself before accepting
a report. Call `resolve_delegation` for every reported task, accepting only
sound evidence or requesting bounded rework.

After an orchestration is fully accepted, reduce its evidence into stable
candidate ids and call:

`security_scan_contract(action="deep_checkpoint", deep_run_id="<run>", orchestration_id="<accepted orchestration>", candidate_ids=[...])`

`candidate_ids` are the candidates observed in that pass, not only novel or
reportable ones. Reuse the same id when independent passes describe the same
source/control/sink/impact tuple. Keep separate ids for independently
attackable tuples.

Continue with new independent focuses until `deep.saturated` is true. The host
requires at least three accepted passes, distinct focus text, a checkpoint for
every pass, and two consecutive passes that add no new candidate ids. A failed,
unresolved, fabricated, or uncheckpointed delegation is not a pass.

## Validation and reporting

Proceed monotonically through threat model, discovery, validation, attack path,
and reporting. Trace each candidate:

`attacker-controlled source → nearest control → sink or broken control → impact`

Close every accumulated candidate as `reportable`, `suppressed`,
`not_applicable`, or `deferred`, with evidence and counterevidence. Every
reportable candidate requires a realistic attacker, reachable entrypoint,
preconditions, concrete reachability path, likelihood rationale, calibrated
severity, and impact.

Every candidate needs a scan-local `candidateId` plus Clark Security semantic
identity: a stable lowercase vulnerability-family `ruleId`, a stable lowercase
root-control `identityAnchor`, and an optional lowercase `identityInstance` for
independently attackable siblings. Also provide `title`, `summary`, `category`,
calibrated `confidence`, optional CWE ids, and concrete `remediation`. Keep
scan ids, display numbers, file names, and line numbers out of semantic
identity, and never duplicate a semantic identity within the bundle.

Attempt an offline PoC for every accumulated candidate after reduction. When
safe and feasible, run a positive exploit control and a distinct safe negative
control through `security_poc_execute`; only its host-issued ids count as
receipts. Outcomes `reproduced`, `partially_reproduced`, and `not_reproduced`
require both passing receipts. `blocked` or `unsafe_to_execute` require concrete
limitations and disposition `deferred`. A reportable finding must be reproduced
or partially reproduced, not merely argued from source. Use independent passes
to seek unexpected boundary compositions that may be novel, while reserving
the labels zero-day and novel for later independent novelty review.

The final bundle uses `mode: "deep"` and copies the host-issued `deepRunId`.
Its candidate ids must exactly equal the union checkpointed across passes.
Write it to `.clark/security-scans/<scan-id>/scan.json`, then call
`security_scan_contract(action="finalize", path="<canonical bundle>")`.

Never claim a clean or completed scan unless finalization succeeds. Report the
sealed pass count, reviewed/excluded counts, findings, PoC receipt ids, deferred
proof gaps, and limitations. With no findings, say "no reportable findings were
validated after the sealed deep passes"; do not claim the repository is secure.
