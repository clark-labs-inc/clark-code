//! Shell-command risk classification.
//!
//! `bash` is the one tool that can reach outside the project sandbox, so every
//! command is classified before it runs. The engine uses this for two things:
//!   * a HARD FLOOR — `Blocked` commands are refused no matter the permission
//!     mode or allowlist (catastrophic, irreversible system damage); and
//!   * RISK-AWARE GATING — `Danger`/`Caution` prompt the user (with a reason)
//!     even in the auto-approve mode, while `Safe` read-only/dev commands run
//!     without nagging.
//!
//! Classification is intentionally conservative: an unrecognized command lands
//! in `Caution`, so anything we don't understand still prompts.

/// How risky a shell command is, lowest to highest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandRisk {
    /// Read-only / standard dev commands (ls, grep, git status, cargo test…).
    Safe,
    /// Mutates the project or fetches from the network (installs, mv, commit…).
    Caution,
    /// Could destroy work or escalate (rm -rf, sudo, force-push, curl | sh…).
    Danger,
    /// Catastrophic and irreversible (rm -rf /, mkfs, dd to a device, fork
    /// bomb…). Never run, regardless of mode.
    Blocked,
}

impl CommandRisk {
    pub fn as_str(self) -> &'static str {
        match self {
            CommandRisk::Safe => "safe",
            CommandRisk::Caution => "caution",
            CommandRisk::Danger => "danger",
            CommandRisk::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Classification {
    pub risk: CommandRisk,
    /// Short, user-facing why ("recursive delete", "runs as sudo"). None for Safe.
    pub reason: Option<String>,
}

impl Classification {
    fn new(risk: CommandRisk, reason: &str) -> Self {
        Self {
            risk,
            reason: Some(reason.to_string()),
        }
    }
    fn safe() -> Self {
        Self {
            risk: CommandRisk::Safe,
            reason: None,
        }
    }
}

/// Read-only / inspection programs — never mutate, safe to run unprompted.
const READONLY: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "pwd",
    "echo",
    "printf",
    "which",
    "type",
    "whoami",
    "id",
    "date",
    "env",
    "printenv",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "ag",
    "find",
    "fd",
    "wc",
    "sort",
    "uniq",
    "cut",
    "tr",
    "awk",
    "sed",
    "diff",
    "tree",
    "file",
    "stat",
    "du",
    "df",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "uname",
    "hostname",
    "ps",
    "top",
    "true",
    "false",
    "test",
    "tee",
    "column",
    "jq",
    "yq",
    "xargs",
    "tar",
    "unzip",
    "gzip",
    "gunzip",
    "base64",
    "md5",
    "shasum",
    "sha256sum",
    "nproc",
    "sleep",
    "start-sleep",
    "clear",
];

/// Build / test / lint programs — standard dev loop, run without nagging.
const DEV_TOOLS: &[&str] = &[
    "cargo",
    "rustc",
    "rustfmt",
    "clippy-driver",
    "npm",
    "pnpm",
    "yarn",
    "bun",
    "node",
    "deno",
    "tsc",
    "eslint",
    "prettier",
    "make",
    "cmake",
    "go",
    "python",
    "python3",
    "pip",
    "pip3",
    "pytest",
    "ruff",
    "mypy",
    "black",
    "isort",
    "poetry",
    "uv",
    "ruby",
    "rake",
    "bundle",
    "gradle",
    "mvn",
    "dotnet",
    "java",
    "javac",
    "php",
    "composer",
    "swift",
    "xcodebuild",
    "ctest",
    "ninja",
    "cmake",
];

/// Read-only git subcommands. (`branch`/`tag`/`config`/`stash` are NOT here —
/// they have mutating forms and are classified per-form in `classify_git`.)
const GIT_READONLY: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "remote",
    "rev-parse",
    "ls-files",
    "blame",
    "describe",
    "rev-list",
    "shortlog",
    "reflog",
    "cat-file",
    "for-each-ref",
];

/// git subcommands that can destroy local work.
const GIT_DESTRUCTIVE: &[&str] = &[
    "reset", "clean", "checkout", "switch", "restore", "rebase", "push",
];

/// Package-install / network programs (mutate deps or reach the network).
const INSTALLERS: &[&str] = &[
    "apt", "apt-get", "brew", "yum", "dnf", "pacman", "apk", "gem", "snap", "port", "nix",
];

