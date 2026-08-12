import {
  ArrowLeft,
  Blocks,
  CircleUser,
  FolderGit2,
  Info,
  Search,
  SlidersHorizontal,
  SquareTerminal,
  MousePointer2,
  type LucideIcon,
} from "lucide-react";
import type { SettingsSection } from "../../store/sessionStore";
import { cn } from "../../lib/cn";

export interface SettingsNavItem {
  id: SettingsSection;
  label: string;
  description: string;
  keywords: string[];
  icon: LucideIcon;
}

export interface SettingsNavGroup {
  label: string;
  items: SettingsNavItem[];
}

export const SETTINGS_GROUPS: SettingsNavGroup[] = [
  {
    label: "Personal",
    items: [
      {
        id: "general",
        label: "General",
        description: "Appearance, approvals, output, and memory",
        keywords: ["theme", "font", "text size", "contrast", "permissions", "browser", "agents"],
        icon: SlidersHorizontal,
      },
      {
        id: "account",
        label: "Account",
        description: "Profile, plan, usage, and sign out",
        keywords: ["profile", "account", "access", "usage", "limit", "sign out"],
        icon: CircleUser,
      },
    ],
  },
  {
    label: "Workspace",
    items: [
      {
        id: "project",
        label: "Project",
        description: "Folder, model, API key, and knowledge",
        keywords: ["folder", "model", "api key", "knowledge", "repository"],
        icon: FolderGit2,
      },
      {
        id: "commands",
        label: "Command policy",
        description: "Always-allowed and blocked commands",
        keywords: ["shell", "terminal", "allow", "deny", "blocked"],
        icon: SquareTerminal,
      },
    ],
  },
  {
    label: "Extensions",
    items: [
      {
        id: "integrations",
        label: "Integrations",
        description: "MCP servers and remote hosts",
        keywords: ["mcp", "ssh", "server", "remote", "tools"],
        icon: Blocks,
      },
    ],
  },
  {
    label: "System",
    items: [
      {
        id: "computer-use",
        label: "Computer use",
        description: "Native helper, macOS privacy, app grants, and receipts",
        keywords: ["accessibility", "screen recording", "mouse", "keyboard", "approval", "revoke"],
        icon: MousePointer2,
      },
      {
        id: "about",
        label: "About & updates",
        description: "Version and software updates",
        keywords: ["version", "update", "release", "license"],
        icon: Info,
      },
    ],
  },
];

export const SETTINGS_SECTIONS = SETTINGS_GROUPS.flatMap((group) => group.items);

export function filterSettingsGroups(query: string): SettingsNavGroup[] {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return SETTINGS_GROUPS;
  return SETTINGS_GROUPS.flatMap((group) => {
    const items = group.items.filter((item) =>
      [item.label, item.description, ...item.keywords]
        .join(" ")
        .toLocaleLowerCase()
        .includes(needle),
    );
    return items.length ? [{ ...group, items }] : [];
  });
}

export function SettingsNavigation({
  active,
  query,
  onQueryChange,
  onSelect,
  onClose,
}: {
  active: SettingsSection;
  query: string;
  onQueryChange: (query: string) => void;
  onSelect: (section: SettingsSection) => void;
  onClose: () => void;
}) {
  const groups = filterSettingsGroups(query);

  return (
    <aside className="flex w-64 shrink-0 flex-col bg-bg-secondary/70">
      <div className="space-y-3 px-3 pb-3 pt-4">
        <button
          type="button"
          onClick={onClose}
          className="flex min-h-8 w-full items-center gap-2 rounded-md px-2 text-sm text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <ArrowLeft className="size-3.5" />
          Back to app
        </button>
        <h2 id="settings-title" className="px-2 text-base font-semibold text-ink">
          Settings
        </h2>
        <label className="relative block">
          <span className="sr-only">Search settings</span>
          <Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-faint" />
          <input
            type="search"
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder="Search settings"
            className="h-8 w-full rounded-md bg-bg-sunken pl-8 pr-2.5 text-sm text-ink outline-none transition placeholder:text-ink-faint focus:ring-2 focus:ring-accent/20"
          />
        </label>
      </div>

      <nav aria-label="Settings sections" className="min-h-0 flex-1 overflow-y-auto px-3 pb-5">
        {groups.map((group) => (
          <div key={group.label} className="mt-4 first:mt-1">
            <div className="mb-1 px-2 text-xs font-medium text-ink-faint">{group.label}</div>
            <div className="space-y-0.5">
              {group.items.map((item) => {
                const selected = item.id === active;
                return (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => onSelect(item.id)}
                    className={cn(
                      "flex min-h-9 w-full items-center gap-2.5 rounded-md px-2 text-left text-sm transition",
                      selected
                        ? "bg-bg-tertiary text-ink"
                        : "text-ink-secondary hover:bg-bg-hover hover:text-ink",
                    )}
                  >
                    <item.icon className={cn("size-3.5 shrink-0", selected ? "text-accent" : "text-ink-muted")} />
                    <span className="truncate">{item.label}</span>
                  </button>
                );
              })}
            </div>
          </div>
        ))}
        {groups.length === 0 && (
          <p className="px-2 py-5 text-sm leading-relaxed text-ink-muted">
            No settings match “{query.trim()}”.
          </p>
        )}
      </nav>
    </aside>
  );
}
