# Scout offline benchmark

This benchmark exercises Scout's deterministic trust contract without loading
`.env`, calling a model, contacting a network service, or mutating a target
system.

```bash
cargo run -p provider-local --example scout_benchmark -- \
  --out target/scout-benchmark/local \
  --host-label local \
  --containment external
```

The default portable fixture remains 1,200 services across eight machines.
Larger deterministic runs use the same contracts and can be requested without
changing source:

```bash
cargo run --release -p provider-local --example scout_benchmark -- \
  --out target/scout-benchmark/enterprise-25000 \
  --host-label local \
  --containment external \
  --enterprise-services 25000 \
  --enterprise-machines 8
```

It writes `receipt.json` and `report.md` into a new output directory and refuses
to overwrite an existing run. The canonical SHA-256 covers ordered case ids,
pass/fail states, and any case-provided deterministic `semantic_sha256`; it
excludes timings and host metadata so equivalent runs on different operating
systems can be compared directly.

The cases exercise capability-safe skill resolution, a synthetic
business-system graph and simulation-readiness contract, fixed-point discovery
and negative controls, append-only ledger replay, authority and
self-certification rejection, replay-recipe and proof-tier requirements,
Wilson reference values, the same seeded-bootstrap kernel used by
`scout_measure`, and a 1,200-service/eight-machine enterprise graph that must
converge under reversed and duplicate authenticated batch delivery. The
enterprise case uses a pinned Ed25519 trust root, exact short-lived coordinator
or collector grants, signed batch envelopes, a coordinator charter, exact
per-scope entity/edge membership, a critical actor-to-journey-to-state-effect
path, and two verified identical passes; a single pass is a negative control
and cannot claim a fixed point. It also rebuilds the target-side SQLite
projection, then proves a warm status and bounded entity page read zero
immutable batch bodies while preserving the canonical event root and graph
digest. Separate cases concurrently ingest those signed batches into a
tenant-isolated coordinator, verify its signed receipt chain and persistent
batch-inclusion accumulator, and exercise an order-independent persistent
object accumulator with membership and nonmembership proofs. A dedicated
affected-row case seeds bounded-degree graphs at three sizes, applies the same
single-entity ordinary append to each, records wall time and authenticated row
read counters, and fails unless the structural work is scale-invariant. Each
hot result must also reproduce the exact status, affected entity/edges, and
event, projection, and enterprise snapshot roots after a forced cold rebuild.

The measured 25,000-service fixture produces 300,041 authenticated events,
150,003 entities, 125,003 edges, 25,000 simulation contracts, and 42 bounded
signed batches from eight machines. On the reference macOS ARM64 host,
build/sign took 26.363 seconds, forward and reverse materialization took
16.489 and 17.317 seconds, and the full two-checkpoint enterprise case took
169.180 seconds. The SQLite projection rebuilt in 36.892 seconds, occupied
1,537,720,320 bytes, and answered warm status in 22 ms without reading an
immutable batch body. Central ingestion accepted all 42 uploads concurrently
in 19.827 seconds, survived restart/idempotent replay, and occupied
302,718,976 bytes. The 100,000-object accumulator completed in 5.046 seconds
and touched at most 42 authenticated nodes per update.

The whole benchmark process took about 298 seconds and peaked at approximately
7.50 GB RSS. This is a passing determinism, concurrency, and simulation gate,
but the memory and 1.54 GB derived index are explicit evidence that the current
monolithic reducer is not yet an economical thousands-of-microservices
production architecture. Horizontal partitions and composable roots remain
required.

A separate store append gate now measures one constant-degree ordinary update
across multiple graph sizes. Its receipt distinguishes affected projection
rows from event-id, entity, edge, history, auxiliary, and incident-edge reads;
it rejects a full-projection fallback or any structural counter that grows with
the seeded graph.

The high-fan-in baseline records the current cost of appending one more
observation to the same entity, coverage cell, frontier task, and simulation
runtime. The default is a cheap 64 prior observations per locator. It records
hot-path structural counters, materialized-row byte growth, wall time, sampled
RSS, database size, and exact hot/cold status, row, and root equality. The
currently unmet `events_replayed == 0` and fixed-width-row expectations are
reported as baseline failures inside a passing measurement case; they become
gates only after the normalized per-locator reducer exists. One unrelated
append primes ledger-authority catch-up before timing the four locator appends.

```bash
# One isolated 1,000-observation baseline.
cargo run --release -p provider-local --example scout_benchmark -- \
  --out target/scout-benchmark/high-fan-in-1k \
  --high-fan-in-only \
  --enterprise-fan-in 1000

# Explicit enterprise sweep: 1k, 10k, then 100k observations per locator.
cargo run --release -p provider-local --example scout_benchmark -- \
  --out target/scout-benchmark/high-fan-in-sweep \
  --high-fan-in-only \
  --enterprise-fan-in-sweep
```

