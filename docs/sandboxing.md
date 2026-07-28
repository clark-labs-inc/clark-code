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
protocol rather than another product's identities or state.

The Windows backend accepts only the product's host-wide-read policy shape. It
rejects narrowed `read_roots`, `deny_read`, or enabled child networking at the
protocol boundary instead of claiming to enforce those unsupported shapes.
Actual readability still follows the dedicated offline account's Windows ACLs:
the current setup transaction grants the selected write roots but does not yet
install broader user-profile read/execute ACLs. Consequently,
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
2. The inline composer card blocks the first local command until the user
   clicks **Enable sandbox**. Windows shows one trusted-publisher UAC consent
   surface; the setup helper itself remains hidden.
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

Bootstrap state lives under
`%LOCALAPPDATA%\Clark\Code\sandbox` (or `Code Dev` for development builds), not
under the NSIS installation directory. The installer migrates the former
`%LOCALAPPDATA%\Clark Code\sandbox` location and preserves either location
across an upgrade or uninstall, so an application replacement cannot silently
discard the enrollment marker.

The proof files prevent either enrollment path from acting as a generic ACL
deputy: it can grant the sandbox identity only to roots where the unelevated
caller could already create files. The desktop cleans them after success,
failure, or UAC cancellation. Each project has a distinct restricting SID, so
previously enrolled ACLs are inert unless that exact root is active in the
current token. Plan/read-only mode omits the project SID while retaining only
the already enrolled Clark document/temp roots.

Windows release builds fail closed unless Azure Artifact Signing can sign a
disposable PE through the production SignTool/dlib boundary before any
benchmark starts. GitHub authenticates with Azure through OIDC; no exportable
PFX or long-lived client secret is used. Tauri's structured `signCommand`
signs the main executable and NSIS installer while the bundle is being
created, and Clark signs every private helper before packaging. The release
then requires each installed executable to have a valid Authenticode
signature with the configured Clark publisher subject and the same
short-lived Artifact Signing certificate used for that release run.

The per-user NSIS installer is also checked for no VC++ runtime dependency,
silent install/start/uninstall behavior, and Tauri updater integrity under the
separate pinned Ed25519 updater key. Helpers remain outside child `PATH`, are
resolved by absolute path, and validate their private sibling location before
privileged work.

The external release identity prerequisites are:

| GitHub setting | Purpose |
| --- | --- |
| Secrets `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID` | Entra workload identity used by `azure/login` through GitHub OIDC. |
| Variables `AZURE_ARTIFACT_SIGNING_RESOURCE_GROUP`, `AZURE_ARTIFACT_SIGNING_PROFILE` | Exact live Clark certificate-profile resource. |
| Variable `CLARK_WINDOWS_SIGNER_SUBJECT` | Exact subject expected on every released executable. |
| Optional variables `AZURE_ARTIFACT_SIGNING_ENDPOINT`, `AZURE_ARTIFACT_SIGNING_ACCOUNT` | Override the East US endpoint and `clarkcodesigning` account defaults. |
| Variable `CLARK_DESKTOP_DOWNLOAD_UPLOAD_ROLE_ARN` | AWS role that writes immutable candidates and advances the public channel. |
| Secrets `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Detached updater-artifact signing identity. |

GitHub must have a protected `release` environment whose deployment policy
allows only stable `vX.Y.Z` tags. The Entra principal must have a federated
credential for that environment's exact OIDC subject (rather than one
credential per tag) and the Artifact Signing Certificate Profile Signer role
on the profile. The AWS upload role must trust the same environment subject.
Both cloud identities are exercised by the live prerequisite job before
benchmarks or native builds begin. The profile itself cannot exist until Azure
identity validation succeeds.

The AWS role needs `GetObject`, `HeadObject`-equivalent read access,
`PutObject`/server-side copy access under `desktop/releases/*` and
`desktop/latest/*`, optional `DeleteObject` for successful rollback-snapshot
cleanup, and CloudFront invalidation permission for `/desktop/latest/*`.
Failure to create or validate the rollback snapshot happens before the first
mutable channel write and therefore blocks the release.

With this repository's default GitHub OIDC claim template, that subject is
`repo:clark-labs-inc/clark-desktop:environment:release`. Re-check the
repository OIDC customization before changing the claim template; Entra and
AWS must match the token's exact subject.

## Windows shell packaging

Clark does not bundle MinGW, MSYS2, or Git Bash as part of the trusted Windows
runtime. MinGW is a compiler toolchain, while Git Bash/MSYS2 is the Unix-like
shell environment users usually mean by this request. Bundling either would
add a second quoting, path, permission, update, licensing, and vulnerability
surface without fixing Windows sandbox enrollment or console visibility.

The native execution contract is PowerShell 7 when installed, then Windows
PowerShell, then `COMSPEC`/`cmd.exe`. Agent-authored commands use redirected
pipes and never allocate a console. Only an explicitly interactive terminal
uses ConPTY. Clark may expose detected user-installed Git Bash or WSL
distributions as optional profiles; if zero-install POSIX compatibility
becomes a product requirement, it should be a separately versioned optional
component rather than part of every signed installer.

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
`windows_native` suite. A release also requires a self-hosted macOS ARM64 runner
with the `clark-utm-qa` label. Before paid or packaging work, that runner starts
the exact stopped `Clark QA - Windows 11 ARM` golden VM and proves a fresh
framebuffer, guest agent, TPM, enabled UAC, no Clark installation or process,
no sandbox state or offline identity, no Clark sandbox firewall rule, and no
Clark WebView profile. The lane then stops that base and creates a uniquely
named disposable clone for the release run. The base is never used for an
install, and the clone is deleted in an unconditional cleanup job. The install
probe fails if any of those artifacts reappear; it never deletes pre-existing
state and then calls the result pristine.

After packaging, that clone must prove install, inline setup, a real pipe-backed
and PTY-backed command through the packaged sandbox, inside write, blocked
outside write, native containment, restart persistence, absence of console
windows, an exact Clark Authenticode subject and certificate thumbprint, signed
update, public CDN identity, and source revision. Candidate bytes are accepted
only when they match the signed Windows build receipt's hash and size. UAC
enrollment requires both a fresh exact-UTM-window screenshot and an observed
Windows `consent.exe` process before the autonomous consent input is sent.

Public channel publication snapshots all seven mutable objects before its first
write. Stable installer aliases and `manifest.json` advance before
`latest.json`, which is the final updater pointer. Rendered-site and packaged
post-publish journeys run before the draft GitHub release becomes public. Any
failure restores the complete prior object generation with its original
metadata and invalidates the CDN; a failed rollback snapshot is retained for
operator recovery. The snapshot is deleted only after the GitHub release is
successfully committed. Snapshot and restore validation includes ETag and any
available S3 SHA-256 checksum. The monotonic guard reads the authoritative S3
pointer, and the public journey streams and hashes the real bytes behind every
immutable installer URL and every stable website alias before publication.

The paid model test is ignored and environment-gated by design; run it only
with explicit authorization, an exact model, a cost cap, and a dedicated key:

```bash
CLARK_SANDBOX_E2E_MODEL=clark-code:minimax_m3 \
CLARK_CODE_API_KEY=... \
cargo test -p provider-local --test sandbox_live \
  paid_cheapest_model_cannot_escape_workspace -- --ignored --nocapture
```

That receipt must show at least two shell attempts, an inside file, no outside
file, and a completed run. It is evidence that the real model/provider/tool loop
reaches the sandbox; it does not replace native OS-boundary tests.
