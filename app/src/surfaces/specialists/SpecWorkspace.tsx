import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import {
  ArrowUp,
  ChevronDown,
  Clock3,
  Download,
  FileText,
  ListTree,
  Loader2,
  Share2,
  X,
} from "lucide-react";
import { useSessionStore } from "../../store/sessionStore";
import { readDocText, saveDocPdf, saveDocText } from "../../lib/docs";
import {
  initialSpecMarkdown,
  latestSpecArtifact,
  scopedSpecPrompt,
  specDocumentTitle,
  specDisplayTitle,
  specFilename,
} from "../../lib/specDocuments";
import { withActiveSpecialistSkill } from "../../lib/specialists";
import { MarkdownContent, MARKDOWN_CLASSES } from "../MarkdownContent";
import { Composer } from "../Composer";
import { PermissionGate } from "../PermissionGate";
import { cn } from "../../lib/cn";
import { currentActivity } from "../../lib/activity";
import { useSpecialistStore } from "../../store/specialistStore";
import { productModule } from "../../product/productModule";
import { composerDraftOwner } from "../../lib/composerDraft";
import {
  loadSpecPromptHistory,
  recentSpecPrompts,
  recordSpecPrompt,
  SPEC_PROMPT_HISTORY_EVENT,
} from "../../lib/specPromptHistory";

interface SpecSelection {
  text: string;
  label: string;
}

export function SpecWritingSkeleton({ repositoryFocused = false }: { repositoryFocused?: boolean }) {
  const lines = ["w-40", "w-full", "w-[86%]", "w-[64%]"];
  return (
    <div
      data-qa="spec-writing-skeleton"
      role="status"
      aria-live="polite"
      aria-label={repositoryFocused
        ? "Clark is reading the focused repository and writing the specification"
        : "Clark is writing the specification"}
      className="space-y-3 rounded-lg border border-border-subtle bg-bg-elevated/95 p-4 shadow-lifted backdrop-blur-sm"
    >
      <p className="text-xs font-medium leading-4 text-accent">
        {repositoryFocused
          ? "Reading the focused code and shaping the next section…"
          : "Shaping the next section…"}
      </p>
      <div className="space-y-2" aria-hidden="true">
        {lines.map((width, lineIndex) => (
          <div
            key={lineIndex}
            className={cn(
              width,
              "spec-writing-line h-2 rounded-full",
            )}
            style={{ "--spec-writing-delay": `${lineIndex * 260}ms` } as CSSProperties}
          />
        ))}
      </div>
    </div>
  );
}

