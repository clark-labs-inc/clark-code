import type { RefObject } from "react";
import { Library, MessageSquare, PanelLeft, PanelLeftClose, Plus, Search, X } from "lucide-react";
import { useSessionStore } from "../../store/sessionStore";
import { productName } from "../../product/productModule";

export function SidebarHeader({ rail, onToggle, filter, onFilter, searchRef, artifactCount, onOpenArtifacts }: {
  rail: boolean;
  onToggle: () => void;
  filter: string;
  onFilter: (value: string) => void;
  searchRef: RefObject<HTMLInputElement | null>;
  artifactCount: number;
  onOpenArtifacts?: () => void;
}) {
  const chooseProject = useSessionStore((s) => s.setNewProjectOpen);
  const quickChat = useSessionStore((s) => s.startQuickChat);
  const action = rail
    ? "grid size-9 place-items-center rounded-lg text-ink-secondary hover:bg-bg-hover"
    : "flex min-h-9 w-full items-center gap-2 rounded-lg px-3 text-sm font-medium text-ink-secondary hover:bg-bg-hover";
  return (
    <div className={rail ? "flex flex-col items-center gap-1" : "shrink-0 space-y-1 px-3 pb-3"}>
      <div className="flex min-h-12 items-center gap-2">
        {!rail && <span className="min-w-0 flex-1 truncate text-base font-semibold text-ink">{productName()}</span>}
        <button type="button" onClick={onToggle} aria-label={rail ? "Expand sidebar" : "Collapse sidebar"}
          title={rail ? "Expand sidebar" : "Collapse sidebar"}
          className="grid size-9 shrink-0 place-items-center rounded-lg text-ink-muted hover:bg-bg-hover hover:text-ink">
          {rail ? <PanelLeft className="size-4" /> : <PanelLeftClose className="size-4" />}
        </button>
      </div>
      <button type="button" onClick={() => chooseProject(true)} aria-label="New session" title="New session — choose a folder or remote host"
        className={rail ? action : "flex min-h-10 w-full items-center gap-2 rounded-lg bg-accent px-3 text-sm font-medium text-on-accent hover:bg-accent-hover"}>
        <Plus className="size-4" />{!rail && "New session…"}
      </button>
      <button type="button" onClick={async () => {
        await quickChat();
        if (!useSessionStore.getState().error) requestAnimationFrame(() => document.querySelector<HTMLTextAreaElement>("textarea.composer-input")?.focus());
      }} aria-label="New quick chat" title="New quick chat — no project required" className={action}>
        <MessageSquare className="size-4" />{!rail && "New quick chat"}
      </button>
      {onOpenArtifacts && <button type="button" onClick={onOpenArtifacts} aria-label={`Artifacts, ${artifactCount}`} title="Artifacts from the current session" className={action}>
        <Library className="size-4" />{!rail && <><span className="flex-1 text-left">Artifacts</span><span className="text-ink-muted">{artifactCount}</span></>}
      </button>}
      {!rail && <div className="mt-3 flex min-h-9 items-center gap-2 rounded-lg bg-bg px-2.5 ring-1 ring-border-subtle focus-within:ring-accent">
        <Search className="size-3.5 shrink-0 text-ink-muted" />
        <input ref={searchRef} value={filter} onChange={(e) => onFilter(e.target.value)}
          placeholder="Search projects and chats" aria-label="Search projects and chats" autoCorrect="off" autoCapitalize="off" spellCheck={false}
          onKeyDown={(e) => { if (e.key === "Escape" && filter) { e.stopPropagation(); onFilter(""); } }}
          className="min-w-0 flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-muted" />
        {filter && <button type="button" onClick={() => { onFilter(""); searchRef.current?.focus(); }} aria-label="Clear search" className="grid size-7 place-items-center rounded text-ink-muted hover:bg-bg-hover"><X className="size-3.5" /></button>}
      </div>}
    </div>
  );
}
