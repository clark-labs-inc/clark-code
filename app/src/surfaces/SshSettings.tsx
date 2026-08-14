import { useCallback, useEffect, useRef, useState } from "react";
import { productName } from "../product/productModule";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  AlertCircle,
  CheckCircle2,
  Circle,
  CircleDot,
  Folder,
  Loader2,
  RefreshCw,
  Server,
  Trash2,
  X,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import {
  blankHost,
  hostReady,
  loadSshHosts,
  saveSshHosts,
  type SshHost,
} from "../lib/sshHosts";
import {
  listSshConfigHosts,
  probeSsh,
  type SshConfigHost,
  type SshProbe,
} from "../lib/ssh";
import { codeKeyAccountBinding } from "../lib/account";
import { cn } from "../lib/cn";
import { DIALOG, OVERLAY, accessibleMotion } from "../lib/motion";
import { RemoteFolderBrowser } from "./EnvironmentPicker";

const input =
  "w-full rounded-lg border border-border bg-bg px-3 py-1.5 text-sm text-ink outline-none transition focus:border-accent focus:ring-2 focus:ring-accent-focus/30 placeholder:text-ink-muted";
const label = "mb-1.5 block text-xs font-medium text-ink-secondary";

type TestState = { loading: boolean; probe?: SshProbe; error?: string };
type SetupMode = "config" | "manual";

export function sshDialogKeyboardIntent(key: string): "close" | "cycle_focus" | "none" {
  if (key === "Escape") return "close";
  if (key === "Tab") return "cycle_focus";
  return "none";
}

export function sshConfigHostDetail(host: SshConfigHost): string {
  if (host.user && host.hostname) return `${host.user}@${host.hostname}`;
  if (host.hostname) return host.hostname;
  if (host.user) return `${host.user}@${host.alias}`;
  return "SSH config alias";
}

function configuredPreset(alias: string): SshHost {
  return { ...blankHost(), label: alias, host: alias };
}

function sameDestination(left: string, right: string): boolean {
  return left.trim().toLowerCase() === right.trim().toLowerCase();
}

function ModeChoice({
  active,
  title,
  description,
  onClick,
}: {
  active: boolean;
  title: string;
  description: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      className={cn(
        "min-w-0 flex-1 rounded-xl border px-4 py-1.5 text-left transition duration-base ease-agent",
        active
          ? "border-accent bg-accent-subtle text-accent"
          : "border-border bg-bg text-ink-secondary hover:border-border-strong hover:bg-bg-hover",
      )}
    >
      <span className="block text-sm font-semibold">{title}</span>
      <span className={cn("mt-0.5 block text-xs", active ? "text-accent" : "text-ink-muted")}>
        {description}
      </span>
    </button>
  );
}

function ConnectionResult({ test }: { test?: TestState }) {
  if (!test || test.loading) return null;
  if (test.probe) {
    return (
      <span className="flex min-w-0 items-center gap-1.5 text-xs text-success">
        <CheckCircle2 className="size-3.5 shrink-0" />
        Reachable
      </span>
    );
  }
  return (
    <span role="alert" className="flex min-w-0 items-center gap-1.5 text-xs text-danger">
      <AlertCircle className="size-3.5 shrink-0" />
      <span className="truncate">{test.error}</span>
    </span>
  );
}

