import { useEffect, useRef, useState } from "react";
import {
  Archive,
  FolderGit2,
  FolderOpen,
  GitBranchPlus,
  Loader2,
  MessageSquare,
  MoreHorizontal,
  Pencil,
  Pin,
  PinOff,
  Server,
  SquarePen,
  X,
} from "lucide-react";
import type { ProjectGroup } from "../lib/projectSidebar";
import { ManagedWorktreeManager } from "./ManagedWorktreeManager";

export interface ProjectMenuPosition {
  left: number;
  top: number;
}

export function ProjectHeader({
  group,
  menuOpen,
  onOpenMenu,
  onNewSession,
}: {
  group: ProjectGroup;
  menuOpen: boolean;
  onOpenMenu: (button: HTMLButtonElement) => void;
  onNewSession: () => void;
}) {
  const Icon = group.kind === "remote" ? Server : group.kind === "local" ? FolderGit2 : MessageSquare;
  const canStartSession = Boolean(group.path || group.remoteHost);
  const newSessionLabel =
    group.kind === "remote"
      ? `New session on ${group.label}`
      : `New session in ${group.label}`;
  return (
    <div
      title={group.title}
      className="group mb-0.5 mt-2 flex h-7 items-center gap-2 rounded-lg px-2 text-sm font-medium text-ink-secondary first:mt-0 hover:bg-bg-hover"
    >
      <Icon className="size-3.5 shrink-0 text-ink-muted" />
      <span className="min-w-0 flex-1 truncate">{group.label}</span>
      {canStartSession && (
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            onNewSession();
          }}
          title={newSessionLabel}
          aria-label={newSessionLabel}
          className="grid size-5 shrink-0 place-items-center rounded-md text-ink-faint opacity-0 transition hover:bg-bg-sunken hover:text-ink group-hover:opacity-100 group-focus-within:opacity-100"
        >
          <SquarePen className="size-3.5" />
        </button>
      )}
      {group.kind !== "none" && (
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            onOpenMenu(event.currentTarget);
          }}
          title={`Project actions for ${group.label}`}
          aria-label={`Project actions for ${group.label}`}
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          className={`grid size-5 shrink-0 place-items-center rounded-md text-ink-faint transition hover:bg-bg-sunken hover:text-ink focus-visible:opacity-100 ${
            menuOpen ? "bg-bg-sunken text-ink opacity-100" : "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100"
          }`}
        >
          <MoreHorizontal className="size-3.5" />
        </button>
      )}
    </div>
  );
}

function MenuItem({
  icon: Icon,
  children,
  danger = false,
  disabled = false,
  onClick,
}: {
  icon: typeof Pin;
  children: React.ReactNode;
  danger?: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      disabled={disabled}
      onClick={onClick}
      className={`flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-sm transition disabled:cursor-not-allowed disabled:opacity-40 ${
        danger
          ? "text-ink-secondary hover:bg-danger/10 hover:text-danger"
          : "text-ink-secondary hover:bg-bg-hover hover:text-ink"
      }`}
    >
      <Icon className="size-4 shrink-0" />
      <span>{children}</span>
    </button>
  );
}

