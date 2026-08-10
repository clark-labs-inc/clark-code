import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  Laptop, Server, Folder, FolderOpen, Check, Plus, GitBranch, GitFork,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { pickFolder, inTauri } from "../lib/pickFolder";
import {
  loadSshHosts,
  saveSshHosts,
  hostLabel,
  hostReady,
  type SshHost,
} from "../lib/sshHosts";
import { codeKeyAccountBinding } from "../lib/account";
import { cn } from "../lib/cn";
import { DIALOG, OVERLAY, accessibleMotion } from "../lib/motion";
import { RemoteFolderBrowser } from "./EnvironmentPicker";
import type { ManagedWorktreeBase } from "../core-bridge/bridge";
import { loadManagedWorktreeBase } from "../lib/managedWorktreeSettings";

const input =
  "w-full rounded-lg border border-border bg-bg px-2.5 py-1.5 text-sm text-ink outline-none transition focus:border-accent placeholder:text-ink-muted";
const CHIP =
  "flex min-h-9 flex-1 items-center justify-center gap-2 rounded-xl px-3 py-1.5 text-sm font-medium transition duration-200 ease-agent";

/** The "New project…" chooser: pick where the new project runs (this machine,
 *  or a remote SSH host) and auto-start its first session. Starting a session
 *  immediately is the point — the folder/host step alone is not the goal. */
