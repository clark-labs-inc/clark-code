import { useMemo, useRef } from "react";
import { ArrowUp, Loader2, X } from "lucide-react";

import { useSessionStore } from "../../store/sessionStore";
import { withActiveSpecialistSkill } from "../../lib/specialists";
import { scopedSpecPrompt } from "../../lib/specDocuments";
import { productModule } from "../../product/productModule";
import { composerDraftOwner } from "../../lib/composerDraft";
import { recordSpecPrompt } from "../../lib/specPromptHistory";
import { specInteractionActions } from "../../lib/specInteractions";
import { specSelectionKey, type SpecSelectionTurn } from "../../lib/specSelectionThreads";

export interface SpecSelection {
  text: string;
  label: string;
  key: string;
}

function selectionBlock(target: Node | null): Element | null {
  const element = target instanceof Element ? target : target?.parentElement;
  return element?.closest("h1, h2, h3, p, li, tr") ?? null;
}

function selectionLabel(root: HTMLElement, block: Element | null, fallback: string): string {
  if (!block) return fallback.slice(0, 80);
  if (block.matches("h1, h2, h3")) return block.textContent?.trim().slice(0, 80) || fallback.slice(0, 80);
  const blocks = [...root.querySelectorAll("h1, h2, h3, p, li, tr")];
  for (let index = blocks.indexOf(block) - 1; index >= 0; index -= 1) {
    const candidate = blocks[index];
    if (candidate.matches("h1, h2, h3")) {
      return candidate.textContent?.trim().slice(0, 80) || fallback.slice(0, 80);
    }
  }
  return fallback.slice(0, 80);
}

function specSelection(root: HTMLElement, block: Element | null, text: string): SpecSelection {
  const label = selectionLabel(root, block, text);
  return { text: text.slice(0, 4_000), label, key: specSelectionKey(label) };
}

export function selectionWithin(root: HTMLElement | null): SpecSelection | null {
  const selection = window.getSelection();
  if (!root || !selection || selection.rangeCount === 0 || selection.isCollapsed) return null;
  const range = selection.getRangeAt(0);
  if (!root.contains(range.commonAncestorContainer)) return null;
  const text = selection.toString().trim();
  if (text.length < 2) return null;
  return specSelection(root, selectionBlock(range.startContainer), text);
}

export function selectionFromClick(root: HTMLElement | null, target: EventTarget | null): SpecSelection | null {
  if (!root) return null;
  if (!(target instanceof Element)) return null;
  if (target.closest("a, button, pre, code")) return null;
  const block = target.closest("h1, h2, h3, p, li, tr");
  const text = block?.matches("tr")
    ? [...block.children]
      .map((cell) => cell.textContent?.trim())
      .filter(Boolean)
      .join(" · ")
    : block?.textContent?.trim();
  if (!text || text.length < 2) return null;
  return specSelection(root, block, text);
}

