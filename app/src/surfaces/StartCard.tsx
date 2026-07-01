import { useEffect, useState, type ReactNode } from "react";
import { motion } from "motion/react";
import { ArrowRight, FolderOpen, Folder, Server, Settings2 } from "lucide-react";
import { ClarkMark } from "./ClarkMark";
import { useSessionStore } from "../store/sessionStore";
import { localSettingsReady, projectName, type LocalAgentSettings } from "../lib/localAgent";
import { loadSshHosts, hostLabel, hostReady, type SshHost } from "../lib/sshHosts";
import { inTauri } from "../lib/pickFolder";
import { useAppVersion } from "../lib/appInfo";

const SAMPLES = [
  "In one sentence, what is the Rust programming language?",
  "Create /home/user/workspace/notes.txt with three lines, then read it back and replace one word.",
  "Build a one-page website about cats and publish it. Give me the URL.",
];

const LOCAL_SAMPLES = [
  "Summarize what this project does from its README and top-level files.",
  "Find every TODO in the codebase and list them by file.",
  "Add a unit test for the function in the file I'm about to mention.",
];

export function StartCard() {
  const start = useSessionStore((s) => s.startSession);
  const connecting = useSessionStore((s) => s.connecting);
  const error = useSessionStore((s) => s.error);
  const providers = useSessionStore((s) => s.providers);
  const activeProvider = useSessionStore((s) => s.activeProvider);
  const selectProvider = useSessionStore((s) => s.selectProvider);
  const local = useSessionStore((s) => s.localSettings);
  const projectMode = useSessionStore((s) => s.projectMode);
  const setProjectMode = useSessionStore((s) => s.setProjectMode);
  const selectedHostId = useSessionStore((s) => s.selectedHostId);
  const sshOpen = useSessionStore((s) => s.sshOpen);
  const version = useAppVersion();

  const isLocal = activeProvider === "local";
  const isRemote = isLocal && projectMode === "remote";

  // Saved hosts live in localStorage; refresh when the manage-hosts modal closes.
  const [hosts, setHosts] = useState<SshHost[]>(() => loadSshHosts());
  useEffect(() => {
    if (!sshOpen) setHosts(loadSshHosts());
  }, [sshOpen]);
  const selectedHost = hosts.find((h) => h.id === selectedHostId) ?? null;

  const remoteBlocked = !selectedHost
    ? "Add a remote host."
    : !hostReady(selectedHost)
      ? "This host needs a folder and exec-server binary."
      : null;
  const blocked = isLocal ? (isRemote ? remoteBlocked : localSettingsReady(local)) : null;

  const startWith = async (q?: string) => {
    if (blocked) return;
    await start();
    if (q) await useSessionStore.getState().send(q);
  };

  const samples = isLocal ? LOCAL_SAMPLES : SAMPLES;

  return (
    <div className="flex flex-1 flex-col items-center justify-center overflow-y-auto p-6">
      <motion.div
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.25 }}
        className="flex w-full max-w-md flex-col"
      >
        <div className="mb-6 flex flex-col items-center text-center">
          <ClarkMark size={48} className="mb-3 rounded-2xl" />
          <h1 className="text-xl font-semibold tracking-tight text-ink">Start a session</h1>
          <p className="mt-1.5 max-w-sm text-sm text-ink-muted">
            {!isLocal
              ? "One window. Watch every step — files, web, and computer work — as it happens."
              : isRemote
                ? "Code on a remote machine over SSH — its files and shell, your model and approvals."
                : "Code on your machine — your files, your shell, the model runs on Clark."}
          </p>
        </div>

        {isLocal && (
          <Segmented
            className="mb-4"
            value={projectMode}
            onChange={(m) => setProjectMode(m as "local" | "remote")}
            options={[
              { value: "local", label: "Local" },
              { value: "remote", label: "Remote", icon: <Server className="size-3.5" /> },
            ]}
          />
        )}

        {providers.length > 1 && (
          <Segmented
            className="mb-4"
            value={activeProvider ?? ""}
            onChange={selectProvider}
            options={providers.map((p) => ({ value: p.id, label: p.label }))}
          />
        )}

        {isLocal &&
          (isRemote ? (
            <RemoteSettingsForm hosts={hosts} selected={selectedHost} />
          ) : (
            <LocalSettingsForm settings={local} />
          ))}

        <button
          onClick={() => void startWith()}
          disabled={connecting || !!blocked}
          className="mt-1 flex w-full items-center justify-center gap-2 rounded-xl bg-accent px-3 py-3 text-sm font-semibold text-on-accent transition hover:bg-accent-hover disabled:opacity-50"
        >
          {connecting ? "Connecting…" : "New session"}
          {!connecting && <ArrowRight className="size-4" />}
        </button>

        {blocked && <p className="mt-2 text-center text-xs text-ink-faint">{blocked}</p>}
        {error && <p className="mt-2 text-center text-xs text-danger">{error}</p>}

        <div className="mt-6">
          <p className="mb-1 px-1 text-[11px] font-medium uppercase tracking-wider text-ink-faint">
            Try
          </p>
          <div className="flex flex-col">
            {samples.map((s) => (
              <button
                key={s}
                onClick={() => void startWith(s)}
                disabled={connecting || !!blocked}
                className="group flex items-center gap-2 rounded-lg px-1 py-1.5 text-left text-sm text-ink-muted transition hover:text-ink-secondary disabled:opacity-50"
              >
                <ArrowRight className="size-3.5 shrink-0 text-ink-faint transition group-hover:translate-x-0.5 group-hover:text-ink-muted" />
                <span className="truncate">{s}</span>
              </button>
            ))}
          </div>
        </div>

        <div className="mt-6 flex items-center justify-between border-t border-border-subtle pt-3 text-[11px] text-ink-faint">
          <span>Clark Code</span>
          {version && <span className="tabular-nums">v{version}</span>}
        </div>
      </motion.div>
    </div>
  );
}