export function SshSettings() {
  const open = useSessionStore((state) => state.sshOpen);
  const setOpen = useSessionStore((state) => state.setSshOpen);
  const selectedHostId = useSessionStore((state) => state.selectedHostId);
  const setSelectedHostId = useSessionStore((state) => state.setSelectedHostId);
  const auth = useSessionStore((state) => state.auth);
  const accountScope = codeKeyAccountBinding(auth);
  const reduce = useReducedMotion();
  const [mode, setMode] = useState<SetupMode>("config");
  const modeRef = useRef<SetupMode>("config");
  const [hosts, setHosts] = useState<SshHost[]>([]);
  const [savedHosts, setSavedHosts] = useState<SshHost[]>([]);
  const [activeHost, setActiveHost] = useState<SshHost | null>(null);
  const [tests, setTests] = useState<Record<string, TestState>>({});
  const [configHosts, setConfigHosts] = useState<SshConfigHost[]>([]);
  const [configLoading, setConfigLoading] = useState(false);
  const [folderBrowserOpen, setFolderBrowserOpen] = useState(false);
  const dialogRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const selectedConfigHostRef = useRef<HTMLButtonElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  const restoreFocus = useCallback(() => {
    const previous = previousFocusRef.current;
    previousFocusRef.current = null;
    if (previous?.isConnected) {
      previous.focus();
      return;
    }
    document.querySelector<HTMLElement>("[aria-label='Spec execution target'] button")?.focus();
  }, []);

  const close = useCallback(() => {
    setOpen(false);
    requestAnimationFrame(restoreFocus);
  }, [restoreFocus, setOpen]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    const loaded = loadSshHosts(accountScope);
    setHosts(loaded);
    setSavedHosts(loaded);
    setTests({});
    setConfigHosts([]);
    setConfigLoading(true);
    setFolderBrowserOpen(false);
    modeRef.current = "config";
    setMode("config");
    setActiveHost(null);

    void listSshConfigHosts()
      .then((configured) => {
        if (cancelled) return;
        setConfigHosts(configured);
        if (configured.length > 0 && modeRef.current === "config") {
          const first = configured[0];
          const saved = loaded.find((host) => sameDestination(host.host, first.alias));
          setActiveHost(saved ?? configuredPreset(first.alias));
        } else if (configured.length === 0 && modeRef.current === "config") {
          modeRef.current = "manual";
          setMode("manual");
          setActiveHost(loaded[0] ?? blankHost());
        }
      })
      .catch(() => {
        if (cancelled) return;
        setConfigHosts([]);
        modeRef.current = "manual";
        setMode("manual");
        setActiveHost(loaded[0] ?? blankHost());
      })
      .finally(() => {
        if (!cancelled) setConfigLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [accountScope, open]);

  useEffect(() => {
    if (!open) return;
    previousFocusRef.current = document.activeElement instanceof HTMLElement
      && document.activeElement !== document.body
      ? document.activeElement
      : null;
    const bodyOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const focusFrame = requestAnimationFrame(() => closeButtonRef.current?.focus());
    const onKeyDown = (event: KeyboardEvent) => {
      const intent = sshDialogKeyboardIntent(event.key);
      if (intent === "close") {
        event.preventDefault();
        event.stopPropagation();
        close();
        return;
      }
      if (intent !== "cycle_focus" || !dialogRef.current) return;
      const focusable = Array.from(dialogRef.current.querySelectorAll<HTMLElement>(
        "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex='-1'])",
      ));
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (event.shiftKey && (active === first || !dialogRef.current.contains(active))) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (active === last || !dialogRef.current.contains(active))) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      cancelAnimationFrame(focusFrame);
      document.body.style.overflow = bodyOverflow;
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [close, open]);

  useEffect(() => {
    if (!open || mode !== "config" || !activeHost) return;
    const focusFrame = requestAnimationFrame(() => selectedConfigHostRef.current?.focus());
    return () => cancelAnimationFrame(focusFrame);
  }, [activeHost, mode, open]);

  const configuredAliases = new Set(configHosts.map((host) => host.alias.toLowerCase()));
  const manualHosts = hosts.filter((host) => !configuredAliases.has(host.host.trim().toLowerCase()));
  const savedMatch = activeHost
    ? hosts.find((host) => host.id === activeHost.id)
      ?? hosts.find((host) => sameDestination(host.host, activeHost.host))
    : null;
  const stagedChanges = JSON.stringify(hosts) !== JSON.stringify(savedHosts);
  const activeChanged = Boolean(activeHost && (
    !savedMatch || JSON.stringify({ ...activeHost, id: savedMatch.id }) !== JSON.stringify(savedMatch)
  ));
  const dirty = stagedChanges || activeChanged;
  const canCommit = Boolean(activeHost && hostReady(activeHost)) || stagedChanges;
  const primaryLabel = savedMatch || stagedChanges ? "Save changes" : "Add remote host";

  const chooseMode = (nextMode: SetupMode) => {
    modeRef.current = nextMode;
    setMode(nextMode);
    setFolderBrowserOpen(false);
    requestAnimationFrame(() => contentRef.current?.scrollTo({ top: 0 }));
    if (nextMode === "config") {
      const configured = configHosts[0];
      if (!configured) {
        setActiveHost(null);
        return;
      }
      const saved = hosts.find((host) => sameDestination(host.host, configured.alias));
      setActiveHost(saved ?? configuredPreset(configured.alias));
      return;
    }
    setActiveHost(manualHosts[0] ?? blankHost());
  };

  const chooseConfigured = (configured: SshConfigHost) => {
    const saved = hosts.find((host) => sameDestination(host.host, configured.alias));
    setActiveHost(saved ?? configuredPreset(configured.alias));
    setFolderBrowserOpen(false);
  };

  const chooseManual = (id: string) => {
    if (id === "new") {
      setActiveHost(blankHost());
      setFolderBrowserOpen(false);
      return;
    }
    setActiveHost(hosts.find((host) => host.id === id) ?? blankHost());
    setFolderBrowserOpen(false);
  };

  const removeActive = () => {
    if (!savedMatch) return;
    const remaining = hosts.filter((host) => host.id !== savedMatch.id);
    setHosts(remaining);
    if (mode === "config") {
      const next = configHosts.find((host) => !sameDestination(host.alias, savedMatch.host));
      const saved = next && remaining.find((host) => sameDestination(host.host, next.alias));
      setActiveHost(next ? saved ?? configuredPreset(next.alias) : null);
    } else {
      const next = remaining.find((host) => !configuredAliases.has(host.host.trim().toLowerCase()));
      setActiveHost(next ?? blankHost());
    }
  };

  const test = async (host: SshHost) => {
    setTests((current) => ({ ...current, [host.id]: { loading: true } }));
    try {
      const probe = await probeSsh(host.host.trim());
      setTests((current) => ({ ...current, [host.id]: { loading: false, probe } }));
    } catch (error) {
      setTests((current) => ({
        ...current,
        [host.id]: { loading: false, error: String(error) },
      }));
    }
  };

  const save = () => {
    let next = hosts;
    let committedId = selectedHostId;
    if (activeHost && hostReady(activeHost)) {
      const match = savedMatch;
      const committed = match ? { ...activeHost, id: match.id } : activeHost;
      next = match
        ? hosts.map((host) => host.id === match.id ? committed : host)
        : [...hosts, committed];
      committedId ??= committed.id;
    }
    saveSshHosts(next, accountScope);
    setSavedHosts(next);
    setSelectedHostId(
      committedId && next.some((host) => host.id === committedId) ? committedId : next[0]?.id ?? null,
    );
    close();
  };

  const activeTest = activeHost ? tests[activeHost.id] : undefined;

  return (
    <AnimatePresence>
      {open && (
        <m.div
          {...accessibleMotion(OVERLAY, reduce)}
          className="fixed inset-0 z-50 grid place-items-center bg-black/40 p-6"
          onClick={close}
        >
          <m.div
            ref={dialogRef}
            {...accessibleMotion(DIALOG, reduce)}
            role="dialog"
            aria-modal="true"
            aria-labelledby="ssh-settings-title"
            onClick={(event) => event.stopPropagation()}
            className="popover-surface flex max-h-[88vh] w-full max-w-2xl flex-col rounded-2xl border border-border bg-bg-elevated shadow-2xl"
          >
            <div className="flex items-center gap-2 border-b border-border-subtle px-5 py-2.5">
              <Server className="size-4 text-ink-secondary" />
              <h2 id="ssh-settings-title" className="text-sm font-semibold text-ink">
                Remote hosts
              </h2>
              <span className="text-xs text-ink-muted">Run {productName()} on a machine over SSH</span>
              <button
                ref={closeButtonRef}
                type="button"
                onClick={close}
                aria-label="Close"
                className="ml-auto grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink"
              >
                <X className="size-4" />
              </button>
            </div>

            <div ref={contentRef} className="min-h-0 flex-1 overflow-y-auto px-5 pb-0 pt-3">
              <div role="tablist" aria-label="Host setup method" className="mb-3 flex max-w-lg gap-2">
                <ModeChoice
                  active={mode === "config"}
                  title="SSH config"
                  description="Use hosts from ~/.ssh/config"
                  onClick={() => chooseMode("config")}
                />
                <ModeChoice
                  active={mode === "manual"}
                  title="Manual"
                  description="Enter host details manually"
                  onClick={() => chooseMode("manual")}
                />
              </div>

              <section aria-labelledby="ssh-machine-step">
                <h3 id="ssh-machine-step" className="mb-2 text-sm font-semibold text-ink">
                  1. {mode === "config" ? "Choose a machine" : "Enter host details"}
                </h3>

                {mode === "config" ? (
                  <div>
                    <div className="mb-2 flex items-center justify-between">
                      <span className="text-xs font-medium text-ink-secondary">
                        Hosts from <span className="font-mono">~/.ssh/config</span>
                      </span>
                      {!configLoading && (
                        <span className="text-xs text-ink-faint">
                          {configHosts.length} found
                        </span>
                      )}
                    </div>
                    {configLoading ? (
                      <div className="flex min-h-24 items-center justify-center gap-2 rounded-xl border border-border-subtle text-sm text-ink-muted">
                        <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" /> Reading SSH config…
                      </div>
                    ) : configHosts.length > 0 ? (
                      <div role="radiogroup" aria-label="SSH config hosts" className="max-h-40 overflow-y-auto rounded-xl border border-border">
                        {configHosts.map((configured, index) => {
                          const selected = Boolean(activeHost && sameDestination(activeHost.host, configured.alias));
                          return (
                            <button
                              key={configured.alias}
                              ref={selected ? selectedConfigHostRef : undefined}
                              type="button"
                              role="radio"
                              aria-checked={selected}
                              onClick={() => chooseConfigured(configured)}
                              className={cn(
                                "grid min-h-9 w-full grid-cols-[1.5rem_minmax(0,1fr)_minmax(0,1fr)] items-center gap-2 px-3 text-left transition duration-base ease-agent",
                                index > 0 && "border-t border-border-subtle",
                                "focus-visible:outline-none",
                                selected
                                  ? "bg-accent-subtle ring-1 ring-inset ring-accent/50"
                                  : "hover:bg-bg-hover",
                              )}
                            >
                              {selected
                                ? <CircleDot className="size-4 text-accent" />
                                : <Circle className="size-4 text-ink-faint" />}
                              <span className="truncate font-mono text-sm font-medium text-ink">
                                {configured.alias}
                              </span>
                              <span className="truncate font-mono text-xs text-ink-faint">
                                {sshConfigHostDetail(configured)}
                              </span>
                            </button>
                          );
                        })}
                      </div>
                    ) : (
                      <div className="rounded-xl border border-border-subtle px-4 py-4 text-sm text-ink-muted">
                        No named hosts were found. Add a <span className="font-mono">Host</span> entry
                        to your SSH config or use manual setup.
                      </div>
                    )}
                  </div>
                ) : (
                  <div className="space-y-3">
                    {manualHosts.length > 0 && (
                      <div>
                        <label htmlFor="saved-manual-host" className={label}>Saved manual host</label>
                        <select
                          id="saved-manual-host"
                          value={savedMatch?.id ?? "new"}
                          onChange={(event) => chooseManual(event.target.value)}
                          className={input}
                        >
                          {manualHosts.map((host) => (
                            <option key={host.id} value={host.id}>
                              {host.label.trim() || host.host.trim()}
                            </option>
                          ))}
                          <option value="new">Add another host…</option>
                        </select>
                      </div>
                    )}
                    <div className="grid grid-cols-2 gap-3">
                      <div>
                        <label htmlFor="manual-ssh-host" className={label}>SSH destination</label>
                        <input
                          id="manual-ssh-host"
                          value={activeHost?.host ?? ""}
                          onChange={(event) => setActiveHost((current) => ({
                            ...(current ?? blankHost()),
                            host: event.target.value,
                          }))}
                          placeholder="user@host"
                          className={cn(input, "font-mono")}
                          autoCorrect="off"
                          autoCapitalize="off"
                          spellCheck={false}
                        />
                      </div>
                      <div>
                        <label htmlFor="manual-host-label" className={label}>Display name</label>
                        <input
                          id="manual-host-label"
                          value={activeHost?.label ?? ""}
                          onChange={(event) => setActiveHost((current) => ({
                            ...(current ?? blankHost()),
                            label: event.target.value,
                          }))}
                          placeholder="GPU box"
                          className={input}
                        />
                      </div>
                    </div>
                  </div>
                )}
              </section>

              <div className="my-2.5 border-t border-border-subtle" />

              <section aria-labelledby="ssh-folder-step" className={cn(!activeHost && "opacity-55")}>
                <h3 id="ssh-folder-step" className="mb-2 text-sm font-semibold text-ink">
                  2. Project folder{activeHost?.host.trim() ? ` on ${activeHost.host.trim()}` : ""}
                </h3>
                <div className="relative">
                  <label htmlFor="remote-project-folder" className="sr-only">Remote project folder</label>
                  <input
                    id="remote-project-folder"
                    value={activeHost?.remoteRoot ?? ""}
                    onChange={(event) => setActiveHost((current) => current
                      ? { ...current, remoteRoot: event.target.value }
                      : current)}
                    placeholder="/home/you/project"
                    disabled={!activeHost}
                    className={cn(input, "pr-10 font-mono disabled:cursor-not-allowed")}
                    autoCorrect="off"
                    autoCapitalize="off"
                    spellCheck={false}
                  />
                  <button
                    type="button"
                    aria-label="Browse remote folders"
                    aria-expanded={folderBrowserOpen}
                    onClick={() => setFolderBrowserOpen((current) => !current)}
                    disabled={!activeHost?.host.trim()}
                    className="absolute inset-y-px right-px grid w-9 place-items-center rounded-r-lg border-l border-border-subtle text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    <Folder className="size-4" />
                  </button>
                  {folderBrowserOpen && activeHost && (
                    <div
                      role="dialog"
                      aria-label={`Remote folders on ${activeHost.label.trim() || activeHost.host.trim()}`}
                      className="popover-surface absolute right-0 top-full z-20 mt-2 rounded-2xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
                    >
                      <RemoteFolderBrowser
                        host={activeHost}
                        onSelect={(path) => {
                          setActiveHost((current) => current ? { ...current, remoteRoot: path } : current);
                          setFolderBrowserOpen(false);
                        }}
                        onManage={() => setFolderBrowserOpen(false)}
                      />
                    </div>
                  )}
                </div>
                <div className="mt-2 flex min-h-8 items-center gap-3">
                  <button
                    type="button"
                    onClick={() => activeHost && void test(activeHost)}
                    disabled={!activeHost?.host.trim() || activeTest?.loading}
                    className="flex min-h-8 items-center gap-1.5 rounded-lg border border-border px-3 text-xs font-medium text-ink-secondary transition hover:bg-bg-hover hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {activeTest?.loading
                      ? <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />
                      : <RefreshCw className="size-3.5" />}
                    Test connection
                  </button>
                  <ConnectionResult test={activeTest} />
                  {savedMatch && (
                    <button
                      type="button"
                      onClick={removeActive}
                      className="ml-auto flex min-h-8 items-center gap-1.5 rounded-lg px-2 text-xs font-medium text-ink-muted transition hover:bg-danger/10 hover:text-danger"
                    >
                      <Trash2 className="size-3.5" /> Remove
                    </button>
                  )}
                </div>
              </section>
            </div>

            <div className="flex items-center gap-2 px-5 py-1.5">
              <span className="text-xs text-ink-faint">
                {hosts.length} saved host{hosts.length === 1 ? "" : "s"}{dirty ? " · unsaved changes" : ""}
              </span>
              <button
                type="button"
                onClick={close}
                className="ml-auto min-h-8 rounded-lg px-3 text-sm font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={save}
                disabled={!canCommit}
                className="min-h-8 rounded-lg bg-accent px-4 text-sm font-semibold text-on-accent transition duration-base ease-agent hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-50"
              >
                {primaryLabel}
              </button>
            </div>
          </m.div>
        </m.div>
      )}
    </AnimatePresence>
  );
}
