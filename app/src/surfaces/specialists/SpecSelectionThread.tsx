import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowUp, Loader2, X } from "lucide-react";

import { useSessionStore } from "../../store/sessionStore";
import { withActiveSpecialistSkill } from "../../lib/specialists";
import { scopedSpecPrompt } from "../../lib/specDocuments";
import { productModule } from "../../product/productModule";
import { composerDraftOwner } from "../../lib/composerDraft";
import { recordSpecPrompt } from "../../lib/specPromptHistory";
import { specInteractionActions } from "../../lib/specInteractions";

export interface SpecSelection {
  text: string;
  label: string;
}

export function selectionWithin(root: HTMLElement | null): SpecSelection | null {
  const selection = window.getSelection();
  if (!root || !selection || selection.rangeCount === 0 || selection.isCollapsed) return null;
  const range = selection.getRangeAt(0);
  if (!root.contains(range.commonAncestorContainer)) return null;
  const text = selection.toString().trim();
  if (text.length < 2) return null;
  const node = range.commonAncestorContainer instanceof Element
    ? range.commonAncestorContainer
    : range.commonAncestorContainer.parentElement;
  const section = node?.closest("h1, h2, h3, p, li, tr");
  return {
    text: text.slice(0, 4_000),
    label: section?.textContent?.trim().slice(0, 80) || text.slice(0, 80),
  };
}

export function selectionFromClick(target: EventTarget | null): SpecSelection | null {
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
  return { text: text.slice(0, 4_000), label: text.slice(0, 80) };
}

export function SpecSelectionThread({
  selection,
  onClose,
}: {
  selection: SpecSelection;
  onClose: () => void;
}) {
  const session = useSessionStore((state) => state.session);
  const send = useSessionStore((state) => state.send);
  const bridge = useSessionStore((state) => state.bridge);
  const cwd = useSessionStore((state) => state.activeProjectRoot ?? state.localSettings.cwd);
  const activeRemote = useSessionStore((state) => state.activeRemote);
  const auth = useSessionStore((state) => state.auth);
  const timeline = useSessionStore((state) => state.snapshot.timeline);
  const busy = useSessionStore((state) => Object.values(state.snapshot.runs)
    .some((run) => run.status === "running" || run.status === "queued"));
  const flashNotice = useSessionStore((state) => state.flashNotice);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [question, setQuestion] = useState("");
  const [sent, setSent] = useState<string[]>([]);
  const [replyAfterIndex, setReplyAfterIndex] = useState<number | null>(null);

  useEffect(() => {
    setQuestion("");
    setSent([]);
    setReplyAfterIndex(null);
  }, [selection.text]);

  const assistantReply = useMemo(() => {
    if (replyAfterIndex === null) return "";
    const message = [...timeline.slice(replyAfterIndex)].reverse().find(
      (item) => item.item === "message" && item.role === "agent",
    );
    if (!message || message.item !== "message") return "";
    return message.blocks
      .filter((block) => block.type === "text")
      .map((block) => block.text)
      .join("\n")
      .trim();
  }, [replyAfterIndex, timeline]);
  const actions = useMemo(() => specInteractionActions(selection.text), [selection.text]);

  const applyAction = (prompt: string) => {
    setQuestion(prompt);
    requestAnimationFrame(() => {
      const input = inputRef.current;
      input?.focus();
      input?.setSelectionRange(prompt.length, prompt.length);
    });
  };

  const submit = async () => {
    const clean = question.trim();
    if (!session) {
      flashNotice("Start the spec with the main composer before discussing a selection.");
      return;
    }
    if (!clean || busy) return;
    setSent((items) => [...items, clean]);
    setReplyAfterIndex(timeline.length);
    setQuestion("");
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
      setQuestion(clean);
      return;
    }
    try {
      await productModule().specialistWorkspace?.prepareDocument?.("spec", session.id);
    } catch {
      flashNotice("Could not load the saved spec. Try again.");
      setQuestion(clean);
      return;
    }
    const outcome = await send(scopedSpecPrompt(selection.text, clean), references);
    if (outcome.kind === "not_sent") {
      setQuestion(clean);
    } else {
      recordSpecPrompt(composerDraftOwner(auth?.user ?? null), session.id, clean);
    }
  };

  return (
    <aside
      data-qa="spec-selection-thread"
      aria-label="Discuss selected specification content"
      className="absolute right-3 top-3 z-20 flex max-h-[calc(100%-1.5rem)] w-[19rem] max-w-[calc(100%-1.5rem)] flex-col overflow-hidden rounded-lg border border-border-subtle bg-bg-elevated/95 shadow-lifted backdrop-blur-sm sm:right-5 sm:top-5 xl:right-20 xl:top-20"
    >
      <header className="flex h-9 shrink-0 items-center gap-2 border-b border-border-subtle px-2.5">
        <span className="shrink-0 text-xs font-medium text-accent">Selection</span>
        <span className="min-w-0 flex-1 truncate text-xs text-ink-faint" title={selection.text}>
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
      <div className="shrink-0 border-b border-border-subtle px-3 py-3">
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
      {sent.length > 0 && (
        <div className="min-h-0 flex-1 space-y-2.5 overflow-y-auto px-3 py-3">
          {sent.map((message, index) => (
            <div key={`${message}:${index}`} className="ml-5 rounded-md bg-bg-secondary px-2.5 py-2 text-xs leading-relaxed text-ink">
              {message}
            </div>
          ))}
          {busy && sent.length > 0 && (
            <div className="flex items-center gap-2 text-xs text-accent">
              <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />
              Updating this part of the spec…
            </div>
          )}
          {!busy && assistantReply && (
            <div className="mr-4 rounded-md bg-accent-subtle px-2.5 py-2 text-xs leading-relaxed text-ink-secondary">
              {assistantReply}
            </div>
          )}
          {!busy && sent.length > 0 && !assistantReply && (
            <div className="text-xs leading-relaxed text-ink-faint">Review the living document above, or keep discussing this selection.</div>
          )}
        </div>
      )}
      <div className="flex shrink-0 items-end gap-2 px-2.5 py-2">
        <textarea
          ref={inputRef}
          value={question}
          onChange={(event) => setQuestion(event.target.value)}
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
          disabled={!question.trim() || busy}
          aria-label="Send selection discussion"
          className="grid size-7 shrink-0 place-items-center rounded-full bg-accent text-on-accent hover:bg-accent-hover disabled:bg-transparent disabled:text-ink-muted"
        >
          <ArrowUp className="size-3.5" />
        </button>
      </div>
    </aside>
  );
}
