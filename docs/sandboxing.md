# Local command sandbox

Clark Code runs local agent commands through one platform-neutral policy and
executor boundary. The default policy is `workspace_write`: host reads are
available, writes are limited to the selected project plus Clark-owned document
and private temporary roots, `.git` metadata is denied, child processes inherit
the boundary, and direct network access is denied.

Clark Cloud is intentionally separate from child-process networking. The typed
`clark_research` host tool is brokered by the application, enabled when a Clark
API key is present, and allowed without a permission prompt. Giving that tool
network access never gives `bash` or any descendant process a socket.

## Architecture

| Layer | Responsibility |
| --- | --- |
| `exec-core` | Lossless argv/env/cwd process contract, streaming/PTY execution, process-tree fence. |
| `exec-sandbox` | Neutral policy, backend selection, environment preparation, setup-action contract. |
| `exec-sandbox-protocol` | Size-bounded, versioned Windows runner/setup wire format and policy attestation. |
| `exec-sandbox-windows` | Offline identity, firewall, ACL provisioning, restricted-token launch, job-object lifetime. |
| `provider-local` | Maps product modes to policies and attaches Clark docs/private session temp roots. |
| Tauri + React | Read-only status, one-time Windows UAC bootstrap, and protected-root fallback. |

The platform adapters are deliberately small:

- macOS compiles the policy to a Seatbelt profile and invokes the system
  `sandbox-exec` boundary.
- Linux invokes a pinned, privately bundled bubblewrap, with a distro binary as
  a verified fallback.
- Windows launches a private helper under a dedicated offline local identity,
  then applies `CreateRestrictedToken`, per-root capability SIDs, outbound
  firewall rules (including loopback), and a kill-on-close Job Object. The
  account satisfies the normal ACL check while only the active roots' capability
  SIDs satisfy the restricted check, so grants from older projects are inert.
  The runner loads the offline profile, redirects profile/temp writes into the
  private session root, and injects an environment-only Git `safe.directory`
  entry while keeping fsmonitor and interactive prompts disabled.

## Package boundary

Packaged commands and implementation helpers occupy different physical trees:

```text
clark-path/
  rg[.exe]
clark-resources/
  sandbox/linux/bwrap
  sandbox/windows/clark-command-runner.exe
  sandbox/windows/clark-windows-sandbox-setup.exe
  licenses/...
```

Only `clark-path` is prepended to child `PATH`. Sandbox helpers are resolved by
absolute path and never fall back to command lookup. Tauri keeps the signed
macOS `rg` sidecar in `Contents/MacOS`; because macOS uses the system Seatbelt
binary and has no private Clark sandbox helper, that signing-specific placement
does not mix public and privileged tools.

The Linux bundle carries the exact digest-pinned bubblewrap source archive with
its LGPL notices. Windows helpers are built from Clark's crates, installed
under the private resource tree, and covered by a versioned runner/setup
protocol rather than Codex's identities or state.

The Windows backend accepts only the product's host-wide-read policy shape. It
rejects narrowed `read_roots`, `deny_read`, or enabled child networking at the
protocol boundary instead of claiming to enforce those unsupported shapes.
Actual readability still follows the dedicated offline account's Windows ACLs:
the current setup transaction grants the selected write roots but does not yet
install Codex's broader user-profile read/execute ACL set. Consequently,
dependencies stored below a private primary-user profile (for example a private
Cargo cache) are not yet guaranteed readable on Windows. The backend reports
this limitation here rather than treating policy-shape validation as proof of a
host-wide read boundary.

`danger_full_access` is the only explicit no-sandbox mode. Read-only uses the
same boundary with no project write grant. If a required backend is unavailable,
the required mode fails closed; automatic mode reports the fallback explicitly.

## Windows setup and enrollment

Windows uses one privileged machine bootstrap, followed by user-mode workspace
enrollment:

1. The unelevated desktop creates an unpredictable `create_new` proof file
   directly inside every requested grant root.
2. The user clicks **Enable sandbox** and Windows shows an **Unknown publisher**
   UAC prompt and command prompt once.
3. The elevated helper validates its private install and state locations,
   creates or rotates the offline identity, DPAPI-protects its credential, and
   installs and reads back SID-scoped outbound-deny firewall rules.
4. It grants one stable, device-only restricting SID access to Windows' NUL
   object and atomically commits the bootstrap marker. That SID is never
   granted to a filesystem root.
5. The initial workspace is enrolled inside the same elevated transaction.
   Later user-owned workspaces are enrolled by the desktop without UAC: it
   consumes the ownership proofs, obtains `WRITE_DAC` using the caller's
   existing ownership, installs the offline-account and root-specific ACEs,
   and commits the exact policy fingerprint last.
6. If Windows refuses user-mode `WRITE_DAC` for a protected or administrator-
   owned root, the explicit setup action retries through the bundled helper and
   presents UAC only for that exceptional root.

The proof files prevent either enrollment path from acting as a generic ACL
deputy: it can grant the sandbox identity only to roots where the unelevated
caller could already create files. The desktop cleans them after success,
failure, or UAC cancellation. Each project has a distinct restricting SID, so
previously enrolled ACLs are inert unless that exact root is active in the
current token. Plan/read-only mode omits the project SID while retaining only
the already enrolled Clark document/temp roots.

Windows release builds currently ship unsigned. They still build and verify
both privilege helpers, use a per-user NSIS installation, exercise install,
startup, and uninstall, verify no VC++ runtime dependency, and keep Tauri
updater artifacts signed with the pinned updater key. Windows may show
SmartScreen and **Unknown publisher** UAC warnings until Clark adopts publicly
trusted Authenticode signing. The helpers remain outside child `PATH`, are
resolved by absolute path, and validate their private sibling location before
privileged work.

## Verification

The required native suites probe all security boundaries, rather than treating
policy compilation as containment proof:

- inside write succeeds;
- outside write fails and creates no file;
- `.git` write fails;
- spawned-child escape fails;
- symlink/junction escape fails;
- PTY execution remains contained;
- loopback networking fails;
- detached descendants die with the parent fence.
- switching projects cannot write a previously consented project through stale
  ACLs.
- switching to read-only drops the project capability while private tool temp
  remains usable.

Run deterministic native conformance and the benchmark with:

```bash
CLARK_SANDBOX_E2E_REQUIRED=1 cargo test -p exec-sandbox --test sandbox_e2e -- --nocapture
cargo run -p exec-sandbox --example sandbox_benchmark -- --iterations 5000 --launch-iterations 30
```

Windows CI additionally builds the release helpers and runs the machine-mutating
`windows_native` suite. The paid model test is ignored and environment-gated by
design; run it only with explicit authorization, an exact model, a cost cap, and
a dedicated key:

```bash
CLARK_SANDBOX_E2E_MODEL=clark-code:kimi_k27_code \
CLARK_CODE_API_KEY=... \
cargo test -p provider-local --test sandbox_live \
  paid_cheapest_model_cannot_escape_workspace -- --ignored --nocapture
```

That receipt must show at least two shell attempts, an inside file, no outside
file, and a completed run. It is evidence that the real model/provider/tool loop
reaches the sandbox; it does not replace native OS-boundary tests.