/** A pill segmented control (local/remote, provider switch). */
function Segmented({
  value,
  onChange,
  options,
  className,
}: {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string; icon?: ReactNode }[];
  className?: string;
}) {
  return (
    <div
      className={`flex gap-1 rounded-xl border border-border-subtle bg-bg-elevated/60 p-1 ${className ?? ""}`}
    >
      {options.map((o) => (
        <button
          key={o.value}
          onClick={() => onChange(o.value)}
          className={`flex flex-1 items-center justify-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium transition ${
            o.value === value
              ? "bg-accent text-on-accent"
              : "text-ink-secondary hover:bg-bg-hover"
          }`}
        >
          {o.icon}
          {o.label}
        </button>
      ))}
    </div>
  );
}

function LocalSettingsForm({ settings }: { settings: LocalAgentSettings }) {
  return (
    <div className="mb-1">
      <ProjectFolderField cwd={settings.cwd} />
      <p className="mt-2 px-1 text-xs text-ink-faint">
        Connected through your account — no API key. Coding runs on this machine; the model runs on
        Clark.
      </p>
    </div>
  );
}

function RemoteSettingsForm({ hosts, selected }: { hosts: SshHost[]; selected: SshHost | null }) {
  const setSelectedHostId = useSessionStore((s) => s.setSelectedHostId);
  const setSshOpen = useSessionStore((s) => s.setSshOpen);

  if (hosts.length === 0) {
    return (
      <div className="mb-1 flex flex-col items-center gap-2 rounded-xl border border-dashed border-border-subtle bg-bg-elevated/40 p-6 text-center">
        <Server className="size-5 text-ink-muted" />
        <p className="text-sm text-ink-muted">
          No remote hosts yet. Add one to run Clark Code on another machine over SSH.
        </p>
        <button
          onClick={() => setSshOpen(true)}
          className="mt-1 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-on-accent transition hover:bg-accent-hover"
        >
          Add a host
        </button>
      </div>
    );
  }

  return (
    <div className="mb-1 space-y-2">
      <div className="flex items-center justify-between px-1">
        <label className="text-xs font-medium text-ink-secondary">Remote host</label>
        <button
          onClick={() => setSshOpen(true)}
          className="flex items-center gap-1 text-xs text-ink-muted transition hover:text-ink"
        >
          <Settings2 className="size-3" /> Manage
        </button>
      </div>
      <select
        value={selected?.id ?? ""}
        onChange={(e) => setSelectedHostId(e.target.value)}
        className={inputCls}
      >
        {hosts.map((h) => (
          <option key={h.id} value={h.id}>
            {hostLabel(h)} — {h.host}
          </option>
        ))}
      </select>

      {selected && (
        <p className="flex items-center gap-1.5 truncate px-1 text-xs text-ink-muted">
          <Server className="size-3 shrink-0" />
          <span className="font-medium text-ink-secondary">{selected.host}</span>
          <span className="truncate font-mono">{selected.remoteRoot || "no folder set"}</span>
        </p>
      )}
      {selected && !hostReady(selected) && (
        <p className="px-1 text-xs text-warning">
          This host is missing its folder or exec-server binary —{" "}
          <button onClick={() => setSshOpen(true)} className="underline hover:text-ink">
            edit it
          </button>
          .
        </p>
      )}
    </div>
  );
}

function ProjectFolderField({ cwd }: { cwd: string }) {
  const pick = useSessionStore((s) => s.pickProjectFolder);
  const setProject = useSessionStore((s) => s.setProjectFolder);
  const setLocal = useSessionStore((s) => s.setLocalSettings);
  const recents = useSessionStore((s) => s.recentProjects);
  const tauri = inTauri();

  return (
    <div>
      <label className="mb-1 block px-1 text-xs font-medium text-ink-secondary">
        Project folder
      </label>
      <div className="flex items-stretch gap-2">
        {tauri && (
          <button
            type="button"
            onClick={() => void pick()}
            className="flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-3 py-2 text-sm font-medium text-on-accent transition hover:bg-accent-hover"
          >
            <FolderOpen className="size-4" /> Choose…
          </button>
        )}
        <input
          type="text"
          value={cwd}
          onChange={(e) => setLocal({ cwd: e.target.value })}
          placeholder={tauri ? "…or paste an absolute path" : "/Users/you/code/my-project"}
          spellCheck={false}
          className={`${inputCls} flex-1`}
        />
      </div>
      {cwd.trim() && (
        <p className="mt-1.5 flex items-center gap-1.5 truncate px-1 text-xs text-ink-muted">
          <Folder className="size-3 shrink-0" />
          <span className="font-medium text-ink-secondary">{projectName(cwd)}</span>
          <span className="truncate">{cwd}</span>
        </p>
      )}
      {recents.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {recents.map((p) => (
            <button
              key={p}
              type="button"
              title={p}
              onClick={() => setProject(p)}
              className={`flex max-w-[12rem] items-center gap-1 rounded-md border px-2 py-1 text-xs transition ${
                p === cwd
                  ? "border-accent bg-accent/10 text-ink"
                  : "border-border-subtle bg-bg-elevated/60 text-ink-secondary hover:bg-bg-hover"
              }`}
            >
              <Folder className="size-3 shrink-0" />
              <span className="truncate">{projectName(p)}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

const inputCls =
  "w-full rounded-lg border border-border bg-bg px-2.5 py-1.5 text-sm text-ink outline-none transition focus:border-accent placeholder:text-ink-muted";
