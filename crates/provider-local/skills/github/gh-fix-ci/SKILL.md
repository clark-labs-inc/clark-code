---
name: gh-fix-ci
description: Diagnose or fix failing GitHub pull-request checks that run in GitHub Actions by tracing the first meaningful failure to current source, implementing an authorized fix, and verifying it locally.
---

# Diagnose and fix GitHub Actions failures

## Inspect the real failure

Resolve the pull request and current head SHA. Use a connected GitHub tool found through `tool_search`, or `gh` through `bash`, to list checks and inspect failing GitHub Actions jobs. If a failed check belongs to an external CI provider, report its name and details URL; do not claim to have inspected logs the host cannot access.

For GitHub Actions, identify the first meaningful error in the failing job rather than the final cancellation or cascade. Record the workflow, job, step, run attempt, commit SHA, and concise failure excerpt. Compare that evidence with the checked-out source and workflow at the same revision.

## Decide the boundary

Separate:

- product or test regressions caused by the branch
- workflow/configuration defects
- flaky or infrastructure failures
- failures already fixed on a newer head
- unrelated required checks

An inspection or diagnosis request remains read-only. A request to fix CI authorizes the smallest in-repository fix and proportionate local verification; it does not automatically authorize pushing, rerunning workflows, changing branch protections, or bypassing checks.

## Fix and verify

Fix the earliest broken contract, not a later symptom. Preserve unrelated working-tree changes. Reproduce with the narrowest relevant command, then run the repository-prescribed check for the touched surface when practical. If reproduction depends on unavailable secrets or hosted infrastructure, say exactly what was and was not verified.

Refresh the check/head state before finishing. Report the root cause, changed files, local verification, current remote status, and any external action still needed.