export function NewProjectDialog() {
  const open = useSessionStore((s) => s.newProjectOpen);
  const setOpen = useSessionStore((s) => s.setNewProjectOpen);
  const auth = useSessionStore((s) => s.auth);
  const accountScope = codeKeyAccountBinding(auth);
  const localCwd = useSessionStore((s) => s.localSettings.cwd);
  const recentProjects = useSessionStore((s) => s.recentProjects);
  const startNewProject = useSessionStore((s) => s.startNewProject);
  const setProjectMode = useSessionStore((s) => s.setProjectMode);
  const setSelectedHostId = useSessionStore((s) => s.setSelectedHostId);
  const setSshOpen = useSessionStore((s) => s.setSshOpen);
  const sshOpen = useSessionStore((s) => s.sshOpen);
  const initialHostId = useSessionStore((s) => s.selectedHostId);
  const reduce = useReducedMotion();

  const [mode, setMode] = useState<"local" | "remote">("local");
  const [localPath, setLocalPath] = useState(localCwd);
  const [localBase, setLocalBase] = useState<ManagedWorktreeBase>("current");
  const [hosts, setHosts] = useState<SshHost[]>(() => loadSshHosts(accountScope));
  const [selectedHostId, setSelectedHostIdLocal] = useState<string | null>(null);
  const [remoteRoot, setRemoteRoot] = useState("");
  const [pickerError, setPickerError] = useState<string | null>(null);

  // Refresh hosts when the manage-hosts modal closes (a host may have been
  // added) exactly like EnvironmentPicker does.
  useEffect(() => {
    if (open && !sshOpen) {
      const loaded = loadSshHosts(accountScope);
      setHosts(loaded);
      const stillThere = loaded.some((h) => h.id === selectedHostId);
      const nextId = stillThere ? selectedHostId : loaded.find(hostReady)?.id ?? loaded[0]?.id ?? null;
      setSelectedHostIdLocal(nextId);
      setRemoteRoot((current) => {
        const host = loaded.find((h) => h.id === nextId);
        return current && host && host.remoteRoot === current ? current : host?.remoteRoot ?? "";
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountScope, open, sshOpen]);

  // Seed from the current environment the first time the dialog opens.
  useEffect(() => {
    if (!open) return;
    setPickerError(null);
    setLocalPath((current) => current || localCwd || "");
    const loaded = loadSshHosts(accountScope);
    setHosts(loaded);
    const nextId =
      loaded.some((h) => h.id === initialHostId) ? initialHostId
        : loaded.find(hostReady)?.id ?? loaded[0]?.id ?? null;
    setSelectedHostIdLocal(nextId);
    setRemoteRoot(loaded.find((h) => h.id === nextId)?.remoteRoot ?? "");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  useEffect(() => {
    if (open) setLocalBase(loadManagedWorktreeBase(accountScope, localPath));
  }, [accountScope, localPath, open]);

  const selectedHost = useMemo(
    () => hosts.find((h) => h.id === selectedHostId) ?? null,
    [hosts, selectedHostId],
  );

  const close = () => setOpen(false);

  const chooseLocal = async () => {
    setPickerError(null);
    try {
      const picked = await pickFolder(localPath || undefined);
      if (picked) setLocalPath(picked);
    } catch (error) {
      setPickerError(`Could not open the folder picker: ${String(error)}`);
    }
  };

  const pickHost = (id: string) => {
    setSelectedHostIdLocal(id);
    setRemoteRoot(hosts.find((h) => h.id === id)?.remoteRoot ?? "");
  };

  const localReady = localPath.trim().length > 0;
  const remoteReady = Boolean(selectedHost && hostReady({ ...selectedHost, remoteRoot }));

  const start = async () => {
    if (!open) return;
    if (mode === "local") {
      if (!localReady) return;
      await startNewProject({ kind: "local", path: localPath.trim(), base: localBase });
    } else {
      if (!selectedHost || !remoteReady) return;
      const host = { ...selectedHost, remoteRoot: remoteRoot.trim() };
      // Keep the UI + next-session selection in sync for when this dialog is
      // opened again.
      setProjectMode("remote");
      setSelectedHostId(host.id);
      saveSshHosts(
        hosts.map((h) => h.id === host.id ? host : h),
        accountScope,
      );
      await startNewProject({ kind: "remote", host });
    }
    close();
  };

  return (
    <AnimatePresence>
      {open && (
        <m.div
          {...accessibleMotion(OVERLAY, reduce)}
          className="fixed inset-0 z-50 grid place-items-center bg-black/40 p-6"
          onClick={close}
        >
          <m.div
            {...accessibleMotion(DIALOG, reduce)}
            role="dialog"
            aria-modal="true"
            aria-labelledby="new-project-title"
            onClick={(e) => e.stopPropagation()}
            className="popover-surface flex max-h-[84vh] w-full max-w-xl flex-col rounded-2xl border border-border bg-bg-elevated shadow-2xl"
          >
            <div className="flex items-center justify-between border-b border-border-subtle px-5 py-4">
              <h2 id="new-project-title" className="text-base font-semibold text-ink">
                New project
              </h2>
            </div>

            <div className="flex min-h-0 flex-1 flex-col gap-4 px-5 py-4">
              {/* Target machine */}
              <div className="flex gap-1.5 rounded-xl bg-bg-secondary p-1">
                <button
                  type="button"
                  onClick={() => setMode("local")}
                  className={cn(
                    CHIP,
                    mode === "local"
                      ? "bg-bg-elevated text-ink shadow-lifted"
                      : "text-ink-muted hover:text-ink",
                  )}
                >
                  <Laptop className="size-4" /> This machine
                </button>
                <button
                  type="button"
                  onClick={() => setMode("remote")}
                  className={cn(
                    CHIP,
                    mode === "remote"
                      ? "bg-bg-elevated text-ink shadow-lifted"
                      : "text-ink-muted hover:text-ink",
                  )}
                >
                  <Server className="size-4" /> Remote over SSH
                </button>
              </div>

              {mode === "local" ? (
                <div className="flex flex-col gap-3">
                  <div className="rounded-xl border border-border-subtle bg-bg-secondary/55 p-2.5">
                    <div className="px-0.5 text-xs font-semibold text-ink-secondary">First session checkout</div>
                    <div className="mt-2 grid grid-cols-2 gap-1.5">
                      <button
                        type="button"
                        aria-pressed={localBase === "current"}
                        onClick={() => setLocalBase("current")}
                        className={cn(
                          "flex min-h-12 items-start gap-2 rounded-lg px-2.5 py-2 text-left text-xs transition hover:bg-bg-hover",
                          localBase === "current" && "bg-bg-elevated shadow-lifted",
                        )}
                      >
                        <GitBranch className="mt-0.5 size-3.5 shrink-0" />
                        <span><strong className="block font-medium">This checkout</strong><span className="mt-0.5 block text-ink-faint">Use this exact folder and revision.</span></span>
                      </button>
                      <button
                        type="button"
                        aria-pressed={localBase === "default"}
                        onClick={() => setLocalBase("default")}
                        className={cn(
                          "flex min-h-12 items-start gap-2 rounded-lg px-2.5 py-2 text-left text-xs transition hover:bg-bg-hover",
                          localBase === "default" && "bg-bg-elevated shadow-lifted",
                        )}
                      >
                        <GitFork className="mt-0.5 size-3.5 shrink-0" />
                        <span><strong className="block font-medium">Fresh default branch</strong><span className="mt-0.5 block text-ink-faint">Create an isolated sibling worktree.</span></span>
                      </button>
                    </div>
                  </div>
                  {inTauri() && (
                    <button
                      type="button"
                      onClick={() => void chooseLocal()}
                      className="flex min-h-10 w-full items-center gap-2 rounded-xl bg-accent px-2.5 py-2 text-sm font-medium text-on-accent transition duration-200 ease-agent hover:bg-accent-hover"
                    >
                      <FolderOpen className="size-4" /> Choose folder…
                    </button>
                  )}
                  <input
                    type="text"
                    value={localPath}
                    onChange={(e) => setLocalPath(e.target.value)}
                    placeholder={inTauri() ? "…or paste an absolute path" : "/Users/you/code/my-project"}
                    autoCorrect="off"
                    autoCapitalize="off"
                    spellCheck={false}
                    aria-label="Project folder path"
                    className={cn(input, "font-mono")}
                  />
                  {pickerError && (
                    <p role="alert" className="px-1 text-xs text-danger">
                      {pickerError} Paste an absolute path above to continue.
                    </p>
                  )}
                  {recentProjects.length > 0 && (
                    <div>
                      <div className="px-1 pb-1.5 text-xs font-semibold uppercase tracking-wider text-ink-faint">
                        Recent
                      </div>
                      <div className="flex flex-col gap-0.5">
                        {recentProjects.map((p) => (
                          <button
                            key={p}
                            type="button"
                            onClick={() => setLocalPath(p)}
                            className={cn(
                              "flex min-h-9 items-center gap-2.5 rounded-xl px-2 py-1.5 text-left transition duration-200 ease-agent hover:bg-accent-subtle",
                              p === localPath && "bg-accent-subtle",
                            )}
                          >
                            <Folder className="size-4 shrink-0 text-ink-muted" />
                            <span className="min-w-0 flex-1">
                              <span className="block truncate text-sm text-ink">{projectName(p)}</span>
                              <span className="block truncate text-xs text-ink-faint">{p}</span>
                            </span>
                            {p === localPath && <Check className="size-4 shrink-0 text-accent" />}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              ) : (
                <div className="flex min-h-0 flex-col gap-4">
                  {/* Host picker */}
                  <div>
                    <div className="mb-1.5 flex items-center justify-between px-1">
                      <span className="text-xs font-semibold uppercase tracking-wider text-ink-faint">
                        SSH host
                      </span>
                      <button
                        type="button"
                        onClick={() => setSshOpen(true)}
                        className="flex items-center gap-1 text-xs font-medium text-ink-muted transition hover:text-ink"
                      >
                        <Plus className="size-3.5" /> Add host…
                      </button>
                    </div>
                    {hosts.length === 0 ? (
                      <p className="px-1 text-sm text-ink-muted">
                        No hosts yet —{" "}
                        <button
                          type="button"
                          onClick={() => setSshOpen(true)}
                          className="font-medium text-accent hover:underline"
                        >
                          add an SSH host
                        </button>{" "}
                        to start a remote project.
                      </p>
                    ) : (
                      <div className="flex flex-col gap-0.5 rounded-xl bg-bg-secondary/55 p-1">
                        {hosts.map((h) => (
                          <button
                            key={h.id}
                            type="button"
                            onClick={() => pickHost(h.id)}
                            className={cn(
                              "flex min-h-9 items-center gap-2.5 rounded-xl px-2 py-1.5 text-left transition duration-200 ease-agent hover:bg-accent-subtle",
                              h.id === selectedHostId && "bg-accent-subtle",
                            )}
                          >
                            <Server className="size-4 shrink-0 text-ink-muted" />
                            <span className="min-w-0 flex-1">
                              <span className="block truncate text-sm text-ink">{hostLabel(h)}</span>
                              <span className="block truncate text-xs text-ink-faint">
                                {h.host.trim() || "needs host"}
                              </span>
                            </span>
                            {h.id === selectedHostId && <Check className="size-4 shrink-0 text-accent" />}
                          </button>
                        ))}
                      </div>
                    )}
                  </div>

                  {/* Remote folder browser */}
                  {selectedHost ? (
                    <RemoteFolderBrowser
                      host={{ ...selectedHost, remoteRoot }}
                      onSelect={(path) => setRemoteRoot(path)}
                      onManage={() => setSshOpen(true)}
                    />
                  ) : (
                    <div className="grid min-h-24 place-items-center rounded-xl border border-border-subtle text-sm text-ink-muted">
                      Select an SSH host to browse its folders.
                    </div>
                  )}
                </div>
              )}
            </div>

            <div className="flex items-center justify-end gap-2 border-t border-border-subtle px-5 py-3.5">
              <button
                type="button"
                onClick={close}
                className="rounded-lg px-3 py-1.5 text-sm font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => void start()}
                disabled={mode === "local" ? !localReady : !remoteReady}
                title={mode === "remote" && selectedHost && !hostReady({ ...selectedHost, remoteRoot })
                  ? "Choose a remote folder first."
                  : undefined}
                className="flex items-center gap-2 rounded-lg bg-accent px-4 py-1.5 text-sm font-semibold text-on-accent transition hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-50"
              >
                {mode === "remote" ? <Server className="size-4" /> : <Laptop className="size-4" />}
                Start session
              </button>
            </div>
          </m.div>
        </m.div>
      )}
    </AnimatePresence>
  );
}
