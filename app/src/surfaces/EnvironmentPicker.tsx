import { useEffect, useRef, useState } from "react";
import {
  Laptop, Server, Cloud, Folder, FolderOpen, Check, ChevronDown, Plus, Settings2,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { loadSshHosts, hostLabel, hostReady, type SshHost } from "../lib/sshHosts";
import { inTauri } from "../lib/pickFolder";
import { cn } from "../lib/cn";

const CHIP =
  "flex min-h-8 items-center gap-1.5 rounded-xl border border-accent/10 bg-accent-subtle px-2.5 py-1.5 text-sm font-medium text-ink-secondary transition duration-200 ease-clark hover:bg-accent-soft hover:text-ink";
const COMPACT_CHIP =
  "flex h-[22px] items-center gap-1 rounded-md bg-composer-context px-1.5 text-[11px] font-medium leading-none text-ink-secondary transition duration-200 ease-clark hover:bg-bg-hover hover:text-ink";

/** The "Local · Select folder…" control that sits above the start-screen
 *  composer. It maps the target machine (Local / a Cloud provider / an SSH host)
 *  and the project folder onto the same store state the session starts from. */
export function EnvironmentPicker({ compact = false }: { compact?: boolean }) {
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

  // Hosts live in localStorage; refresh when the manage-hosts modal closes.
  const [hosts, setHosts] = useState<SshHost[]>(() => loadSshHosts());
  useEffect(() => {
    if (!sshOpen) setHosts(loadSshHosts());
  }, [sshOpen]);

  const isLocal = activeProvider === "local";
  const isRemote = isLocal && projectMode === "remote";
  const selectedHost = hosts.find((h) => h.id === selectedHostId) ?? null;
  const cloudProviders = providers.filter((p) => p.id !== "local");

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

  const folderPicker = isLocal && !isRemote ? (
    <FolderChip cwd={cwd} compact={compact} />
  ) : null;

  return (
    <div
      className={cn("flex items-center", compact ? "min-w-0 gap-1.5" : "flex-wrap gap-2")}
    >
      {/* Target machine */}
      {targetPicker}

      {/* Project folder — only meaningful when coding on this machine */}
      {folderPicker}

      {isRemote && selectedHost && (
        <button
          onClick={() => setSshOpen(true)}
          className={compact ? COMPACT_CHIP : CHIP}
          title="Manage remote hosts"
        >
          <Settings2 className="size-3.5" />
          <span className="max-w-[12rem] truncate font-mono text-xs">
            {selectedHost.remoteRoot || "no folder set"}
          </span>
        </button>
      )}
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
              className="mb-1 flex min-h-10 w-full items-center gap-2 rounded-xl bg-accent px-2.5 py-2 text-sm font-medium text-on-accent transition duration-200 ease-clark hover:bg-accent-hover"
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
        "flex min-h-9 w-full items-center gap-2.5 rounded-xl px-2 py-1.5 text-left transition duration-200 ease-clark hover:bg-accent-subtle",
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
  children,
}: {
  trigger: React.ReactNode;
  children: (close: () => void) => React.ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button type="button" onClick={() => setOpen((o) => !o)}>
        {trigger}
      </button>
      {open && (
        <div className="popover-surface absolute bottom-full left-0 z-30 mb-2 rounded-[22px] bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle">
          {children(() => setOpen(false))}
        </div>
      )}
    </div>
  );
}