export function SpecSelectionThread({
  selection,
  turns,
  draft,
  onDraftChange,
  onClose,
}: {
  selection: SpecSelection;
  turns: readonly SpecSelectionTurn[];
  draft: string;
  onDraftChange: (value: string) => void;
  onClose: () => void;
}) {
  const session = useSessionStore((state) => state.session);
  const send = useSessionStore((state) => state.send);
  const bridge = useSessionStore((state) => state.bridge);
  const cwd = useSessionStore((state) => state.activeProjectRoot ?? state.localSettings.cwd);
  const activeRemote = useSessionStore((state) => state.activeRemote);
  const auth = useSessionStore((state) => state.auth);
  const busy = useSessionStore((state) => state.snapshot.starting === true || Object.values(state.snapshot.runs)
    .some((run) => run.status === "running" || run.status === "queued"));
  const flashNotice = useSessionStore((state) => state.flashNotice);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const actions = useMemo(() => specInteractionActions(selection.text), [selection.text]);

  const applyAction = (prompt: string) => {
    onDraftChange(prompt);
    requestAnimationFrame(() => {
      const input = inputRef.current;
      input?.focus();
      input?.setSelectionRange(prompt.length, prompt.length);
    });
  };

  const submit = async () => {
    const clean = draft.trim();
    if (!session) {
      flashNotice("Start the spec with the main composer before discussing a selection.");
      return;
    }
    if (!clean || busy) return;
    onDraftChange("");
    const catalog = await bridge?.listSkills?.(
      cwd,
      activeRemote ? { id: activeRemote.id } : null,
    );
    const references = withActiveSpecialistSkill(
      [],
      catalog?.skills ?? [],
      "spec",
      "spec:spec",
    );
    if (references.length === 0) {
      flashNotice("The Spec workflow is unavailable. Reload skills and try again.");
      onDraftChange(clean);
      return;
    }
    try {
      await productModule().specialistWorkspace?.prepareDocument?.("spec", session.id);
    } catch {
      flashNotice("Could not load the saved spec. Try again.");
      onDraftChange(clean);
      return;
    }
    const outcome = await send(scopedSpecPrompt(selection.text, clean, selection.label), references);
    if (outcome.kind === "not_sent") {
      onDraftChange(clean);
    } else {
      recordSpecPrompt(composerDraftOwner(auth?.user ?? null), session.id, clean);
    }
  };

  return (
    <aside
      data-qa="spec-selection-thread"
      aria-label="Discuss selected specification content"
      className="absolute inset-0 z-20 flex min-h-0 flex-col overflow-hidden border-l border-border-subtle bg-bg-elevated lg:static lg:z-auto lg:h-full lg:w-[22rem] lg:max-w-[38vw] lg:shrink-0"
    >
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border-subtle px-3">
        <span className="size-1.5 shrink-0 rounded-full bg-accent" />
        <span className="min-w-0 flex-1 truncate text-xs font-semibold text-ink" title={selection.text}>
          {selection.label}
        </span>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close selection discussion"
          className="grid size-6 shrink-0 place-items-center rounded-md text-ink-muted hover:bg-bg-hover hover:text-ink"
        >
          <X className="size-3.5" />
        </button>
      </header>
      <div className="shrink-0 border-b border-border-subtle bg-accent-subtle/25 px-4 py-3">
        <p className="max-h-16 overflow-hidden text-xs leading-5 text-ink-secondary">
          “{selection.text}”
        </p>
        <div data-qa="spec-selection-actions" className="mt-2 flex flex-wrap gap-1.5">
          {actions.map((action) => (
            <button
              key={action.id}
              type="button"
              onClick={() => applyAction(action.prompt)}
              className="rounded-full border border-border-subtle bg-bg px-2.5 py-1 text-xs font-medium text-ink-muted transition hover:border-accent/35 hover:bg-accent-subtle hover:text-accent"
            >
              {action.label}
            </button>
          ))}
        </div>
      </div>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-4 py-4" aria-live="polite">
        {turns.map((turn, index) => (
          <div key={`${turn.runId}:${index}`} className="space-y-3">
            <div>
              <p className="mb-1 text-xs font-medium text-ink-muted">You</p>
              <div className="ml-5 rounded-lg border border-border-subtle bg-bg-secondary px-3 py-2.5 text-xs leading-relaxed text-ink">
                {turn.question}
              </div>
            </div>
            {turn.reply ? (
              <div>
                <p className="mb-1 text-xs font-medium text-ink-muted">Clark</p>
                <div className="mr-4 whitespace-pre-wrap rounded-lg border border-accent/10 bg-accent-subtle px-3 py-2.5 text-xs leading-relaxed text-ink-secondary">
                  {turn.reply}
                </div>
              </div>
            ) : busy && index === turns.length - 1 ? (
              <div className="flex items-center gap-2 text-xs text-accent">
                <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />
                Updating this section…
              </div>
            ) : (
              <p className="text-xs leading-relaxed text-ink-faint">Review the living document, or keep discussing this section.</p>
            )}
          </div>
        ))}
      </div>
      <div className="flex shrink-0 items-end gap-2 border-t border-border-subtle px-3 py-3">
        <textarea
          ref={inputRef}
          value={draft}
          onChange={(event) => onDraftChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              void submit();
            }
          }}
          rows={1}
          placeholder="Answer, revise, or ask about this…"
          aria-label="Selection discussion message"
          className="max-h-24 min-h-8 min-w-0 flex-1 resize-none bg-transparent px-1 py-1.5 text-xs leading-relaxed text-ink outline-none placeholder:text-ink-muted"
        />
        <button
          type="button"
          onClick={() => void submit()}
          disabled={!draft.trim() || busy}
          aria-label="Send selection discussion"
          className="grid size-7 shrink-0 place-items-center rounded-full bg-accent text-on-accent hover:bg-accent-hover disabled:bg-transparent disabled:text-ink-muted"
        >
          <ArrowUp className="size-3.5" />
        </button>
      </div>
    </aside>
  );
}
