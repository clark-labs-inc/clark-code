import { useEffect, useId, useRef, useState } from "react";
import {
  Laptop, Server, Cloud, Folder, FolderOpen, Check, ChevronDown, ChevronRight, ArrowUp,
  Plus, Settings2, Loader2, AlertCircle,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { loadSshHosts, saveSshHosts, hostLabel, hostReady, type SshHost } from "../lib/sshHosts";
import { listSshDirectories, type RemoteDirectoryListing } from "../lib/ssh";
import { codeKeyAccountBinding } from "../lib/account";
import { inTauri } from "../lib/pickFolder";
import { cn } from "../lib/cn";

const CHIP =
  "flex min-h-8 items-center gap-1.5 rounded-xl border border-accent/10 bg-accent-subtle px-2.5 py-1.5 text-sm font-medium text-ink-secondary transition duration-base ease-agent hover:bg-accent-soft hover:text-ink";
const COMPACT_CHIP =
  "flex min-h-7 items-center gap-1 rounded-md bg-composer-context px-1.5 text-xs font-medium leading-none text-ink-secondary transition duration-base ease-agent hover:bg-bg-hover hover:text-ink";

/** The "Local · Select folder…" control that sits above the start-screen
 *  composer. It maps the target machine (Local / a Cloud provider / an SSH host)
 *  and the project folder onto the same store state the session starts from. */
export function EnvironmentPicker({
  compact = false,
  allowCloud = true,
  showLocalFolder = true,
  onEnvironmentChanged,
}: {
  compact?: boolean;
  /** Specialist workflows backed by the native worker can run locally or over
   * SSH, but cannot be redirected into an unrelated cloud provider. */
  allowCloud?: boolean;
  /** Document-first workflows own their local workspace. They still need the
   * remote folder picker when SSH is selected. */
  showLocalFolder?: boolean;
  onEnvironmentChanged?: () => void;
}) {
  const providers = useSessionStore((s) => s.providers);
  const activeProvider = useSessionStore((s) => s.activeProvider);
  const selectProvider = useSessionStore((s) => s.selectProvider);
  const projectMode = useSessionStore((s) => s.projectMode);
  const setProjectMode = useSessionStore((s) => s.setProjectMode);
  const cwd = useSessionStore((s) => s.localSettings.cwd);
  const selectedHostId = useSessionStore((s) => s.selectedHostId);
  const setSelectedHostId = useSessionStore((s) => s.setSelectedHostId);
  const setSshOpen = useSessionStore((s) => s.setSshOpen);
  const sshOpen = useSessionStore((s) => s.sshOpen);
  const auth = useSessionStore((s) => s.auth);
  const accountScope = codeKeyAccountBinding(auth);

  // Hosts live in localStorage; refresh when the manage-hosts modal closes.
  const [hosts, setHosts] = useState<SshHost[]>(() => loadSshHosts(accountScope));
  useEffect(() => {
    if (!sshOpen) setHosts(loadSshHosts(accountScope));
  }, [accountScope, sshOpen]);

  // Provider discovery is asynchronous. Until it completes, the workspace is
  // still local rather than an invented third "Cloud" destination.
  const isLocal = activeProvider === null || activeProvider === "local";
  const isRemote = isLocal && projectMode === "remote";
  const selectedHost = hosts.find((h) => h.id === selectedHostId) ?? null;
  const cloudProviders = allowCloud
    ? providers.filter((p) => p.id !== "local" && !p.internal)
    : [];

  let label = "Local";
  let TargetIcon = Laptop;
  if (isRemote) {
    label = selectedHost ? hostLabel(selectedHost) : "Remote";
    TargetIcon = Server;
  } else if (!isLocal) {
    label = providers.find((p) => p.id === activeProvider)?.label ?? "Cloud";
    TargetIcon = Cloud;
  }

  const pickLocal = () => {
    if (!isLocal) selectProvider("local");
    setProjectMode("local");
  };
  const pickRemoteHost = (id: string) => {
    if (!isLocal) selectProvider("local");
    setProjectMode("remote");
    setSelectedHostId(id);
  };
  const pickCloud = (id: string) => selectProvider(id);

  const targetPicker = (
    <Popover
      popupLabel="Execution targets"
      trigger={
        <span className={compact ? COMPACT_CHIP : CHIP}>
          <TargetIcon className={compact ? "size-3" : "size-4"} />
          <span className="max-w-[10rem] truncate">{label}</span>
          {!compact && <ChevronDown className="size-3.5 text-ink-faint" />}
        </span>
      }
    >
      {(close) => (
        <div className="w-64">
          <OptionRow
            icon={<Laptop className="size-4" />}
            label="Local"
            hint="This machine"
            active={isLocal && !isRemote}
            onClick={() => {
              pickLocal();
              close();
            }}
          />

          {cloudProviders.length > 0 && (
            <>
              <SectionLabel>Cloud</SectionLabel>
              {cloudProviders.map((p) => (
                <OptionRow
                  key={p.id}
                  icon={<Cloud className="size-4" />}
                  label={p.label}
                  active={activeProvider === p.id}
                  onClick={() => {
                    pickCloud(p.id);
                    close();
                  }}
                />
              ))}
            </>
          )}

          <SectionLabel>SSH</SectionLabel>
          {hosts.map((h) => (
            <OptionRow
              key={h.id}
              icon={<Server className="size-4" />}
              label={hostLabel(h)}
              hint={hostReady(h) ? h.host : "needs setup"}
              active={isRemote && selectedHostId === h.id}
              onClick={() => {
                pickRemoteHost(h.id);
                close();
              }}
            />
          ))}
          <OptionRow
            icon={<Plus className="size-4" />}
            label="Add SSH host…"
            onClick={() => {
              setSshOpen(true);
              close();
            }}
          />
        </div>
      )}
    </Popover>
  );

  const updateRemoteRoot = (host: SshHost, remoteRoot: string) => {
    const next = hosts.map((entry) => entry.id === host.id ? { ...entry, remoteRoot } : entry);
    setHosts(next);
    saveSshHosts(next, accountScope);
    onEnvironmentChanged?.();
  };

  const folderPicker = isLocal && !isRemote && showLocalFolder ? (
    <FolderChip cwd={cwd} compact={compact} />
  ) : isRemote && selectedHost ? (
    <RemoteFolderChip
      host={selectedHost}
      compact={compact}
      onSelect={(path) => updateRemoteRoot(selectedHost, path)}
      onManage={() => setSshOpen(true)}
    />
  ) : null;

  return (
    <div
      className={cn("flex items-center", compact ? "min-w-0 gap-1.5" : "flex-wrap gap-2")}
    >
      {/* Target machine */}
      {targetPicker}

      {/* Project folder — only meaningful when coding on this machine */}
      {folderPicker}

    </div>
  );
}

function RemoteFolderChip({
  host,
  compact,
  onSelect,
  onManage,
}: {
  host: SshHost;
  compact: boolean;
  onSelect: (path: string) => void;
  onManage: () => void;
}) {
  const has = host.remoteRoot.trim().length > 0;
  return (
    <Popover
      popupLabel={`Remote folders on ${hostLabel(host)}`}
      trigger={
        <span className={cn(compact ? COMPACT_CHIP : CHIP, has ? "text-ink" : "text-ink-faint")}>
          <Folder className={compact ? "size-3" : "size-4"} />
          <span className="max-w-[12rem] truncate">
            {has ? projectName(host.remoteRoot) : "Select remote folder…"}
          </span>
          {!compact && <ChevronDown className="size-3.5 text-ink-faint" />}
        </span>
      }
    >
      {(close) => (
        <RemoteFolderBrowser
          host={host}
          onSelect={(path) => {
            onSelect(path);
            close();
          }}
          onManage={() => {
            onManage();
            close();
          }}
        />
      )}
    </Popover>
  );
}

export function RemoteFolderBrowser({
  host,
  onSelect,
  onManage,
}: {
  host: SshHost;
  onSelect: (path: string) => void;
  onManage: () => void;
}) {
  const [listing, setListing] = useState<RemoteDirectoryListing | null>(null);
  const [path, setPath] = useState(host.remoteRoot);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);

  const browse = async (nextPath?: string | null) => {
    const id = ++request.current;
    setLoading(true);
    setError(null);
    try {
      const next = await listSshDirectories(host.host.trim(), nextPath);
      if (request.current !== id) return;
      setListing(next);
      setPath(next.path);
    } catch (cause) {
      if (request.current !== id) return;
      setError(String(cause));
    } finally {
      if (request.current === id) setLoading(false);
    }
  };

  useEffect(() => {
    void browse(host.remoteRoot || null);
    return () => {
      request.current += 1;
    };
    // The browser is remounted for each popover opening/host selection.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [host.id]);

  return (
    <div className="w-[22rem] p-1">
      <div className="mb-1 flex items-center gap-1.5 px-1">
        <Server className="size-3.5 shrink-0 text-ink-muted" />
        <span className="truncate text-xs font-medium text-ink-secondary">{hostLabel(host)}</span>
      </div>
      <form
        className="flex gap-1"
        onSubmit={(event) => {
          event.preventDefault();
          void browse(path);
        }}
      >
        <input
          value={path}
          onChange={(event) => setPath(event.target.value)}
          aria-label="Remote folder path"
          placeholder="/home/ubuntu/git/project"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          className="min-w-0 flex-1 rounded-lg border border-border bg-bg px-2 py-1.5 font-mono text-xs text-ink outline-none transition focus:border-accent"
        />
        <button
          type="submit"
          disabled={loading || !path.trim()}
          className="grid size-8 shrink-0 place-items-center rounded-lg bg-bg-tertiary text-ink-secondary transition hover:bg-bg-hover disabled:opacity-50"
          aria-label="Open remote path"
        >
          <ChevronRight className="size-4" />
        </button>
      </form>

      <div className="mt-1 max-h-64 min-h-32 overflow-y-auto rounded-xl border border-border-subtle bg-bg">
        {listing?.parent && (
          <button
            type="button"
            onClick={() => void browse(listing.parent)}
            disabled={loading}
            className="flex min-h-9 w-full items-center gap-2 border-b border-border-subtle px-2.5 text-left text-sm text-ink-secondary transition hover:bg-accent-subtle disabled:opacity-50"
          >
            <ArrowUp className="size-3.5 text-ink-muted" />
            <span>Parent folder</span>
          </button>
        )}
        {loading && (
          <div className="flex min-h-28 items-center justify-center gap-2 text-sm text-ink-muted">
            <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" /> Loading folders…
          </div>
        )}
        {!loading && error && (
          <div className="flex min-h-28 items-start gap-2 p-3 text-sm text-danger">
            <AlertCircle className="mt-0.5 size-4 shrink-0" />
            <span className="min-w-0 break-words">{error}</span>
          </div>
        )}
        {!loading && !error && listing?.directories.length === 0 && (
          <div className="grid min-h-28 place-items-center text-sm text-ink-muted">No subfolders</div>
        )}
        {!loading && !error && listing?.directories.map((directory) => (
          <button
            type="button"
            key={directory.path}
            onClick={() => void browse(directory.path)}
            className="flex min-h-9 w-full items-center gap-2 border-b border-border-subtle px-2.5 text-left transition last:border-0 hover:bg-accent-subtle"
          >
            <Folder className="size-3.5 shrink-0 text-ink-muted" />
            <span className="min-w-0 flex-1 truncate text-sm text-ink">{directory.name}</span>
            <ChevronRight className="size-3.5 shrink-0 text-ink-faint" />
          </button>
        ))}
      </div>

      <div className="mt-1.5 flex items-center gap-1.5">
        <button
          type="button"
          onClick={onManage}
          className="grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink"
          aria-label="Manage remote host"
          title="Manage remote host"
        >
          <Settings2 className="size-3.5" />
        </button>
        <button
          type="button"
          onClick={() => listing && onSelect(listing.path)}
          disabled={loading || !listing}
          className="ml-auto min-h-8 rounded-lg bg-accent px-3 text-sm font-semibold text-on-accent transition hover:bg-accent-hover disabled:opacity-50"
        >
          Use this folder
        </button>
      </div>
    </div>
  );
}

/** Folder chip: shows the project name, opens a popover with a native picker
 *  (Tauri), a path field, and recent projects. */
function FolderChip({ cwd, compact = false }: { cwd: string; compact?: boolean }) {
  const pick = useSessionStore((s) => s.pickProjectFolder);
  const setProject = useSessionStore((s) => s.setProjectFolder);
  const setLocal = useSessionStore((s) => s.setLocalSettings);
  const recents = useSessionStore((s) => s.recentProjects);
  const tauri = inTauri();
  const has = cwd.trim().length > 0;

  return (
    <Popover
      popupLabel="Project folder"
      trigger={
        <span
          className={cn(compact ? COMPACT_CHIP : CHIP, has ? "text-ink" : "text-ink-faint")}
        >
          <Folder className={compact ? "size-3" : "size-4"} />
          <span className="max-w-[12rem] truncate">
            {has ? projectName(cwd) : "Select folder…"}
          </span>
          {!compact && <ChevronDown className="size-3.5 text-ink-faint" />}
        </span>
      }
    >
      {(close) => (
        <div className="w-72 p-1">
          {tauri && (
            <button
              onClick={() => {
                void pick();
                close();
              }}
              className="mb-1 flex min-h-10 w-full items-center gap-2 rounded-xl bg-accent px-2.5 py-2 text-sm font-medium text-on-accent transition duration-base ease-agent hover:bg-accent-hover"
            >
              <FolderOpen className="size-4" /> Choose folder…
            </button>
          )}
          <input
            type="text"
            value={cwd}
            onChange={(e) => setLocal({ cwd: e.target.value })}
            placeholder={tauri ? "…or paste an absolute path" : "/Users/you/code/my-project"}
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            className="w-full rounded-lg border border-border bg-bg px-2.5 py-1.5 text-sm text-ink outline-none transition focus:border-accent placeholder:text-ink-muted"
          />
          {recents.length > 0 && (
            <div className="mt-2">
              <SectionLabel>Recent</SectionLabel>
              {recents.map((p) => (
                <OptionRow
                  key={p}
                  icon={<Folder className="size-4" />}
                  label={projectName(p)}
                  hint={p}
                  active={p === cwd}
                  onClick={() => {
                    setProject(p);
                    close();
                  }}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </Popover>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-2 pb-1 pt-2 text-xs font-semibold uppercase tracking-wider text-ink-faint">
      {children}
    </div>
  );
}

function OptionRow({
  icon,
  label,
  hint,
  active,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  hint?: string;
  active?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex min-h-9 w-full items-center gap-2.5 rounded-xl px-2 py-1.5 text-left transition duration-base ease-agent hover:bg-accent-subtle",
        active && "bg-accent-subtle",
      )}
    >
      <span className={cn("shrink-0", active ? "text-accent" : "text-ink-muted")}>{icon}</span>
      <span className="min-w-0 flex-1 leading-tight">
        <span className="block truncate text-sm text-ink">{label}</span>
        {hint && <span className="block truncate text-xs text-ink-faint">{hint}</span>}
      </span>
      {active && <Check className="size-4 shrink-0 text-accent" />}
    </button>
  );
}

/** Click-outside popover that opens upward (the picker sits at the bottom of the
 *  window, above the composer). */
function Popover({
  trigger,
  popupLabel,
  children,
}: {
  trigger: React.ReactNode;
  popupLabel: string;
  children: (close: () => void) => React.ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popupId = useId();
  const close = (restoreFocus = true) => {
    setOpen(false);
    if (restoreFocus) triggerRef.current?.focus();
  };
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      close();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button
        ref={triggerRef}
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? popupId : undefined}
        onClick={() => setOpen((o) => !o)}
      >
        {trigger}
      </button>
      {open && (
        <div
          id={popupId}
          role="dialog"
          aria-label={popupLabel}
          className="popover-surface absolute bottom-full left-0 z-30 mb-2 rounded-2xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
        >
          {children(close)}
        </div>
      )}
    </div>
  );
}