export function ProjectActionsMenu({
  group,
  position,
  pinned,
  onClose,
  onPin,
  onReveal,
  onCreateWorktree,
  onListManagedWorktrees,
  onUseManagedWorktree,
  onSaveManagedWorktreeBranch,
  onCleanupManagedWorktree,
  activeWorktreePaths,
  onRename,
  onArchive,
  onRemove,
}: {
  group: ProjectGroup;
  position: ProjectMenuPosition;
  pinned: boolean;
  onClose: () => void;
  onPin: (pinned: boolean) => void;
  onReveal: () => void;
  onCreateWorktree: (name: string) => Promise<void>;
  onListManagedWorktrees: () => Promise<import("../core-bridge/bridge").ManagedWorktree[]>;
  onUseManagedWorktree: (path: string) => void;
  onSaveManagedWorktreeBranch: (id: string) => Promise<{ branch: string }>;
  onCleanupManagedWorktree: (id: string) => Promise<void>;
  /** Current/streaming Clark sessions must keep their checkout intact. */
  activeWorktreePaths: string[];
  onRename: (name: string) => void;
  onArchive: () => void;
  onRemove: () => void;
}) {
  const [mode, setMode] = useState<"menu" | "rename" | "worktree" | "managed" | "remove">("menu");
  const [name, setName] = useState(group.label);
  const [worktreeName, setWorktreeName] = useState("");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDown = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  const submitWorktree = async () => {
    if (!worktreeName.trim() || creating) return;
    setCreating(true);
    setError(null);
    try {
      await onCreateWorktree(worktreeName.trim());
      onClose();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setCreating(false);
    }
  };

  return (
    <div
      ref={ref}
      role="menu"
      aria-label={`Project actions for ${group.label}`}
      style={{ left: position.left, top: position.top }}
      className={"popover-surface fixed z-50 rounded-xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle " + (
        mode === "managed" ? "w-[23rem] max-w-[calc(100vw-1rem)]" : "w-60"
      )}
    >
      {mode === "menu" && (
        <>
          <MenuItem
            icon={pinned ? PinOff : Pin}
            onClick={() => {
              onPin(!pinned);
              onClose();
            }}
          >
            {pinned ? "Unpin project" : "Pin project"}
          </MenuItem>
          {group.path && (
            <>
              <MenuItem
                icon={FolderOpen}
                onClick={() => {
                  onReveal();
                  onClose();
                }}
              >
                Reveal in Finder
              </MenuItem>
              <MenuItem icon={GitBranchPlus} onClick={() => setMode("worktree")}>
                Create permanent worktree
              </MenuItem>
              <MenuItem
                icon={FolderOpen}
                onClick={() => {
                  setMode("managed");
                }}
              >
                Manage isolated worktrees
              </MenuItem>
            </>
          )}
          <MenuItem icon={Pencil} onClick={() => setMode("rename")}>
            Rename project
          </MenuItem>
          <MenuItem
            icon={Archive}
            disabled={group.convos.length === 0}
            onClick={() => {
              onArchive();
              onClose();
            }}
          >
            Archive chats
          </MenuItem>
          <MenuItem icon={X} danger onClick={() => setMode("remove")}>
            Remove
          </MenuItem>
        </>
      )}

      {mode === "rename" && (
        <form
          className="p-1.5"
          onSubmit={(event) => {
            event.preventDefault();
            if (!name.trim()) return;
            onRename(name);
            onClose();
          }}
        >
          <label className="mb-2 block text-xs font-medium text-ink-muted" htmlFor="project-alias">
            Project name
          </label>
          <input
            id="project-alias"
            autoFocus
            value={name}
            onChange={(event) => setName(event.target.value)}
            className="composer-input h-8 w-full rounded-lg bg-bg-sunken px-2.5 text-sm text-ink outline-none ring-1 ring-border-subtle focus:ring-border-strong"
          />
          <div className="mt-2 flex justify-end gap-1.5">
            <button type="button" onClick={() => setMode("menu")} className="rounded-lg px-2.5 py-1.5 text-xs text-ink-muted hover:bg-bg-hover hover:text-ink">
              Cancel
            </button>
            <button type="submit" disabled={!name.trim()} className="rounded-lg bg-accent px-2.5 py-1.5 text-xs font-medium text-white disabled:opacity-40">
              Rename
            </button>
          </div>
        </form>
      )}

      {mode === "worktree" && (
        <form
          className="p-1.5"
          onSubmit={(event) => {
            event.preventDefault();
            void submitWorktree();
          }}
        >
          <div className="mb-2 text-xs text-ink-muted">
            Fetches the latest <span className="font-medium text-ink-secondary">origin/main</span> commit into a sibling checkout without changing this checkout.
          </div>
          <label className="mb-1.5 block text-xs font-medium text-ink-muted" htmlFor="worktree-name">
            Worktree name
          </label>
          <input
            id="worktree-name"
            autoFocus
            value={worktreeName}
            onChange={(event) => setWorktreeName(event.target.value)}
            placeholder="feature-name"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            className="composer-input h-8 w-full rounded-lg bg-bg-sunken px-2.5 text-sm text-ink outline-none ring-1 ring-border-subtle focus:ring-border-strong"
          />
          {error && <p className="mt-2 text-xs leading-4 text-danger">{error}</p>}
          <div className="mt-2 flex justify-end gap-1.5">
            <button type="button" disabled={creating} onClick={() => setMode("menu")} className="rounded-lg px-2.5 py-1.5 text-xs text-ink-muted hover:bg-bg-hover hover:text-ink disabled:opacity-40">
              Cancel
            </button>
            <button type="submit" disabled={!worktreeName.trim() || creating} className="flex items-center gap-1.5 rounded-lg bg-accent px-2.5 py-1.5 text-xs font-medium text-white disabled:opacity-40">
              {creating && <Loader2 className="size-3 animate-spin" />}
              Create
            </button>
          </div>
        </form>
      )}

      {mode === "managed" && (
        <ManagedWorktreeManager
          loadWorktrees={onListManagedWorktrees}
          onUseWorktree={(path) => {
            onUseManagedWorktree(path);
            onClose();
          }}
          onSaveBranch={onSaveManagedWorktreeBranch}
          onArchiveCheckout={onCleanupManagedWorktree}
          activeWorktreePaths={activeWorktreePaths}
          onBack={() => setMode("menu")}
        />
      )}

      {mode === "remove" && (
        <div className="p-1.5">
          <p className="text-xs leading-4 text-ink-muted">
            Remove <span className="font-medium text-ink">{group.label}</span> from Projects?
            {group.convos.length > 0 && ` Its ${group.convos.length} active chat${group.convos.length === 1 ? "" : "s"} will be archived.`}
          </p>
          <p className="mt-1 text-xs leading-4 text-ink-faint">Files on disk won't be changed.</p>
          <div className="mt-2 flex justify-end gap-1.5">
            <button type="button" onClick={() => setMode("menu")} className="rounded-lg px-2.5 py-1.5 text-xs text-ink-muted hover:bg-bg-hover hover:text-ink">
              Cancel
            </button>
            <button
              type="button"
              onClick={() => {
                onRemove();
                onClose();
              }}
              className="rounded-lg bg-danger/10 px-2.5 py-1.5 text-xs font-medium text-danger hover:bg-danger/20"
            >
              Remove
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
