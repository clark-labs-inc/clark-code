# Clark Security adversarial simulation report

Date: 2026-07-29

## Outcome

The standard, diff, deep-contract, fake-provider, history UI, negative-control,
cloud-ingest, managed snapshot/analysis/PoC/seal worker, enterprise backend,
versioned KMS vault, ClarkChat, infrastructure, and paid-model seams are
exercised. The paid semantic evaluation used Clark Platform's advertised
`qwen/qwen3.7-flash` model and did not alter the shipped Security workflow's
exact `z-ai/glm-5.2` production model policy.

## Corpus

`harness/fixtures/security-vulnerable-repo` is a deliberately non-deployable,
multi-language fake service with 14 independently exploitable files:

- broken admin authorization and two tenant-boundary IDORs;
- SQL and shell injection;
- path traversal and Zip Slip;
- SSRF;
- unsigned JWT acceptance;
- fail-open webhook verification;
- stored XSS and open redirect;
- insecure session cookies;
- a fake hardcoded credential pattern.

`harness/security-vulnerable-oracle.json` is outside the fake repository so the
model cannot discover answers by reading the oracle. Three protected controls
exercise parameterized SQL, destination allowlisting, and canonical archive
paths. Generated vendor code is explicitly excluded by the fixture policy.

## Acceptance matrix

| Journey | Boundary | Expected | Result |
| --- | --- | --- | --- |
| Standard scan | deterministic finalizer | all 14 reportable candidates seal; safe controls do not | passed |
| Missing coverage | deterministic finalizer | cannot report a partial scan as clean | passed, rejected |
| Stale inventory | deterministic finalizer | source mutation invalidates the receipt | passed, rejected |
| Missing attack path | deterministic finalizer | reportable candidate cannot seal | passed, rejected |
| Invented evidence path | deterministic finalizer | out-of-repository evidence cannot seal | passed, rejected |
| Working-tree diff | Git target contract | rename, delete, modification, and new untracked source are bound | passed |
| Stale diff | Git target contract | a later content change invalidates the target | passed, rejected |
| Deep scan | host ledger unit suite | accepted distinct passes plus two zero-novelty passes required | passed |
| Fake provider `/security` | provider and tool loop | schema, inventory, artifact write, finalize, seal, history | passed |
| Populated history | rendered browser journey | standard, diff, and deep receipts render and expand | passed |
| Empty history | rendered browser journey | actionable empty state | passed |
| Artifact error | rendered browser journey | bounded visible error, no crash | passed |
| Dismissal | rendered browser journey | Escape closes the popover | passed |
| Remote session | rendered browser journey | local artifact history is hidden | passed |
| Paid semantic scan | Clark Platform + Qwen | at least 70% recall and no safe-control false positives | passed |
| Desktop cloud ingest | native Clark client | exact scan binding, credential isolation, idempotent verified-artifact retry | passed |
| Managed cloud scan | Clark worker + Postgres | immutable GitHub target, frozen policy, fenced phases, distinct Clark signers, no human actor | passed |
| Managed PoC and seal | PoC Lab + vault + backend | paired controls, full restricted trace, exact Seal-task read, bounded ledger, signed seal | passed |
| Vault object roundtrip | LocalStack S3/KMS | KMS PUT, exact-version HEAD/GET, metadata and SHA-256 match, exact-version cleanup | passed |
| Production infrastructure | Pulumi contracts | dedicated vault/KMS, no app list/delete, exact secret injection, autoscaling and alarms | passed |
| Enterprise backend | Postgres | 10,000 repositories, 2,000 engineers, tenant isolation, concurrent leases, crash recovery | passed |
| Automatic admission | Postgres | background floods defer while interactive and incident reserves remain available; exact receipts settle integer micro-USD usage | passed |
| Admission crash and concurrency | Postgres | concurrent workers share one atomic envelope; expired reservations settle once and reclaim under a higher fence | passed |
| ClarkChat workspace | React product surface | posture, repo drilldown, zero-day lab, scans, PoC vault, immediate decisions | passed |
| Novelty confirmation | Postgres transaction + DB constraint | empty or unbound evidence fails; same-scan PoC pair, sealed prior-art artifact, two independent searches, and member confirmation succeed without a queue | passed |
| Automated prior-art phase | worker unit + PostgreSQL integration | taxonomy-only queries, fixed NVD/GHSA/OSV corpora, allowlisted bounded captures, exact GLM receipt, task/artifact binding, and no automated zero-day claim | passed |
| Prior-art outcome | React product surface | no-match path confirms novelty; relevant-match path disables confirmation and records known variant | passed |

## Paid run receipt

The live catalog returned `qwen/qwen3.7-flash` and did not advertise the stale
`clark-code:free` alias. A first pre-token request against that stale alias
failed with `model_not_found`, zero tool calls, and no usage. The harness was
then corrected to the current raw paid identifier and rerun.

The successful run reported:

- model: `qwen/qwen3.7-flash`;
- findings matched: 14/14;
- protected controls recognized: 3/3;
- protected-control false positives: 0;
- unmatched finding paths: 0;
- read-only model tool calls: 23;
- input tokens: 44,575;
- output tokens: 2,791;
- reported cost: $0.001532;
- wall time: 21.345 seconds.

The machine receipt and rendered screenshots from this run are:

- `/tmp/clark-security-simulation/receipt-paid.json`;
- `/tmp/clark-security-simulation/security-history-populated.png`;
- `/tmp/clark-security-simulation/security-history-error.png`;
- `/tmp/clark-security-novelty-audit/05-confirmation-flow-after.png`;
- `/tmp/clark-security-novelty-audit/06-known-variant-path.png`.

These `/tmp` paths are disposable evidence, not source-controlled product
artifacts.

The admission regressions are deterministic and do not spend model credits:

```bash
SECURITY_TEST_DATABASE_URL=postgres://clark_security:clark_security@127.0.0.1:55000/clark_security \
  cargo test -p clark-security --test security_platform_postgres_admission -- --test-threads=1
```

## Commands

Run every deterministic and rendered journey:

```bash
node harness/security-simulation.mjs --offline
```

Run only the explicitly authorized paid Qwen evaluation:

```bash
node harness/security-simulation.mjs --paid --live-only
```

The paid command fails preflight unless the model is exactly
`qwen/qwen3.7-flash`.
