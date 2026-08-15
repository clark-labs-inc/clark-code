import type { Dispatch, RefObject, SetStateAction } from "react";
import {
  AlertCircle,
  CheckCircle2,
  Circle,
  CircleDot,
  Folder,
  Loader2,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { blankHost, type SshHost } from "../lib/sshHosts";
import type { SshConfigHost, SshProbe } from "../lib/ssh";
import { cn } from "../lib/cn";
import { RemoteFolderBrowser } from "./EnvironmentPicker";

const input =
  "w-full rounded-lg border border-border bg-bg px-3 py-1.5 text-sm text-ink outline-none transition focus:border-accent focus:ring-2 focus:ring-accent-focus/30 placeholder:text-ink-muted";
const label = "mb-1.5 block text-xs font-medium text-ink-secondary";

export type TestState = { loading: boolean; probe?: SshProbe; error?: string };
export type SetupMode = "config" | "manual";

export function sshConfigHostDetail(host: SshConfigHost): string {
  if (host.user && host.hostname) return `${host.user}@${host.hostname}`;
  if (host.hostname) return host.hostname;
  if (host.user) return `${host.user}@${host.alias}`;
  return "SSH config alias";
}

export function sameDestination(left: string, right: string): boolean {
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

type Props = {
  mode: SetupMode;
  chooseMode: (mode: SetupMode) => void;
  configHosts: SshConfigHost[];
  configLoading: boolean;
  activeHost: SshHost | null;
  setActiveHost: Dispatch<SetStateAction<SshHost | null>>;
  selectedConfigHostRef: RefObject<HTMLButtonElement | null>;
  chooseConfigured: (host: SshConfigHost) => void;
  manualHosts: SshHost[];
  savedMatch: SshHost | null;
  chooseManual: (id: string) => void;
  folderBrowserOpen: boolean;
  setFolderBrowserOpen: Dispatch<SetStateAction<boolean>>;
  activeTest?: TestState;
  test: (host: SshHost) => Promise<void>;
  removeActive: () => void;
};

export function SshSettingsForm({
  mode,
  chooseMode,
  configHosts,
  configLoading,
  activeHost,
  setActiveHost,
  selectedConfigHostRef,
  chooseConfigured,
  manualHosts,
  savedMatch,
  chooseManual,
  folderBrowserOpen,
  setFolderBrowserOpen,
  activeTest,
  test,
  removeActive,
}: Props) {
  return (
    <>
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
                <span className="text-xs text-ink-faint">{configHosts.length} found</span>
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
    </>
  );
}