The conflict-corpus gate independently seeds 64, middle, and requested-ceiling
sets of real signed coverage disagreements (default 1,000, maximum 100,000).
It then appends one unrelated entity and requires zero conflict writes/deletes,
only the constant 64-row status preview, no full fallback, and exact hot/cold
equality for status plus the event, graph, event-set, projection-map, and
enterprise-snapshot roots. It records wall and projection-rows-per-wall ratios
without enforcing a latency plateau while ledger-authority integration remains
in progress.

Two larger gates keep object-authentication scale separate from graph-reducer
scale:

```bash
cargo run --release -p scout-accumulator --example accumulator_scale -- \
  --objects 1000000 \
  --out target/scout-benchmark/accumulator-scale-1m/receipt.json

cargo run --release -p agent-orchestration \
  --example scout_million_event_gate -- \
  --events 1000000 \
  --services 10000 \
  --out target/scout-benchmark/million-event-gate
```

The one-million-object accumulator completed in 67.10 seconds at
1,424,228,352 bytes peak RSS with 1,999,999 active nodes, order-independent
root construction, and valid membership/nonmembership proofs. The separate
one-million-`EnterpriseEvent` gate completed in 199.361 seconds at
3,628,122,112 bytes sampled peak RSS, materialized 20,000 entities and 10,000
edges with zero conflicts, and produced identical roots and bounded query
results under forward and reverse batch replay. A second full run reproduced
the same event, graph, query, semantic, and storage digests. This is a passing
audit gate, not evidence that the current in-memory reducer should absorb
unbounded enterprise history.

The distributed scheduling oracle and its current coordinator persistence have
a separate frontier-task gate:

```bash
cargo run --release -p scout-coordinator --example scheduler_scale -- \
  --tasks 100000 \
  --claim-batch 1024 \
  --out target/scout-benchmark/scheduler-scale-100k
```

The current 100,000-task reference passed target-affine fenced claiming, exact
operation replay, restart receipt equality, and the enforced latency gates. A
fresh run used 473,063,424 bytes of coordinator state, claimed 1,024 tasks in
891 ms, returned the exact idempotent retry in 1 ms, and reconstructed the
restart receipt in 292 ms. It mutated 1,024 task rows while leaving 98,976
untouched. The portable full-state oracle remains the semantic reference; the
hosted coordinator uses normalized affected rows and streams the legacy exact
root over all tasks.

`external` containment is an explicit capability limitation. To test a Linux
bubblewrap boundary, launch the binary inside a read-only root with one writable
output bind and pass `--containment bwrap --denied-write PATH`, where `PATH` is
inside a read-only bind. The benchmark fails if that negative write succeeds.

UTM qualification must cross the guest boundary with a byte-for-byte binary
read-back and a marker-authenticated result. A successful `utmctl exec` return
alone is not evidence. Build the appropriate portable binary, then run:

```bash
node harness/scout-utm-qualify.mjs \
  --platform ubuntu \
  --binary target/aarch64-unknown-linux-musl/release/examples/scout_benchmark \
  --reference target/scout-benchmark/local/receipt.json \
  --out target/scout-benchmark/utm-ubuntu

node harness/scout-utm-qualify.mjs \
  --platform windows \
  --binary target/aarch64-pc-windows-msvc/release/examples/scout_benchmark.exe \
  --reference target/scout-benchmark/local/receipt.json \
  --out target/scout-benchmark/utm-windows
```

The qualifier refuses to overwrite an output directory. It verifies the
binary read-back, guest receipt/report hashes, canonical and semantic graph
roots, and exact guest scratch cleanup. A Windows failure receipt also carries
the read-back length/hash plus path-scoped Defender, signature, and file-state
evidence. Quarantine is a failed packaging/trust gate; do not add an exclusion
or disable real-time protection to turn it green.

Live target adapters have a separate credential-safe qualification harness:

```bash
CLARK_EXEC_TOKEN="$CLARK_EXEC_TOKEN" \
SCOUT_GITHUB_AUTHORITY="$SCOUT_GITHUB_AUTHORITY" \
node harness/scout-adapter-live-qualification.mjs \
  --url ws://127.0.0.1:PORT \
  --root /target/project/.clark/scout/adapters/private \
  --out target/scout-benchmark/live-adapter
```

The authenticated target service must already be reachable through the Clark
SSH/VM transport. The harness verifies every opaque candidate and follows
provider cursors, but its receipt contains only status classes, counts, and
hashes—never credential values, principal ids, authority names, provider
records, or raw cursors.