function selectionWithin(root: HTMLElement | null): SpecSelection | null {
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

function selectionFromClick(target: EventTarget | null): SpecSelection | null {
  if (!(target instanceof Element)) return null;
  const block = target.closest("h1, h2, h3, p, li, tr");
  const text = block?.textContent?.trim();
  if (!text || text.length < 2) return null;
  return { text: text.slice(0, 4_000), label: text.slice(0, 80) };
}

function SelectionThread({
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
  const [question, setQuestion] = useState("");
  const [sent, setSent] = useState<string[]>([]);
  const [replyAfterIndex, setReplyAfterIndex] = useState<number | null>(null);
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
          value={question}
          onChange={(event) => setQuestion(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              void submit();
            }
          }}
          rows={1}
          placeholder="Ask about this selection…"
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

export function SpecWorkspace() {
  const session = useSessionStore((state) => state.session);
  const snapshot = useSessionStore((state) => state.snapshot);
  const title = useSessionStore((state) => session
    ? state.conversations.find((conversation) => conversation.id === session.id)?.title
    : null);
  const share = useSessionStore((state) => state.shareConversation);
  const auth = useSessionStore((state) => state.auth);
  const flashNotice = useSessionStore((state) => state.flashNotice);
  const setComposerPrefill = useSessionStore((state) => state.setComposerPrefill);
  const renameConversation = useSessionStore((state) => state.renameConversation);
  const repositoryFocused = useSpecialistStore((state) => Boolean(
    state.contexts.spec?.repositoryPath?.trim(),
  ));
  const documentRef = useRef<HTMLDivElement>(null);
  const [markdown, setMarkdown] = useState(() => initialSpecMarkdown(title));
  const [selection, setSelection] = useState<SpecSelection | null>(null);
  const [outlineOpen, setOutlineOpen] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [historyRevision, setHistoryRevision] = useState(0);
  const [downloadOpen, setDownloadOpen] = useState(false);
  const artifact = useMemo(() => latestSpecArtifact(snapshot.artifacts), [snapshot.artifacts]);
  const activity = useMemo(() => currentActivity(snapshot), [snapshot]);
  const documentTitle = useMemo(
    () => specDocumentTitle(markdown) ?? title,
    [markdown, title],
  );
  const displayTitle = specDisplayTitle(documentTitle);
  const promptHistory = useMemo(() => recentSpecPrompts(
    loadSpecPromptHistory(composerDraftOwner(auth?.user ?? null), session?.id ?? null),
    snapshot.timeline,
  ), [auth?.user, historyRevision, session?.id, snapshot.timeline]);

  useEffect(() => {
    const refresh = () => setHistoryRevision((revision) => revision + 1);
    window.addEventListener(SPEC_PROMPT_HISTORY_EVENT, refresh);
    window.addEventListener("storage", refresh);
    return () => {
      window.removeEventListener(SPEC_PROMPT_HISTORY_EVENT, refresh);
      window.removeEventListener("storage", refresh);
    };
  }, []);

  useEffect(() => {
    let alive = true;
    if (!artifact?.uri) {
      setMarkdown(initialSpecMarkdown(title));
      return () => { alive = false; };
    }
    void readDocText(artifact.uri).then((text) => {
      if (alive && text) setMarkdown(text);
    });
    return () => { alive = false; };
  }, [artifact?.id, artifact?.uri, snapshot.timeline.length, title]);

  useEffect(() => {
    if (!session || activity.busy) return;
    const canonicalTitle = specDocumentTitle(markdown);
    if (!canonicalTitle || canonicalTitle === title) return;
    renameConversation(session.id, canonicalTitle);
  }, [activity.busy, markdown, renameConversation, session, title]);

  const headings = useMemo(() => markdown.split("\n")
    .filter((line) => /^#{1,3}\s+/.test(line))
    .map((line) => line.replace(/^#{1,3}\s+/, "").trim()), [markdown]);

  const downloadDocument = async (format: "md" | "pdf") => {
    setDownloadOpen(false);
    const filename = specFilename(documentTitle, format);
    try {
      const saved = format === "md"
        ? await saveDocText(markdown, filename)
        : await saveDocPdf(markdown, specFilename(documentTitle, "md"), artifact?.uri);
      if (saved) flashNotice(`Saved ${filename}`);
    } catch (error: unknown) {
      const detail = error instanceof Error ? error.message : String(error);
      flashNotice(`Could not save ${filename}: ${detail}`);
    }
  };

  return (
    <section data-qa="spec-workspace" className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-bg">
      <header className="flex min-h-[5.5rem] shrink-0 items-center gap-2 border-b border-border-subtle px-4 py-3 sm:px-6">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-sm leading-5">
            <span className="font-medium text-accent">Spec</span>
            <span className="text-ink-faint">/</span>
            <span className="truncate font-medium text-ink">{displayTitle}</span>
          </div>
          <div className="mt-1 flex items-center gap-2 text-xs leading-4 text-ink-faint">
            <span className="min-w-0 truncate">{specFilename(documentTitle, "md")}</span>
            <span className="hidden lg:inline [[data-text-size='150']_&]:hidden [[data-text-size='175']_&]:hidden [[data-text-size='200']_&]:hidden" aria-hidden>·</span>
            <span className="hidden shrink-0 items-center gap-1.5 lg:flex [[data-text-size='150']_&]:hidden [[data-text-size='175']_&]:hidden [[data-text-size='200']_&]:hidden">
              <span className={cn(
                "size-1.5 rounded-full",
                activity.busy
                  ? "breathe bg-accent"
                  : "bg-success",
              )} />
              {activity.busy ? "Clark is working" : "Evolving"} · Auto-saved
            </span>
          </div>
        </div>
        <nav aria-label="Specification actions" className="flex shrink-0 items-center gap-0.5 sm:gap-1">
          <button
            type="button"
            onClick={() => setOutlineOpen((open) => !open)}
            aria-expanded={outlineOpen}
            aria-label="Outline"
            title="Outline"
            className="flex h-9 items-center gap-2 rounded-lg px-2 text-xs font-medium text-ink-muted hover:bg-bg-hover hover:text-ink lg:px-3"
          >
            <ListTree className="size-4" /> <span className="hidden lg:inline [[data-text-size='150']_&]:hidden [[data-text-size='175']_&]:hidden [[data-text-size='200']_&]:hidden">Outline</span>
          </button>
          <div className="relative">
            <button
              type="button"
              onClick={() => setHistoryOpen((open) => !open)}
              aria-expanded={historyOpen}
              aria-label="Prompt history"
              title="Prompt history"
              className="flex h-9 items-center gap-2 rounded-lg px-2 text-xs font-medium text-ink-muted hover:bg-bg-hover hover:text-ink lg:px-3"
            >
              <Clock3 className="size-4" /> <span className="hidden lg:inline [[data-text-size='150']_&]:hidden [[data-text-size='175']_&]:hidden [[data-text-size='200']_&]:hidden">History</span>
            </button>
            {historyOpen && (
              <div
                data-qa="spec-prompt-history"
                className="popover-surface absolute right-0 top-full z-30 mt-1 w-80 max-w-[calc(100vw-2rem)] rounded-xl bg-bg-elevated p-2 shadow-lifted ring-1 ring-border-subtle"
              >
                <div className="flex items-center justify-between px-2 py-1.5">
                  <p className="text-xs font-semibold text-ink">Recent prompts</p>
                  <span className="text-xs text-ink-faint">Last {promptHistory.length}</span>
                </div>
                {promptHistory.length === 0 ? (
                  <p className="px-2 py-3 text-xs leading-5 text-ink-faint">
                    Your latest prompts will stay here for context.
                  </p>
                ) : (
                  <ol className="max-h-72 space-y-1 overflow-y-auto">
                    {[...promptHistory].reverse().map((prompt, index) => (
                      <li key={`${prompt.submittedAt}:${prompt.text}`}>
                        <button
                          type="button"
                          onClick={() => {
                            setComposerPrefill(prompt.text);
                            setHistoryOpen(false);
                          }}
                          title="Put this prompt back in the composer"
                          className="w-full rounded-lg px-2.5 py-2 text-left text-xs leading-5 text-ink-secondary hover:bg-bg-hover hover:text-ink"
                        >
                          <span className="mr-2 text-ink-faint">{promptHistory.length - index}.</span>
                          {prompt.text}
                        </button>
                      </li>
                    ))}
                  </ol>
                )}
              </div>
            )}
          </div>
          <div className="relative">
            <button
              type="button"
              onClick={() => setDownloadOpen((open) => !open)}
              aria-expanded={downloadOpen}
              aria-label="Download"
              title="Download"
              className="flex h-9 items-center gap-2 rounded-lg px-2 text-xs font-medium text-ink-muted hover:bg-bg-hover hover:text-ink lg:px-3"
            >
              <Download className="size-4" /> <span className="hidden lg:inline [[data-text-size='150']_&]:hidden [[data-text-size='175']_&]:hidden [[data-text-size='200']_&]:hidden">Download</span> <ChevronDown className="hidden size-3 lg:block [[data-text-size='150']_&]:hidden [[data-text-size='175']_&]:hidden [[data-text-size='200']_&]:hidden" />
            </button>
            {downloadOpen && (
              <div className="popover-surface absolute right-0 top-full z-30 mt-1 w-52 rounded-xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle">
                <button
                  type="button"
                  onClick={() => void downloadDocument("md")}
                  className="flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-xs text-ink hover:bg-bg-hover"
                >
                  <FileText className="size-4 text-ink-muted" /> {specFilename(documentTitle, "md")}
                </button>
                <button
                  type="button"
                  onClick={() => void downloadDocument("pdf")}
                  className="flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-xs text-ink hover:bg-bg-hover"
                >
                  <FileText className="size-4 text-ink-muted" /> {specFilename(documentTitle, "pdf")}
                </button>
              </div>
            )}
          </div>
          <button
            type="button"
            onClick={() => void share()}
            disabled={!session || !auth}
            aria-label="Share"
            title="Share"
            className="ml-1 flex h-9 items-center gap-2 rounded-lg bg-accent px-2 text-xs font-semibold text-on-accent hover:bg-accent-hover disabled:opacity-40 lg:px-3"
          >
            <Share2 className="size-4" /> <span className="hidden lg:inline [[data-text-size='150']_&]:hidden [[data-text-size='175']_&]:hidden [[data-text-size='200']_&]:hidden">Share</span>
          </button>
        </nav>
      </header>

      <div className="relative flex min-h-0 flex-1 overflow-hidden">
        {outlineOpen && (
          <aside className="w-52 shrink-0 overflow-y-auto border-r border-border-subtle px-3 py-5">
            <p className="px-2 text-xs font-semibold uppercase tracking-[0.1em] text-ink-faint">Sections</p>
            <div className="mt-2 space-y-0.5">
              {headings.map((heading, index) => (
                <button
                  key={`${heading}:${index}`}
                  type="button"
                  onClick={() => {
                    const nodes = documentRef.current?.querySelectorAll("h1, h2, h3");
                    nodes?.[index]?.scrollIntoView({ behavior: "smooth", block: "start" });
                  }}
                  className="flex min-h-8 w-full items-center gap-2 rounded-md px-2 text-left text-xs text-ink-muted hover:bg-bg-hover hover:text-ink"
                >
                  <span className="text-ink-faint">{index + 1}</span>
                  <span className="truncate">{heading}</span>
                </button>
              ))}
            </div>
          </aside>
        )}
        <main className="relative min-w-0 flex-1 overflow-y-auto px-7 pb-28 pt-6">
          <div
            ref={documentRef}
            data-qa="spec-document"
            onMouseUp={() => {
              const next = selectionWithin(documentRef.current);
              if (next) setSelection(next);
            }}
            onDoubleClick={(event) => {
              const next = selectionWithin(documentRef.current) ?? selectionFromClick(event.target);
              if (next) setSelection(next);
            }}
            className={cn(
              MARKDOWN_CLASSES,
              "ml-[clamp(3rem,6vw,6rem)] max-w-[36rem] cursor-text select-text pb-16 text-sm leading-7",
              "[&_h1]:font-serif [&_h1]:text-4xl [&_h1]:font-semibold [&_h1]:tracking-[-0.035em]",
              "[&_h2]:mt-8 [&_h2]:border-t [&_h2]:border-border-subtle [&_h2]:pt-6 [&_h2]:font-serif [&_h2]:text-xl",
              "[&_h1]:cursor-pointer [&_h2]:cursor-pointer [&_h3]:cursor-pointer [&_li]:cursor-text [&_p]:cursor-text",
              "selection:bg-accent/20 selection:text-ink",
            )}
          >
            <MarkdownContent diagrams>{markdown}</MarkdownContent>
          </div>
        </main>
        {activity.busy && (
          <div className="pointer-events-none absolute bottom-4 left-1/2 z-10 w-[min(36rem,calc(100%-3rem))] -translate-x-1/2">
            <SpecWritingSkeleton repositoryFocused={repositoryFocused} />
          </div>
        )}
        {selection && <SelectionThread selection={selection} onClose={() => setSelection(null)} />}
      </div>

      <div className="shrink-0 border-t border-border-subtle bg-bg">
        {snapshot.pending_permission && (
          <div className="mx-auto max-w-[70rem] px-7 pt-3">
            <PermissionGate req={snapshot.pending_permission} />
          </div>
        )}
        <div className="mx-auto -mb-1 flex max-w-[70rem] items-center gap-2 px-7 pt-2 text-xs text-ink-faint">
          <span className="size-1.5 rounded-full bg-success" />
          The document evolves from every answer. Select any text to start a focused discussion.
        </div>
        <Composer />
      </div>
    </section>
  );
}
