---
name: security-diff
description: Review an exact Git range or working-tree patch for security regressions with complete changed-file coverage, supporting-code receipts, validation, attack paths, and a sealed result.
---

# Clark Security diff scan

This is a source-read-only security assessment unless the user separately and
explicitly asks to fix a finding. Do not change the checkout, stage files,
install dependencies, create tickets, publish results, or run live provider
tests. Writing the canonical scan bundle under `.clark/security-scans/` is
permitted. `security_poc_execute` may write and execute controls only in its
automatically provisioned disposable offline copy.

Clark pins the root Security turn to the exact `z-ai/glm-5.2` production model.
The workflow model is not chosen from the conversation model setting.

## Bind the exact target

Start with `security_scan_contract(action="schema")`, then choose exactly one
target:

- Current checkout: call
  `security_scan_contract(action="diff_inventory", scope=".", diff_kind="working_tree", base="HEAD")`.
- Git range: call
  `security_scan_contract(action="diff_inventory", scope=".", diff_kind="range", base="<base>", head="<head>")`.

Use a narrower scope only when the user explicitly requested one. Page until
`nextCursor` is null. Preserve the returned `inventoryId` and complete
`diffTarget` object. For a working-tree scan, the returned `resolvedHead` is a
throwaway Git tree containing staged, unstaged, untracked, renamed, and deleted
paths without modifying the real index. Review patches against the returned
`resolvedBase` and `resolvedHead`; do not silently switch to a moving symbolic
revision.

Never claim a clean or completed review unless finalization succeeds. Write the
canonical bundle to `.clark/security-scans/<scan-id>/scan.json`, then call
`security_scan_contract(action="finalize", path="<canonical bundle>")`.
A changed target, missing changed file, unrelated candidate, incomplete
evidence, or reportable finding without an attack path must remain a visible
failure.

## Coverage and policy

- Read applicable `SECURITY.md` files from repository root toward each changed
  file. The nearest policy governs.
- Put every returned changed path in `coverage` exactly once as `reviewed` or
  `excluded`. A deletion is still a changed path and must be reviewed in the
  patch or excluded with a concrete reason.
- Put unchanged files actually inspected to understand reachability or controls
  in `supportingCoverage`. Do not duplicate changed paths there.
- Do not rank changed files away. Generated, vendored, binary, and data-only
  changes may be excluded with a reason; apparent harmlessness is not a reason.
- Every candidate must touch at least one changed or previous path. Findings
  unrelated to the target belong in a separate standard scan.

## Review method

Proceed monotonically through preflight, threat model, discovery, validation,
attack path, and reporting. Build a change-specific threat model: which assets,
trust boundaries, attacker inputs, and invariants the patch can affect.

For every changed file, inspect the exact patch and enough surrounding code to
trace:

`attacker-controlled source → changed or removed control → sink → impact`

Every candidate needs a scan-local `candidateId` plus Clark Security semantic
identity: a stable lowercase vulnerability-family `ruleId`, a stable lowercase
root-control `identityAnchor`, and an optional lowercase `identityInstance` for
independently attackable siblings. Also provide `title`, `summary`, `category`,
calibrated `confidence`, optional CWE ids, and concrete `remediation`. Keep
scan ids, display numbers, file names, and line numbers out of semantic
identity, and never duplicate a semantic identity within the bundle.

Pay special attention to added entrypoints, weakened authorization, tenant
scope changes, parser and deserializer changes, filesystem/process/network
calls, credential handling, sandbox and plugin boundaries, unsafe defaults,
error-path changes, and deleted checks. A changed dependency is not itself a
finding without reachable repository impact.

Close every candidate as exactly one of `reportable`, `suppressed`,
`not_applicable`, or `deferred`. Use the strongest safe evidence available and
record counterevidence. Every reportable candidate requires a realistic
attacker, reachable entrypoint, preconditions, concrete path, likelihood
rationale, calibrated severity, and impact.

Attempt a PoC for every candidate. When safe and feasible, run both a positive
and distinct negative control with `security_poc_execute` and preserve its
host-issued receipt ids in the bundle. Outcomes `reproduced`,
`partially_reproduced`, and `not_reproduced` require both controls to pass.
`blocked` and `unsafe_to_execute` require concrete limitations and disposition
`deferred`. A reportable diff finding must be reproduced or partially
reproduced; static reasoning alone cannot mint a receipt. Challenge surprising
patch behavior as a potential novel vulnerability, but do not call it a
zero-day until independent novelty review confirms that claim.

## Reporting

Only sealed findings are findings. Report the exact resolved base/head and diff
target id, changed reviewed/excluded counts, supporting file count, deferred
proof gaps, limitations, and each finding's evidence, PoC receipts, and attack
path. If the seal has no findings, say "no reportable findings were validated
in this exact diff"; do not claim the patch or repository is secure.