fn program(token: &str) -> &str {
    // Strip a leading path: /usr/bin/grep -> grep.
    token.rsplit('/').next().unwrap_or(token)
}

/// Classify a full shell command line (may contain `&&`, `;`, `|`, etc.). The
/// overall risk is the highest of its parts.
pub fn classify_command(command: &str) -> Classification {
    let lower = command.to_lowercase();

    // --- HARD FLOOR: catastrophic, irreversible. Checked on the whole line so
    // operator tricks (rm -rf / & ) can't slip a segment past us. ---
    if is_fork_bomb(&lower) {
        return Classification::new(CommandRisk::Blocked, "fork bomb");
    }
    if mentions(&lower, &["mkfs", "mke2fs"]) {
        return Classification::new(CommandRisk::Blocked, "formats a filesystem");
    }
    if lower.contains("dd ") && (lower.contains("of=/dev/") || lower.contains("of=\\\\.\\")) {
        return Classification::new(CommandRisk::Blocked, "writes a raw disk device");
    }
    if lower.contains("of=/dev/") || lower.contains("> /dev/sd") || lower.contains(">/dev/sd") {
        return Classification::new(CommandRisk::Blocked, "writes to a disk device");
    }
    if is_root_destroyer(&lower) {
        return Classification::new(
            CommandRisk::Blocked,
            "recursive delete of a system / home root",
        );
    }
    if lower.contains("chmod") && lower.contains("-r") && contains_root_target(&lower) {
        return Classification::new(CommandRisk::Blocked, "recursive chmod of a system root");
    }

    // Whole-line check: a remote payload piped into a shell (the `|` is gone once
    // we split into segments, so detect it here).
    let mut worst = Classification::safe();
    if (lower.contains("curl") || lower.contains("wget"))
        && ["| sh", "|sh", "| bash", "|bash", "| zsh", "|zsh"]
            .iter()
            .any(|p| lower.contains(p))
    {
        worst = Classification::new(CommandRisk::Danger, "pipes a remote script into a shell");
    }

    // --- Per-segment risk; take the max. ---
    for segment in split_segments(command) {
        let c = classify_segment(segment);
        if c.risk > worst.risk {
            worst = c;
        }
    }

    // Command substitution / backticks can hide an entirely different command
    // inside an otherwise-benign line (`echo $(rm -rf ~/x)`), and `split_segments`
    // doesn't descend into them. We can't see inside, so keep the line out of
    // `Safe` — it must at least prompt (in auto mode) instead of auto-running as
    // if it were the harmless outer program.
    if worst.risk < CommandRisk::Caution && (lower.contains("$(") || lower.contains('`')) {
        worst = Classification::new(CommandRisk::Caution, "contains command substitution");
    }
    worst
}

