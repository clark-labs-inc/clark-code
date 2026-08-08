# `code-host`

`code-host` is the reusable, provider-neutral control plane for headless coding
agents. It owns the boundaries that must stay stable while workers and plugins
change:

- registered project IDs resolve to canonical roots; request payloads cannot
  select arbitrary paths;
- a strict versioned JSONL envelope supports ping, catalog, invoke, cancel, and
  shutdown; protocol v2 adds ordered progress frames followed by exactly one
  terminal result or error;
- `HeadlessPlugin` implementations advertise manifests and receive a resolved
  `PluginContext`;
- every invocation gets a cancellation token and a durable start/finish record;
- invocations reserve an owner-private request receipt before running,
  persist bounded ordered progress plus the terminal response before replying,
  replay exact completed retries after worker restart, and fail closed for an
  interrupted or conflicting request ID;
- durable receipts have a fixed 512 MiB budget per trajectory root. The host
  never evicts an old request ID: it refuses untracked new work when full and
  leaves an oversized completion ambiguous rather than risking re-execution.

The host does not load arbitrary dynamic libraries or let a model install code
into its own process. Expansion happens through typed plugin registration at
the worker composition root, so a new plugin can be added without changing the
control protocol or existing plugins.

```rust,ignore
let mut host = HeadlessHost::new(projects, trajectory_root);
host.register_plugin(MyPlugin::new())?;
let response = host.handle(request).await;
```

The worker composition root uses `handle_stream` with a bounded output channel.
Plugins publish through `PluginContext::progress`; backpressure therefore
reaches the producing plugin instead of creating an unbounded transcript. The
durable replay capture is separately capped at 4,096 frames and 8 MiB; exceeding
that cap makes a retry ambiguous instead of risking duplicate execution.
