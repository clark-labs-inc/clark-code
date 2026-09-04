import { create } from "zustand";
import {
  SPECIALISTS,
  SPECIALIST_KINDS,
  isSpecialistKind,
  isSpecialistTab,
  type SpecialistContext,
  type SpecialistKind,
  type SpecialistTab,
} from "../lib/specialists";
import { accountScopedKey } from "../lib/accountProjectStorage";
import { codeKeyAccountBinding } from "../lib/account";
import { loadAuthSession } from "../lib/auth";

const STORAGE_KEY = "agent-desktop:specialist-view:v1";

interface PersistedSpecialistState {
  tabs?: Partial<Record<SpecialistKind, SpecialistTab>>;
  contexts?: Partial<Record<SpecialistKind, SpecialistContext>>;
}

function loadPersisted(scope: string | null): PersistedSpecialistState {
  try {
    const value = JSON.parse(
      localStorage.getItem(accountScopedKey(STORAGE_KEY, scope)) ?? "{}",
    ) as PersistedSpecialistState;
    return value && typeof value === "object" ? value : {};
  } catch {
    return {};
  }
}

function savePersisted(
  scope: string | null,
  tabs: Record<SpecialistKind, SpecialistTab>,
  contexts: Partial<Record<SpecialistKind, SpecialistContext>>,
): void {
  try {
    localStorage.setItem(accountScopedKey(STORAGE_KEY, scope), JSON.stringify({ tabs, contexts }));
  } catch {
    // Hardened previews may not expose storage. Cloud conversations still own
    // durable specialist context; this preference only restores the last lens.
  }
}

interface SpecialistState {
  accountScope: string | null;
  active: SpecialistKind | null;
  /** Visible Scout authority chooser opened from the composer context chip. */
  scoutScopeOpen: boolean;
  /** The specialist whose saved-session branch remains expanded in the sidebar.
   * Navigation expansion is intentionally independent from the active workspace:
   * opening a regular session leaves the branch available for the next switch. */
  expanded: SpecialistKind | null;
  tabs: Record<SpecialistKind, SpecialistTab>;
  contexts: Partial<Record<SpecialistKind, SpecialistContext>>;
  open: (kind: SpecialistKind, context?: Partial<SpecialistContext>) => void;
  close: () => void;
  setScoutScopeOpen: (open: boolean) => void;
  setTab: (tab: SpecialistTab) => void;
  setContext: (patch: Partial<SpecialistContext>) => void;
  setAccountScope: (scope: string | null) => void;
}

export function contextsAfterSpecialistOpen(
  current: Partial<Record<SpecialistKind, SpecialistContext>>,
  kind: SpecialistKind,
  defaultWorkflow: string,
  context: Partial<SpecialistContext> = {},
): Partial<Record<SpecialistKind, SpecialistContext>> {
  const requestedContext = Object.keys(context).length === 0
    ? { workflow: defaultWorkflow }
    : context;
  return {
    ...current,
    // Opening establishes a new ownership boundary. Saved conversations pass
    // their complete durable context, while a new lens gets only its default
    // workflow. Merging with the previous context here lets composer-local
    // composer-local authority fields leak into unrelated composers.
    [kind]: { ...requestedContext, kind },
  };
}

function tabsFrom(persisted: PersistedSpecialistState) {
  return Object.fromEntries(
    SPECIALIST_KINDS.map((kind) => {
      const persistedTab = persisted.tabs?.[kind] ?? "";
      return [
        kind,
        isSpecialistTab(kind, persistedTab)
          ? persistedTab
          : SPECIALISTS[kind].defaultTab,
      ];
    }),
  ) as Record<SpecialistKind, SpecialistTab>;
}

function contextsFrom(persisted: PersistedSpecialistState) {
  return Object.fromEntries(
    Object.entries(persisted.contexts ?? {}).filter(
      ([kind, context]) => isSpecialistKind(kind) && context?.kind === kind,
    ),
  ) as Partial<Record<SpecialistKind, SpecialistContext>>;
}

const initialScope = codeKeyAccountBinding(loadAuthSession());
const persisted = loadPersisted(initialScope);
const initialTabs = tabsFrom(persisted);
const initialContexts = contextsFrom(persisted);

export const useSpecialistStore = create<SpecialistState>((set, get) => ({
  accountScope: initialScope,
  active: null,
  scoutScopeOpen: false,
  expanded: null,
  tabs: initialTabs,
  contexts: initialContexts,
  open: (kind, context = {}) => {
    if (!isSpecialistKind(kind)) return;
    // Opening a specialist from navigation starts its default workflow. A
    // slash command may select a narrower workflow for the next conversation,
    // but that choice must not silently stick to later free-form prompts.
    // Saved conversations pass their full context explicitly below and keep
    // the workflow they were created with.
    const contexts = contextsAfterSpecialistOpen(
      get().contexts,
      kind,
      SPECIALISTS[kind].defaultWorkflow,
      context,
    );
    set({
      active: kind,
      expanded: kind,
      contexts,
      scoutScopeOpen: false,
    });
    savePersisted(get().accountScope, get().tabs, contexts);
  },
  close: () => set({ active: null, scoutScopeOpen: false }),
  setScoutScopeOpen: (scoutScopeOpen) => set({ scoutScopeOpen }),
  setTab: (tab) => {
    const kind = get().active;
    if (!kind || !isSpecialistTab(kind, tab)) return;
    const tabs = { ...get().tabs, [kind]: tab };
    set({ tabs });
    savePersisted(get().accountScope, tabs, get().contexts);
  },
  setContext: (patch) => {
    const kind = get().active;
    if (!kind) return;
    const contexts = {
      ...get().contexts,
      [kind]: { ...get().contexts[kind], ...patch, kind },
    };
    set({ contexts });
    savePersisted(get().accountScope, get().tabs, contexts);
  },
  setAccountScope: (scope) => {
    if (scope === get().accountScope) return;
    const next = loadPersisted(scope);
    set({
      accountScope: scope,
      active: null,
      scoutScopeOpen: false,
      expanded: null,
      tabs: tabsFrom(next),
      contexts: contextsFrom(next),
    });
  },
}));

export function activeSpecialistContext(): SpecialistContext | null {
  const state = useSpecialistStore.getState();
  return state.active
    ? (state.contexts[state.active] ?? { kind: state.active })
    : null;
}