fn classify_segment(segment: &str) -> Classification {
    let lower = segment.to_lowercase();
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    let Some(first) = tokens.first() else {
        return Classification::safe();
    };
    let normalized_program = program(first).to_ascii_lowercase();
    let prog = normalized_program.as_str();
    let flags: String = tokens
        .iter()
        .skip(1)
        .filter(|t| t.starts_with('-'))
        .map(|t| t.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");

    match prog {
        "sudo" | "su" | "doas" => Classification::new(CommandRisk::Danger, "runs as another user"),
        "rm" => {
            if flags.contains('r') || flags.contains('f') {
                Classification::new(CommandRisk::Danger, "recursive / forced delete")
            } else {
                Classification::new(CommandRisk::Caution, "deletes files")
            }
        }
        "rmdir" => Classification::new(CommandRisk::Caution, "removes a directory"),
        "kill" | "killall" | "pkill" => Classification::new(CommandRisk::Danger, "kills processes"),
        "chmod" | "chown" | "chgrp" => {
            if flags.contains('r') {
                Classification::new(CommandRisk::Danger, "recursive permission change")
            } else {
                Classification::new(CommandRisk::Caution, "changes permissions")
            }
        }
        "dd" => Classification::new(CommandRisk::Danger, "low-level disk write"),
        "eval" | "exec" | "source" | "." => {
            Classification::new(CommandRisk::Danger, "evaluates arbitrary code")
        }
        "git" => classify_git(&tokens),
        "cargo" => classify_cargo(&tokens),
        "mv" | "cp" | "ln" | "mkdir" | "touch" | "rename" => {
            Classification::new(CommandRisk::Caution, "modifies the filesystem")
        }
        "curl" | "wget" | "nc" | "ncat" | "ssh" | "scp" | "rsync" | "ftp" => {
            Classification::new(CommandRisk::Caution, "network access")
        }
        "gh" if is_publish(prog, &lower) => {
            Classification::new(CommandRisk::Danger, "publishes / deploys")
        }
        "gh" => Classification::new(CommandRisk::Caution, "accesses GitHub"),
        "docker" | "podman" | "kubectl" | "terraform" | "systemctl" | "launchctl" | "service" => {
            Classification::new(CommandRisk::Danger, "controls system / infra")
        }
        p if p == "npm" || p == "pnpm" || p == "yarn" || p == "bun" => classify_node(&tokens),
        p if INSTALLERS.contains(&p) => {
            Classification::new(CommandRisk::Caution, "installs system packages")
        }
        p if is_publish(p, &lower) => {
            Classification::new(CommandRisk::Danger, "publishes / deploys")
        }
        p if READONLY.contains(&p) => Classification::safe(),
        p if DEV_TOOLS.contains(&p) => Classification::safe(),
        _ => Classification::new(CommandRisk::Caution, "unrecognized command"),
    }
}

fn classify_git(tokens: &[&str]) -> Classification {
    let sub = tokens.get(1).map(|s| program(s)).unwrap_or("");
    let rest = tokens.join(" ").to_lowercase();
    // Non-flag args after the subcommand distinguish listing forms
    // (`git branch`, `git tag -l`) from mutating ones (`git branch x`).
    let has_positional = tokens.iter().skip(2).any(|t| !t.starts_with('-'));
    if sub == "stash" {
        // Everything except list/show moves or drops working-tree changes —
        // in a shared tree that can hide another agent's in-progress work.
        // A bare `git stash` defaults to push, so the default is the risky arm.
        return match tokens.get(2).copied().unwrap_or("push") {
            "list" | "show" => Classification::safe(),
            _ => Classification::new(
                CommandRisk::Danger,
                "moves or discards working-tree changes (git stash)",
            ),
        };
    }
    if sub == "branch" {
        return if has_positional {
            Classification::new(CommandRisk::Caution, "creates or deletes a branch")
        } else {
            Classification::safe()
        };
    }
    if sub == "tag" {
        return if has_positional && !rest.contains(" -l") && !rest.contains("--list") {
            Classification::new(CommandRisk::Caution, "creates or deletes a tag")
        } else {
            Classification::safe()
        };
    }
    if sub == "config" {
        return if rest.contains("--get") || rest.contains("--list") || rest.contains(" -l") {
            Classification::safe()
        } else {
            Classification::new(CommandRisk::Caution, "writes git config")
        };
    }
    if sub == "push" && (rest.contains("--force") || rest.contains(" -f")) {
        return Classification::new(CommandRisk::Danger, "force-pushes to a remote");
    }
    if sub == "reset" && rest.contains("--hard") {
        return Classification::new(CommandRisk::Danger, "discards local changes (reset --hard)");
    }
    if sub == "clean" && (rest.contains("-f") || rest.contains("-d") || rest.contains("-x")) {
        return Classification::new(CommandRisk::Danger, "deletes untracked files (git clean)");
    }
    if (sub == "checkout" || sub == "restore") && rest.contains(' ') && !rest.contains("-b") {
        return Classification::new(CommandRisk::Danger, "discards changes (checkout/restore)");
    }
    if sub == "push" {
        return Classification::new(CommandRisk::Caution, "pushes to a remote");
    }
    if matches!(sub, "fetch" | "pull" | "clone" | "ls-remote") {
        return Classification::new(CommandRisk::Caution, "accesses a Git remote");
    }
    if GIT_READONLY.contains(&sub) {
        return Classification::safe();
    }
    if GIT_DESTRUCTIVE.contains(&sub) {
        return Classification::new(CommandRisk::Caution, "rewrites git state");
    }
    if sub == "commit" || sub == "add" || sub == "merge" || sub == "init" {
        return Classification::new(CommandRisk::Caution, "modifies the repository");
    }
    Classification::new(CommandRisk::Caution, "git command")
}

fn classify_cargo(tokens: &[&str]) -> Classification {
    let sub = tokens.get(1).map(|s| s.to_lowercase()).unwrap_or_default();
    match sub.as_str() {
        "publish" => Classification::new(CommandRisk::Danger, "publishes a crate"),
        "add" | "remove" | "rm" | "install" | "uninstall" | "update" => {
            Classification::new(CommandRisk::Caution, "changes dependencies")
        }
        _ => Classification::safe(), // build / test / check / run / fmt / clippy …
    }
}

fn classify_node(tokens: &[&str]) -> Classification {
    let sub = tokens.get(1).map(|s| s.to_lowercase()).unwrap_or_default();
    let rest = tokens.join(" ").to_lowercase();
    if rest.contains("publish") {
        return Classification::new(CommandRisk::Danger, "publishes a package");
    }
    if matches!(
        sub.as_str(),
        "install" | "i" | "add" | "ci" | "update" | "upgrade" | "remove" | "uninstall"
    ) {
        return Classification::new(CommandRisk::Caution, "changes dependencies");
    }
    // npm test / run build / run lint, etc.
    Classification::safe()
}

fn is_publish(prog: &str, lower: &str) -> bool {
    (prog == "cargo" && lower.contains("publish"))
        || (prog == "gh" && lower.contains("release"))
        || (prog == "twine")
        || (prog == "gem" && lower.contains("push"))
}

fn mentions(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

fn is_fork_bomb(lower: &str) -> bool {
    let squished: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    squished.contains(":(){:|:&};:") || squished.contains(":(){:|:&}")
}

/// Whether the command targets a filesystem/home root in a destructive way.
fn contains_root_target(lower: &str) -> bool {
    lower.split_whitespace().any(|t| {
        matches!(t, "/" | "/*" | "~" | "~/" | "/." | "$home" | "${home}")
            || t.starts_with("/*")
            || (t == "*")
    })
}

fn is_root_destroyer(lower: &str) -> bool {
    // rm with recursive+force (in any flag order, incl. combined like -rf) at a
    // root/home/wildcard target.
    let has_rm = lower.split_whitespace().any(|t| program(t) == "rm");
    if !has_rm {
        return false;
    }
    let recursive = lower.contains(" -r")
        || lower.contains(" -fr")
        || lower.contains(" -rf")
        || lower.contains("--recursive")
        || lower.contains(" -f ") && lower.contains(" -r");
    recursive && contains_root_target(lower)
}

/// Split a command line into logically separate commands on shell operators.
pub(crate) fn split_segments(command: &str) -> Vec<&str> {
    command
        .split([';', '\n'])
        .flat_map(|s| s.split("&&"))
        .flat_map(|s| s.split("||"))
        .flat_map(|s| s.split('|'))
        .flat_map(|s| s.split('&'))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Whether a command is expected to cross the local network boundary. Auto
/// mode uses this to surface a user approval before execution instead of
/// letting the OS sandbox turn the attempt into a misleading DNS failure.
pub(crate) fn command_requires_network(command: &str) -> bool {
    split_segments(command).iter().any(|segment| {
        let tokens = segment.split_whitespace().collect::<Vec<_>>();
        let Some(first) = tokens.first() else {
            return false;
        };
        let prog = program(first);
        let sub = tokens.get(1).map(|value| program(value)).unwrap_or("");
        match prog {
            "curl" | "wget" | "nc" | "ncat" | "ssh" | "scp" | "rsync" | "ftp" | "gh" => true,
            "git" => matches!(sub, "fetch" | "pull" | "push" | "clone" | "ls-remote"),
            "cargo" => matches!(sub, "add" | "install" | "publish" | "search" | "update"),
            "npm" | "pnpm" | "yarn" | "bun" => matches!(
                sub,
                "install"
                    | "i"
                    | "add"
                    | "ci"
                    | "update"
                    | "upgrade"
                    | "publish"
                    | "login"
                    | "whoami"
            ),
            "docker" | "podman" | "kubectl" | "terraform" => true,
            p if INSTALLERS.contains(&p) => true,
            _ => false,
        }
    })
}

/// Whether a command needs a one-call escape from the workspace sandbox after
/// the user approves it. Network access, Git metadata writes, privilege tools,
/// and host/infra controllers cannot succeed inside the normal sandbox. Other
/// approved mutations remain sandboxed to the project.
pub(crate) fn command_requires_host(command: &str) -> bool {
    command_requires_network(command)
        || split_segments(command).iter().any(|segment| {
            let tokens = segment.split_whitespace().collect::<Vec<_>>();
            let Some(first) = tokens.first() else {
                return false;
            };
            match program(first) {
                "sudo" | "su" | "doas" | "docker" | "podman" | "kubectl" | "terraform"
                | "systemctl" | "launchctl" | "service" => true,
                "git" => classify_git(&tokens).risk != CommandRisk::Safe,
                _ => false,
            }
        })
}

/// Programs from [`READONLY`] that only ever *inspect* — the strict subset a
/// read-only session phase (Plan Mode) may run unprompted. Deliberately
/// excludes programs that are fine to auto-approve in normal modes but can
/// write files or run other commands: `tee`, `xargs`, `sed`/`awk` (`-i`,
/// `system()`), archivers, `env` (`env CMD` executes CMD), `find`/`fd`
/// (handled per-flag below).
const READONLY_INSPECT: &[&str] = &[
    // Changes only the shell process's working directory. Plan-mode commands
    // commonly prefix a batched inspection with `cd app && ...`; treating
    // that as a mutation creates a redundant permission prompt even though
    // every subsequent segment is independently checked below.
    "cd",
    "ls",
    "cat",
    "head",
    "tail",
    "pwd",
    "echo",
    "printf",
    "which",
    "type",
    "whoami",
    "id",
    "date",
    "printenv",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "ag",
    "wc",
    "sort",
    "uniq",
    "cut",
    "tr",
    "diff",
    "tree",
    "file",
    "stat",
    "du",
    "df",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "uname",
    "hostname",
    "ps",
    "top",
    "true",
    "false",
    "test",
    "column",
    "jq",
    "yq",
    "base64",
    "md5",
    "shasum",
    "sha256sum",
    "nproc",
    "sleep",
    "start-sleep",
    "clear",
];

/// Whether a whole command line is strictly read-only — safe to run while Plan
/// Mode holds the session read-only. Much stricter than `CommandRisk::Safe`
/// (which admits build tools and writers like `tee`): every segment must be a
/// pure inspection program, nothing may redirect output to a file, and nothing
/// may hide a command in substitution.
pub fn is_read_only_command(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Substitution can hide any command inside a benign-looking line, and
    // segments don't descend into it.
    if trimmed.contains("$(") || trimmed.contains('`') {
        return false;
    }
    // Output redirection writes files. Strip the harmless stderr-silencing
    // forms first, then reject any `>` that remains.
    let stripped = trimmed
        .replace("2>&1", "")
        .replace("2>/dev/null", "")
        .replace("2> /dev/null", "")
        .replace(">/dev/null", "")
        .replace("> /dev/null", "");
    if stripped.contains('>') {
        return false;
    }
    // Segment the stripped line: the removed `2>&1` would otherwise leave a
    // bogus `1` segment behind the `&` split.
    let segments = split_segments(&stripped);
    !segments.is_empty() && segments.iter().all(|seg| is_read_only_segment(seg))
}

fn is_read_only_segment(segment: &str) -> bool {
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    let Some(first) = tokens.first() else {
        return false;
    };
    let normalized_program = program(first).to_ascii_lowercase();
    match normalized_program.as_str() {
        // Reuses the listing-vs-mutating analysis (`status`/`log`/`diff`/
        // `branch` with no positional args… are Safe; anything that writes is
        // not).
        "git" => classify_git(&tokens).risk == CommandRisk::Safe,
        // `find`/`fd` are searches unless a flag deletes or executes.
        "find" => !tokens.iter().any(|t| {
            matches!(
                *t,
                "-delete"
                    | "-exec"
                    | "-execdir"
                    | "-ok"
                    | "-okdir"
                    | "-fprint"
                    | "-fprintf"
                    | "-fls"
            )
        }),
        "fd" => !tokens
            .iter()
            .any(|t| matches!(*t, "-x" | "-X" | "--exec" | "--exec-batch")),
        p => READONLY_INSPECT.contains(&p),
    }
}

#[cfg(test)]
#[path = "safety_tests.rs"]
mod tests;
