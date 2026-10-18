import { useEffect, useRef, useState } from "react";
import {
  Archive,
  Check,
  ChevronLeft,
  ChevronRight,
  FolderOpen,
  GitBranchPlus,
  Loader2,
  MoveVertical,
  Pencil,
  Pin,
  PinOff,
  X,
} from "lucide-react";
import type { ProjectGroup } from "../lib/projectSidebar";
import { handleMenuNavigation } from "../lib/menuNavigation";
import { ManagedWorktreeManager } from "./ManagedWorktreeManager";

export interface ProjectMenuPosition {
  left: number;
  top: number;
}

export interface ProjectMoveDestination {
  index: number;
  label: string;
  current: boolean;
}


function MenuItem({
  icon: Icon,
  children,
  danger = false,
  disabled = false,
  autoFocus = false,
  onClick,
}: {
  icon: typeof Pin;
  children: React.ReactNode;
  danger?: boolean;
  disabled?: boolean;
  autoFocus?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      disabled={disabled}
      autoFocus={autoFocus}
      onClick={onClick}
      className={`flex min-h-9 w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-sm transition disabled:cursor-not-allowed disabled:opacity-40 ${
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
  moveDestinations = [],
  onClose,
  onPin,
  onMove,
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
  moveDestinations?: ProjectMoveDestination[];
  onClose: () => void;
  onPin: (pinned: boolean) => void;
  onMove?: (destinationIndex: number) => void;
  onReveal: () => void;
  onCreateWorktree: (name: string) => Promise<void>;
  onListManagedWorktrees: () => Promise<import("../core-bridge/bridge").ManagedWorktree[]>;
  onUseManagedWorktree: (path: string) => void;
  onSaveManagedWorktreeBranch: (id: string) => Promise<{ branch: string }>;
  onCleanupManagedWorktree: (id: string) => Promise<void>;
  /** Current/streaming the agent sessions must keep their checkout intact. */
  activeWorktreePaths: string[];
  onRename: (name: string) => void;
  onArchive: () => void;
  onRemove: () => void;
}) {
  const [mode, setMode] = useState<"menu" | "worktrees" | "move" | "rename" | "worktree" | "managed" | "remove">("menu");
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
      onKeyDown={(event) => {
        if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose(); }
        else handleMenuNavigation(event);
      }}
      role={["menu", "move", "worktrees"].includes(mode) ? "menu" : "dialog"}
      aria-label={`Project actions for ${group.label}`}
      style={{
        left: position.left,
        top: position.top,
        maxHeight: `calc(100vh - ${position.top + 8}px)`,
      }}
      className={"popover-surface fixed z-50 overflow-y-auto rounded-xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle " + (
        mode === "managed" ? "w-[23rem] max-w-[calc(100vw-1rem)]" : "w-60"
      )}
    >
      {mode === "menu" && (
        <>
          <MenuItem
            icon={pinned ? PinOff : Pin}
            autoFocus
            onClick={() => {
              onPin(!pinned);
              onClose();
            }}
          >
            {pinned ? "Unpin project" : "Pin project"}
          </MenuItem>
          {pinned && moveDestinations.length > 1 && onMove && (
            <MenuItem icon={MoveVertical} onClick={() => setMode("move")}>
              Move project…
            </MenuItem>
          )}
          <MenuItem icon={Pencil} onClick={() => setMode("rename")}>
            Rename project
          </MenuItem>
          {group.path && <MenuItem icon={FolderOpen} onClick={() => { onReveal(); onClose(); }}>
            Reveal in Finder
          </MenuItem>}
          {(group.path || group.remoteHost) && <MenuItem icon={GitBranchPlus} onClick={() => setMode("worktrees")}>
            <span className="flex items-center gap-6">Worktrees <ChevronRight className="ml-auto size-3.5" /></span>
          </MenuItem>}
          <div className="my-1 border-t border-border-subtle" />
          <MenuItem
            icon={Archive}
            onClick={() => {
              onArchive();
              onClose();
            }}
          >
            Archive all chats
          </MenuItem>
          <MenuItem icon={X} danger onClick={() => setMode("remove")}>
            Remove project…
          </MenuItem>
        </>
      )}

      {mode === "worktrees" && <>
        <MenuItem icon={ChevronLeft} autoFocus onClick={() => setMode("menu")}>Back to project actions</MenuItem>
        <div className="my-1 border-t border-border-subtle" />
        {group.path && <MenuItem icon={FolderOpen} onClick={() => setMode("managed")}>Manage isolated worktrees</MenuItem>}
        <MenuItem icon={GitBranchPlus} onClick={() => setMode("worktree")}>Create permanent worktree</MenuItem>
      </>}

      {mode === "move" && onMove && (
        <div className="p-1">
          <div className="mb-1 flex items-center gap-1 px-0.5">
            <button
              type="button"
              autoFocus
              onClick={() => setMode("menu")}
              aria-label="Back to project actions"
              className="grid size-7 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink"
            >
              <ChevronLeft className="size-4" />
            </button>
            <span className="text-sm font-medium text-ink-muted">Move {group.label}</span>
          </div>
          {moveDestinations.map((destination) => (
            <button
              key={destination.index}
              type="button"
              role="menuitem"
              disabled={destination.current}
              aria-current={destination.current ? "true" : undefined}
              onClick={() => {
                onMove(destination.index);
                onClose();
              }}
              className="flex min-h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-base text-ink-secondary transition hover:bg-bg-hover hover:text-ink disabled:cursor-default disabled:bg-bg-sunken disabled:text-ink-muted"
            >
              <span className="grid size-4 shrink-0 place-items-center">
                {destination.current && <Check className="size-3.5" />}
              </span>
              <span>{destination.label}</span>
              {destination.current && <span className="ml-auto text-sm text-ink-faint">Current</span>}
            </button>
          ))}
        </div>
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
          <label className="mb-2 block text-sm font-medium text-ink-muted" htmlFor="project-alias">
            Project name
          </label>
          <input
            id="project-alias"
            autoFocus
            value={name}
            onChange={(event) => setName(event.target.value)}
            className="composer-input h-8 w-full rounded-lg bg-bg-sunken px-2.5 text-base text-ink outline-none ring-1 ring-border-subtle focus:ring-border-strong"
          />
          <div className="mt-2 flex justify-end gap-1.5">
            <button type="button" onClick={() => setMode("menu")} className="rounded-lg px-2.5 py-1.5 text-sm text-ink-muted hover:bg-bg-hover hover:text-ink">
              Cancel
            </button>
            <button type="submit" disabled={!name.trim()} className="rounded-lg bg-accent px-2.5 py-1.5 text-sm font-medium text-on-accent disabled:opacity-40">
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
          <div className="mb-2 text-sm text-ink-muted">
            Fetches the latest <span className="font-medium text-ink-secondary">origin/main</span> commit into a sibling checkout without changing this checkout.
          </div>
          <label className="mb-1.5 block text-sm font-medium text-ink-muted" htmlFor="worktree-name">
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
            className="composer-input h-8 w-full rounded-lg bg-bg-sunken px-2.5 text-base text-ink outline-none ring-1 ring-border-subtle focus:ring-border-strong"
          />
          {error && <p className="mt-2 text-sm leading-4 text-danger">{error}</p>}
          <div className="mt-2 flex justify-end gap-1.5">
            <button type="button" disabled={creating} onClick={() => setMode("menu")} className="rounded-lg px-2.5 py-1.5 text-sm text-ink-muted hover:bg-bg-hover hover:text-ink disabled:opacity-40">
              Cancel
            </button>
            <button type="submit" disabled={!worktreeName.trim() || creating} className="flex items-center gap-1.5 rounded-lg bg-accent px-2.5 py-1.5 text-sm font-medium text-on-accent disabled:opacity-40">
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
          <p className="text-sm leading-4 text-ink-muted">
            Remove <span className="font-medium text-ink">{group.label}</span> from the sidebar?
          </p>
          <p className="mt-1 text-sm leading-4 text-ink-faint">Your chats stay available in search. Files on disk won't be changed.</p>
          <div className="mt-2 flex justify-end gap-1.5">
            <button type="button" autoFocus onClick={() => setMode("menu")} className="rounded-lg px-2.5 py-1.5 text-sm text-ink-muted hover:bg-bg-hover hover:text-ink">
              Cancel
            </button>
            <button
              type="button"
              onClick={() => {
                onRemove();
                onClose();
              }}
              className="rounded-lg bg-danger/10 px-2.5 py-1.5 text-sm font-medium text-danger hover:bg-danger/20"
            >
              Remove
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
