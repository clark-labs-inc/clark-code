import { useCallback, useEffect, useRef, useState } from "react";
import { productName } from "../product/productModule";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  Server,
  X,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import {
  blankHost,
  hostCanSave,
  loadSshHosts,
  saveSshHosts,
  type SshHost,
} from "../lib/sshHosts";
import {
  listSshConfigHosts,
  probeSsh,
  type SshConfigHost,
} from "../lib/ssh";
import { codeKeyAccountBinding } from "../lib/account";
import { DIALOG, OVERLAY, accessibleMotion } from "../lib/motion";
import type { SshOpenPurpose } from "../store/sessionStore.runtime";
import {
  sameDestination,
  SshSettingsForm,
  type SetupMode,
  type TestState,
} from "./SshSettingsForm";

export { sshConfigHostDetail } from "./SshSettingsForm";

export function sshDialogKeyboardIntent(key: string): "close" | "cycle_focus" | "none" {
  if (key === "Escape") return "close";
  if (key === "Tab") return "cycle_focus";
  return "none";
}

export function sshConfigSelectionKey(mode: SetupMode, host: SshHost | null): string | null {
  return mode === "config" ? host?.host.trim().toLowerCase() || null : null;
}

export function sshTargetAfterSave({
  purpose,
  selectedHostId,
  committedHostId,
  hosts,
}: {
  purpose: SshOpenPurpose;
  selectedHostId: string | null;
  committedHostId: string | null;
  hosts: SshHost[];
}): { selectedHostId: string | null; activateRemote: boolean } {
  const selectedStillExists = selectedHostId
    ? hosts.some((host) => host.id === selectedHostId)
    : false;
  const nextSelectedHostId = purpose === "select_execution_target" && committedHostId
    ? committedHostId
    : selectedStillExists
      ? selectedHostId
      : hosts[0]?.id ?? null;
  return {
    selectedHostId: nextSelectedHostId,
    activateRemote: purpose === "select_execution_target" && Boolean(committedHostId),
  };
}

function configuredPreset(alias: string): SshHost {
  return { ...blankHost(), label: alias, host: alias };
}

export function SshSettings() {
  const open = useSessionStore((state) => state.sshOpen);
  const setOpen = useSessionStore((state) => state.setSshOpen);
  const selectedHostId = useSessionStore((state) => state.selectedHostId);
  const setSelectedHostId = useSessionStore((state) => state.setSelectedHostId);
  const setProjectMode = useSessionStore((state) => state.setProjectMode);
  const purpose = useSessionStore((state) => state.sshOpenPurpose);
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
    document.querySelector<HTMLElement>("[aria-label$=' execution target'] button")?.focus();
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

  const configSelectionKey = sshConfigSelectionKey(mode, activeHost);
  useEffect(() => {
    if (!open || !configSelectionKey) return;
    const focusFrame = requestAnimationFrame(() => selectedConfigHostRef.current?.focus());
    return () => cancelAnimationFrame(focusFrame);
  }, [configSelectionKey, open]);

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
  const canCommit = Boolean(activeHost && hostCanSave(activeHost)) || stagedChanges;
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
    let committedHostId: string | null = null;
    if (activeHost && hostCanSave(activeHost)) {
      const match = savedMatch;
      const committed = match ? { ...activeHost, id: match.id } : activeHost;
      next = match
        ? hosts.map((host) => host.id === match.id ? committed : host)
        : [...hosts, committed];
      committedHostId = committed.id;
    }
    saveSshHosts(next, accountScope);
    setSavedHosts(next);
    const target = sshTargetAfterSave({
      purpose,
      selectedHostId,
      committedHostId,
      hosts: next,
    });
    setSelectedHostId(target.selectedHostId);
    if (target.activateRemote) setProjectMode("remote");
    close();
  };

  const activeTest = activeHost ? tests[activeHost.id] : undefined;

  return (
    <AnimatePresence>
      {open && (
        <m.div
          {...accessibleMotion(OVERLAY, reduce)}
          className="fixed inset-0 z-50 grid place-items-center bg-scrim p-6"
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
              <SshSettingsForm
                mode={mode}
                chooseMode={chooseMode}
                configHosts={configHosts}
                configLoading={configLoading}
                activeHost={activeHost}
                setActiveHost={setActiveHost}
                selectedConfigHostRef={selectedConfigHostRef}
                chooseConfigured={chooseConfigured}
                manualHosts={manualHosts}
                savedMatch={savedMatch ?? null}
                chooseManual={chooseManual}
                folderBrowserOpen={folderBrowserOpen}
                setFolderBrowserOpen={setFolderBrowserOpen}
                activeTest={activeTest}
                test={test}
                removeActive={removeActive}
              />
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
