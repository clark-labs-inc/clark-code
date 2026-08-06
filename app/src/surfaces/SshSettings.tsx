import { useEffect, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { Server, Trash2, X, Loader2, CheckCircle2, AlertCircle, Plus } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import {
  loadSshHosts,
  saveSshHosts,
  blankHost,
  newlyAddedSshHostId,
  type SshHost,
} from "../lib/sshHosts";
import { probeSsh, type SshProbe } from "../lib/ssh";
import { codeKeyAccountBinding } from "../lib/account";
import { cn } from "../lib/cn";
import { DIALOG, OVERLAY, accessibleMotion } from "../lib/motion";

const input =
  "w-full rounded-lg border border-border bg-bg px-2.5 py-1.5 text-sm text-ink outline-none transition focus:border-accent placeholder:text-ink-muted";
const label = "mb-1 block text-xs font-medium text-ink-secondary";

type TestState = { loading: boolean; probe?: SshProbe; error?: string };

function HostCard({
  host,
  test,
  onChange,
  onRemove,
  onTest,
}: {
  host: SshHost;
  test?: TestState;
  onChange: (h: SshHost) => void;
  onRemove: () => void;
  onTest: () => void;
}) {
  return (
    <div className="rounded-xl border border-border-subtle bg-bg-elevated/40 p-3">
      <div className="mb-2.5 flex items-center gap-2">
        <input
          value={host.label}
          onChange={(e) => onChange({ ...host, label: e.target.value })}
          placeholder="label (e.g. gpu box)"
          className={cn(input, "flex-1 font-medium")}
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
        />
        <button
          onClick={onTest}
          disabled={!host.host.trim() || test?.loading}
          className="flex items-center gap-1 rounded-md bg-bg-tertiary px-2 py-1 text-xs font-medium text-ink-secondary transition hover:bg-bg-hover disabled:opacity-50"
        >
          {test?.loading && <Loader2 className="size-3 animate-[spin_1s_linear_infinite]" />}
          Test
        </button>
        <button
          onClick={onRemove}
          aria-label="Remove host"
          className="grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-danger/15 hover:text-danger"
        >
          <Trash2 className="size-3.5" />
        </button>
      </div>

      <div className="grid grid-cols-[1fr_1fr] gap-2">
        <div>
          <label className={label}>SSH host</label>
          <input
            value={host.host}
            onChange={(e) => onChange({ ...host, host: e.target.value })}
            placeholder="alias or user@host"
            className={cn(input, "font-mono text-xs")}
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
          />
        </div>
        <div>
          <label className={label}>Remote project folder</label>
          <input
            value={host.remoteRoot}
            onChange={(e) => onChange({ ...host, remoteRoot: e.target.value })}
            placeholder="/home/you/project"
            className={cn(input, "font-mono text-xs")}
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
          />
        </div>
      </div>

      {test && !test.loading && (test.probe || test.error) && (
        <div
          className={cn(
            "mt-2.5 flex items-start gap-2 rounded-lg px-2.5 py-1.5 text-xs",
            test.probe ? "bg-success/10 text-ink-secondary" : "bg-danger/10 text-ink-secondary",
          )}
        >
          {test.probe ? (
            <CheckCircle2 className="mt-0.5 size-3.5 shrink-0 text-success" />
          ) : (
            <AlertCircle className="mt-0.5 size-3.5 shrink-0 text-danger" />
          )}
          <span className="min-w-0">
            {test.probe ? (
              <>
                Reachable · <span className="font-mono text-ink-faint">{test.probe.arch}</span> · home{" "}
                <span className="font-mono text-ink-faint">{test.probe.home}</span>
              </>
            ) : (
              <span className="text-danger">{test.error}</span>
            )}
          </span>
        </div>
      )}
    </div>
  );
}

export function SshSettings() {
  const open = useSessionStore((s) => s.sshOpen);
  const setOpen = useSessionStore((s) => s.setSshOpen);
  const session = useSessionStore((s) => s.session);
  const selectProvider = useSessionStore((s) => s.selectProvider);
  const setProjectMode = useSessionStore((s) => s.setProjectMode);
  const setSelectedHostId = useSessionStore((s) => s.setSelectedHostId);
  const auth = useSessionStore((s) => s.auth);
  const accountScope = codeKeyAccountBinding(auth);
  // Instant, no opacity fade under Reduced Motion — see Settings for why.
  const reduce = useReducedMotion();
  const [hosts, setHosts] = useState<SshHost[]>([]);
  const [savedHosts, setSavedHosts] = useState<SshHost[]>([]);
  const [tests, setTests] = useState<Record<string, TestState>>({});

  useEffect(() => {
    if (open) {
      const loaded = loadSshHosts(accountScope);
      setHosts(loaded);
      setSavedHosts(loaded);
    }
  }, [accountScope, open]);

  const update = (id: string, h: SshHost) =>
    setHosts((current) => current.map((x) => (x.id === id ? h : x)));
  const remove = (id: string) => setHosts((current) => current.filter((x) => x.id !== id));
  const add = () => setHosts((current) => [...current, blankHost()]);
  const dirty = JSON.stringify(hosts) !== JSON.stringify(savedHosts);
  const close = () => setOpen(false);
  const save = () => {
    const addedHostId = newlyAddedSshHostId(hosts, savedHosts);
    saveSshHosts(hosts, accountScope);
    setSavedHosts(hosts);
    if (addedHostId && !session) {
      selectProvider("local");
      setProjectMode("remote");
      setSelectedHostId(addedHostId);
    }
    setOpen(false);
  };

  const test = async (h: SshHost) => {
    setTests((t) => ({ ...t, [h.id]: { loading: true } }));
    try {
      const probe = await probeSsh(h.host.trim());
      setTests((t) => ({ ...t, [h.id]: { loading: false, probe } }));
    } catch (e) {
      setTests((t) => ({ ...t, [h.id]: { loading: false, error: String(e) } }));
    }
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
            aria-labelledby="ssh-settings-title"
            onClick={(e) => e.stopPropagation()}
            className="popover-surface flex max-h-[80vh] w-full max-w-2xl flex-col rounded-2xl border border-border bg-bg-elevated shadow-2xl"
          >
            <div className="flex items-center gap-2 border-b border-border-subtle px-4 py-3">
              <Server className="size-4 text-ink-secondary" />
              <h2 id="ssh-settings-title" className="text-sm font-semibold text-ink">
                Remote hosts
              </h2>
              <span className="text-xs text-ink-muted">Run Clark Code on a machine over SSH</span>
              <button
                onClick={close}
                aria-label="Close"
                className="ml-auto grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
              >
                <X className="size-4" />
              </button>
            </div>

            <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-4">
              {hosts.length === 0 && (
                <p className="px-1 pb-1 text-sm text-ink-muted">
                  Add a host to run the agent on a remote machine. Your tools and terminal execute
                  there; the model and approval gate stay here. Auth uses your own SSH — keys, agent,
                  and <span className="font-mono text-ink-faint">~/.ssh/config</span> — nothing is
                  stored.
                </p>
              )}
              {hosts.map((h) => (
                <HostCard
                  key={h.id}
                  host={h}
                  test={tests[h.id]}
                  onChange={(next) => update(h.id, next)}
                  onRemove={() => remove(h.id)}
                  onTest={() => void test(h)}
                />
              ))}
              <button
                onClick={add}
                className="flex w-full items-center justify-center gap-1.5 rounded-lg border border-dashed border-border py-2 text-sm font-medium text-ink-muted transition hover:border-accent hover:text-ink"
              >
                <Plus className="size-3.5" /> Add host
              </button>
            </div>

            <div className="flex items-center gap-2 border-t border-border-subtle px-4 py-3">
              <span className="text-xs text-ink-faint">
                {hosts.length > 0
                  ? `${hosts.length} host${hosts.length === 1 ? "" : "s"}${dirty ? " · unsaved changes" : " · saved on this device"}`
                  : "Saved on this device · no credentials stored"}
              </span>
              <button
                type="button"
                onClick={close}
                className="ml-auto min-h-8 rounded-lg px-3 py-1.5 text-sm font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={save}
                className="min-h-8 rounded-lg bg-accent px-3 py-1.5 text-sm font-semibold text-on-accent transition duration-200 ease-clark hover:bg-accent-hover"
              >
                Save
              </button>
            </div>
          </m.div>
        </m.div>
      )}
    </AnimatePresence>
  );
}
