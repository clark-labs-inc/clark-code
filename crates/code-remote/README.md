# `code-remote`

`code-remote` is the host-side SSH transport for a complete headless coding
worker. It does not execute provider calls itself: it deploys and starts a
host-configured worker binary, then carries the strict `code-host` JSONL
protocol over the SSH stdio channel. Worker identity, provider composition,
credentials, plugins, and product-specific invoke lanes belong to the
downstream product.

The lifecycle is deliberately bounded:

1. Validate the project/trajectory roots, worker config, credential variable,
   and shell-safe host/path inputs.
2. Open one process-owned SSH master on a private mode-`0700` control socket.
   Every probe, upload, and worker channel multiplexes over that connection;
   neither the socket nor the master persists after the worker lifecycle.
3. Probe the remote OS and home, create only the registered roots and configured
   artifact directories, and select a versioned architecture-specific binary.
4. Upload the binary through a random partial path, verify SHA-256 remotely,
   and atomically install it mode `0700`. Install the credential-free,
   content-addressed worker config mode `0600` only when its digest is absent.
5. Start the worker with a multiplexed `ssh -T` channel and, when configured,
   send one credential
   line through encrypted SSH stdin. The credential is not in argv, config,
   trajectory, or responses.
6. Correlate concurrent requests by `request_id`; validate each progress
   sequence, bound per-request buffering, and require exactly one terminal
   response. The worker durably reserves mutating IDs before execution and
   replays their bounded ordered progress and terminal response after a lost
   connection or process restart; an incomplete or conflicting receipt fails
   closed. Cancellation and shutdown remain first-class protocol messages. A ping must prove
   `execution_residency=remote_worker` before connect succeeds.
7. On disconnect, cancel active work and tear down both the worker channel and
   its SSH master. The
   credential-free, content-addressed config remains as an immutable cache for
   the next process start.

Protocol v2 is intentionally incompatible with stale workers that cannot
stream ordered progress. The startup ping rejects an old protocol before it can
own a conversation.

The Tauri `remote_worker_connect` command is the sole attach/reconnect
boundary. It health-checks an existing account/project worker, reuses it when
healthy, or atomically replaces it after a fresh bootstrap while keeping the
opaque handle stable for attached sessions. It never persists the bootstrap
secret. Completed request retries replay from the worker's owner-private
receipt; incomplete or mismatched retries are rejected without re-execution.

Use `RemoteWorkerSpec` only from a trusted native caller. The foundation's
`remote_worker_connect` command delegates worker selection, credential binding,
and launch policy to `ProductIntegration::prepare_remote_worker`.

For the trusted GPU autoresearch lane, the remote worker config may explicitly
set `provider.permission_mode` to `allow_anything` together with
`provider.sandbox_mode` `disabled` or `danger-full-access`. The worker then
auto-answers tool permission requests for both its Scientist and Engineer
effects. This profile is valid only with `execution_residency:
"remote_worker"`; local workers must keep the default prompt/permission path.
The transport still validates the registered project and remote roots, and the
worker retains its budget, residency, and terminal-receipt checks.

Product-owned live transport and paid model receipts are intentionally outside
this crate's normal CI contract.
