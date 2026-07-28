# Windows first-run and release reliability postmortem

Date: 2026-07-27

## Customer-visible incident

The released Windows client could report:

```text
project sandbox is not ready: SetupRequired {
  backend: WindowsRestrictedToken,
  reason: "read Windows sandbox setup marker ... setup-marker-v1.json:
  The system cannot find the path specified. (os error 3)"
}
```

The error suggested disabling the sandbox or choosing Full Access. Full Access
then ran commands on the host, while ordinary command execution could flash
PowerShell, CMD, conhost, or Windows Terminal surfaces. The public installer was
also older than the unshipped client work users expected to receive.

## Root cause

This was a chain of contract failures rather than one missing directory.

1. A missing marker is a legitimate first-run `SetupRequired` state, but the
   provider-construction boundary converted it into a terminal capability error
   before the user had an actionable setup surface.
2. The composer did not query sandbox readiness, block submission, or present
   the existing explicit setup command inline. Full Access bypassed containment,
   so it appeared to be the required fix even though it was an unsafe escape
   hatch.
3. Production sandbox state lived at `%LOCALAPPDATA%\Clark Code\sandbox`,
   nested under the per-user NSIS install directory. Replacement or uninstall
   could therefore remove security enrollment along with application files.
4. Ordinary agent shell calls used the PTY executor even though they were
   non-interactive. The elevated console-subsystem setup helper was also shown
   with `SW_SHOWNORMAL`. Those choices allowed visible console surfaces from a
   GUI application.
5. The release version in source and the latest tag both remained `0.1.91`.
   The CDN was serving that tagged release; newer workspace code had never been
   versioned and released. Because mutable release documents carried no source
   revision and the website aliases were not verified, the old pipeline also
   could not prove which commit every public surface represented.

The v0.1.91 workflow provides the concrete publication failure. All build jobs
passed, and the publish job uploaded immutable and mutable `latest.json` and
`manifest.json` objects. It then failed because the AWS role could create a
CloudFront invalidation but could not call `GetInvalidation`, which the waiter
required. The job ended at 16:09:37 UTC; the GitHub release was nevertheless
public at 16:11:53 UTC. The workflow did not advance or verify the five stable
installer aliases used by the rendered Clark page, so its documents, website
links, and GitHub release could represent different generations after a failed
run.

## Why the previous gates passed

| Previous gate | What it proved | Missing customer boundary |
| --- | --- | --- |
| Deterministic benchmark on Ubuntu | Source-level tool and feature behavior | No Windows package, UAC, installer, or WebView journey |
| Paid benchmark on macOS | Real model loop under Seatbelt | No Windows restricted-token setup |
| `windows-latest` package smoke | Silent NSIS install, process startup, helper `--self-test`, layout, uninstall | No first-run UI, UAC consent, actual restricted command, restart persistence, or visible-window observation |
| Native helper tests | Individual protocol and containment primitives | The installed desktop never drove those primitives as a user would |
| Optional UTM receipts | A reusable VM could execute probes | Not a mandatory release dependency; guest was already installed and retained `ClarkSandboxOffline` |
| CDN convergence check | `latest.json` reached the intended version | No installer aliases, rendered website links, source revision, signed installed identity, or rollback |
| CloudFront waiter | Invalidation creation plus readback when IAM allowed it | Mutable pointers were already written; an IAM read failure left no rollback and did not prevent later manual publication |

The simulations were not useless; they were answering narrower questions than
their release-gate names implied. The first broken contract was at the packaged
first-run boundary, which none of them exercised.

## Corrections

- The composer now fails closed on an exact cwd-scoped sandbox status and shows
  one inline **Enable sandbox** action. Folder changes cannot reuse another
  project's readiness result.
- UAC is the only visible privileged surface. The helper is hidden, production
  binaries use one Clark Authenticode identity, and ordinary shell execution is
  pipe-backed. Only the explicit integrated terminal uses ConPTY.
- Sandbox product data moved to `%LOCALAPPDATA%\Clark\Code\sandbox`; installer
  hooks migrate and preserve legacy state.
- Windows packages use native PowerShell/COMSPEC discovery. The package verifier
  rejects bundled MinGW, MSYS2, Git Bash, and their shell/toolchain binaries.
- A signed build receipt binds tag, commit, installer hash/size, signer subject,
  and certificate thumbprint. The UTM guest accepts only those exact bytes and
  installed identities.
- The release starts from a pristine UAC-enabled golden VM, stops it, clones it,
  runs the full packaged first-run/update/post-publish journey on the disposable
  clone, and deletes the clone afterward. The candidate installer refuses to
  clean up or mask an existing install, registry entry, sandbox directory,
  offline identity, or Clark firewall rule.
- Required receipts prove inline setup, Full Access warning copy, real
  pipe/PTY commands through the enrolled sandbox, inside write, blocked outside
  write, a fresh exact-VM screenshot while Windows `consent.exe` is active,
  native containment, no console windows, restart persistence, update, public
  CDN identity, and source revision.
- Mutable public objects are snapshotted before the first write. Installer
  aliases and `manifest.json` advance before `latest.json`; browser and packaged
  post-publish checks run before the GitHub draft becomes public. A failure
  restores and verifies the prior generation. The downgrade guard reads the
  authoritative S3 pointer rather than a potentially stale CDN response, and
  the public journey hashes every immutable installer and stable alias body.
- Every Azure/AWS writer uses the protected `release` environment OIDC subject.
  The tag must be stable, match `tauri.conf.json`, and point to a commit contained
  in `origin/main`.

## Release-blocking external readiness

The code intentionally cannot turn these infrastructure gaps into a pass:

- GitHub needs a protected `release` environment, Azure OIDC secrets and
  Artifact Signing variables, and a registered online self-hosted runner with
  the exact `clark-utm-qa` label.
- Azure needs the environment-subject federated credential and the live Clark
  certificate profile.
- AWS must trust the same environment subject and permit immutable upload,
  mutable-channel snapshot/copy, optional snapshot cleanup, and CloudFront
  invalidation.
- The golden `Clark QA - Windows 11 ARM` VM must contain no Clark installation,
  sandbox state, offline identity, firewall rule, or Clark WebView profile.

As of the dated audit, the VM still contained Clark Code `0.1.91` and the
`ClarkSandboxOffline` identity, so the new pristine preflight correctly returned
`blocked`. No release should be published until a signed packaged journey
produces passing receipts on a remediated golden base.
