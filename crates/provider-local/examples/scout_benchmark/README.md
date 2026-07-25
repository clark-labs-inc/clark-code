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

It writes `receipt.json` and `report.md` into a new output directory and refuses
to overwrite an existing run. The canonical SHA-256 covers only ordered case
ids and pass/fail states, so equivalent runs on different operating systems can
be compared directly.

The cases exercise capability-safe skill resolution, the exhaustive
all-manifest-rows-terminal contract, append-only ledger replay, authority and
self-certification rejection, replay-recipe and proof-tier requirements,
Wilson reference values, and the same seeded-bootstrap kernel used by
`scout_measure`.

`external` containment is an explicit capability limitation. To test a Linux
bubblewrap boundary, launch the binary inside a read-only root with one writable
output bind and pass `--containment bwrap --denied-write PATH`, where `PATH` is
inside a read-only bind. The benchmark fails if that negative write succeeds.
