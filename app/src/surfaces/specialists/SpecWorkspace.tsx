import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  ChevronDown,
  Clock3,
  Download,
  ListTree,
  Share2,
  Sparkles,
} from "lucide-react";
import { useSessionStore } from "../../store/sessionStore";
import { readDocText, saveDocPdf, saveDocText } from "../../lib/docs";
import {
  initialSpecMarkdown,
  latestSpecArtifact,
  specDocumentTitle,
  specDisplayTitle,
  specFilename,
} from "../../lib/specDocuments";
import { MarkdownContent, MARKDOWN_CLASSES } from "../MarkdownContent";
import { SpecDownloadMenu, SpecPromptHistoryMenu } from "./SpecWorkspaceHeaderMenus";
import { SpecLiveDraft } from "./SpecLiveDraft";
import { Composer } from "../Composer";
import { PermissionGate } from "../PermissionGate";
import { cn } from "../../lib/cn";
import { currentActivity } from "../../lib/activity";
import { composerDraftOwner } from "../../lib/composerDraft";
import { wouldAutoApprove } from "../../lib/permissions";
import {
  loadSpecPromptHistory,
  recentSpecPrompts,
  SPEC_PROMPT_HISTORY_EVENT,
} from "../../lib/specPromptHistory";
import { specDocumentDiff, specDocumentInteraction } from "../../lib/specDiff";
import { specGuidance } from "../../lib/specGuidance";
import { currentSpecToolCalls, specLiveStatus, type SpecLiveStatus } from "../../lib/specProgress";
import { completedDocumentWrites, specLiveDraft } from "../../lib/specLiveDraft";
import { specSelectionConversations } from "../../lib/specSelectionThreads";
import { accessibleMotion, RISE } from "../../lib/motion";
import { SpecDocumentDiff, type LiveSpecDocumentDiff } from "./SpecDocumentDiff";
import { SpecRunProgress } from "./SpecRunProgress";
import { SpecWorkingState } from "./SpecWorkingState";
import {
  SpecGuidedDocumentCue,
  SpecGuidedInterview,
  type SpecGuidedPreview,
} from "./SpecGuidedInterview";
import {
  selectionFromClick,
  selectionWithin,
  SpecSelectionThread,
  type SpecSelection,
} from "./SpecSelectionThread";

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
  const documentRef = useRef<HTMLDivElement>(null);
  const loadedArtifactUriRef = useRef<string | null>(null);
  const streamedFirstDraftRef = useRef(false);
  const [markdown, setMarkdown] = useState(() => initialSpecMarkdown(title));
  const [documentLoadState, setDocumentLoadState] = useState<"idle" | "loading" | "ready" | "unavailable">("idle");
  const markdownRef = useRef(markdown);
  const revisionRef = useRef(0);
  const [documentRevision, setDocumentRevision] = useState(0);
  const [documentDiff, setDocumentDiff] = useState<LiveSpecDocumentDiff | null>(null);
  const [guidedOpen, setGuidedOpen] = useState(false);
  const [guidedPreview, setGuidedPreview] = useState<SpecGuidedPreview | null>(null);
  const [selection, setSelection] = useState<SpecSelection | null>(null);
  const [selectionDrafts, setSelectionDrafts] = useState<Record<string, string>>({});
  const [outlineOpen, setOutlineOpen] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [historyRevision, setHistoryRevision] = useState(0);
  const [downloadOpen, setDownloadOpen] = useState(false);
  const artifact = useMemo(() => latestSpecArtifact(snapshot.artifacts), [snapshot.artifacts]);
  const activity = useMemo(() => currentActivity(snapshot), [snapshot]);
  const documentTitle = useMemo(() => specDocumentTitle(markdown), [markdown]);
  const displayTitle = documentTitle ? specDisplayTitle(documentTitle) : "New specification";
  const promptHistory = useMemo(() => recentSpecPrompts(
    loadSpecPromptHistory(composerDraftOwner(auth?.user ?? null), session?.id ?? null),
    snapshot.timeline,
  ), [auth?.user, historyRevision, session?.id, snapshot.timeline]);
  const reduceMotion = useReducedMotion();
  const documentInteraction = specDocumentInteraction(activity.busy);
  const hasDocument = Boolean(artifact?.uri && markdown.trim());
  const hasSubmittedPrompt = promptHistory.length > 0 || snapshot.timeline.some(
    (item) => item.item === "message" && item.role === "user",
  );
  const opening = Boolean(artifact) && !hasDocument && documentLoadState === "loading";
  // Only `busy` (and progress/steps, via spread) are consumed from this; the
  // "Opening your saved spec…" copy lives once, on `runStatus` below.
  const visibleActivity = opening ? { ...activity, busy: true } : activity;
  const runCalls = useMemo(() => currentSpecToolCalls(snapshot), [snapshot]);
  const liveStatus = useMemo(() => specLiveStatus(snapshot, runCalls), [snapshot, runCalls]);
  const liveDraft = useMemo(() => specLiveDraft(runCalls), [runCalls]);
  const documentWrites = useMemo(() => completedDocumentWrites(snapshot), [snapshot]);

  // This component is not keyed by conversation, so refs outlive a switch. A
  // draft streamed in one conversation must not suppress another's reveal.
  useEffect(() => {
    streamedFirstDraftRef.current = false;
  }, [session?.id]);
  useEffect(() => {
    if (liveDraft?.kind === "document") streamedFirstDraftRef.current = true;
  }, [liveDraft]);
  // The load of a saved document is not run activity, so it overrides the ladder.
  const runStatus: SpecLiveStatus = opening
    ? { label: "Opening your saved spec…", source: "starting" }
    : liveStatus;
  const selectionConversations = useMemo(
    () => specSelectionConversations(snapshot.timeline),
    [snapshot.timeline],
  );
  const blockingPermission = snapshot.pending_permission
    && !wouldAutoApprove("full", snapshot.pending_permission)
    ? snapshot.pending_permission
    : null;
  const guidance = useMemo(() => specGuidance(markdown), [markdown]);
  const documentParts = useMemo(() => {
    const match = /^#\s+.+$/m.exec(markdown);
    if (!match || match.index === undefined) return { title: "", body: markdown };
    return {
      title: match[0],
      body: `${markdown.slice(0, match.index)}${markdown.slice(match.index + match[0].length)}`.trim(),
    };
  }, [markdown]);
  const documentMotion = documentRevision > 0
    ? accessibleMotion(RISE, reduceMotion)
    : { initial: false as const };

  const applyDocumentText = useCallback((text: string, animateChange: boolean) => {
    const previous = markdownRef.current;
    if (!text || text === previous) return;
    markdownRef.current = text;
    setMarkdown(text);
    if (!animateChange) return;
    // The all-adds diff of an empty page was this surface's reveal before the
    // draft streamed itself. When the reader has just watched the document being
    // typed, replaying it as a green-diff animation shows the same content a
    // second time. Rewrites of an existing document keep the animation — the
    // draft panel does not render over a live document, so the diff is still
    // the only view of what changed.
    if (!previous.trim() && streamedFirstDraftRef.current) return;
    const diff = specDocumentDiff(previous, text);
    if (!diff) return;
    revisionRef.current += 1;
    const revision = revisionRef.current;
    setDocumentRevision(revision);
    setDocumentDiff({ ...diff, revision });
  }, []);

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
      loadedArtifactUriRef.current = null;
      const initial = initialSpecMarkdown(title);
      markdownRef.current = initial;
      setMarkdown(initial);
      setDocumentLoadState("idle");
      return () => { alive = false; };
    }
    if (loadedArtifactUriRef.current !== artifact.uri) {
      loadedArtifactUriRef.current = artifact.uri;
      markdownRef.current = "";
      setMarkdown("");
      setDocumentLoadState("loading");
    } else {
      setDocumentLoadState(markdownRef.current.trim() ? "ready" : "loading");
    }
    // Coalesces the 350 ms poll against a slow read. Deliberately per-instance:
    // a re-run means something changed that warrants a fresh read, and `alive`
    // already guarantees only the newest instance applies its result. Sharing
    // this across instances would make a re-run skip its own read and could
    // leave the document blank with nothing left to re-trigger it.
    let reading = false;
    const refresh = async () => {
      if (reading) return;
      reading = true;
      let text: string | null = null;
      try {
        text = await readDocText(artifact.uri);
      } finally {
        // `finally`, because a rejected read used to leave the flag latched and
        // silently wedge every later poll tick for this instance.
        reading = false;
      }
      if (!alive) return;
      if (text?.trim()) {
        applyDocumentText(text, activity.busy);
        setDocumentLoadState("ready");
        return;
      }
      // An empty read is only evidence of a missing document when there is
      // nothing already on screen. Otherwise it is a transient — a write in
      // flight, a truncated read — and must not replace a readable spec with an
      // error state.
      if (!activity.busy && !markdownRef.current.trim()) {
        setDocumentLoadState("unavailable");
      }
    };
    void refresh();
    const poll = activity.busy ? window.setInterval(() => void refresh(), 350) : null;
    return () => {
      alive = false;
      if (poll !== null) window.clearInterval(poll);
    };
    // `documentWrites` replaces `snapshot.timeline.length`: it advances when a
    // file actually lands, whereas timeline growth is mostly announcements of
    // work that has written nothing yet — reading then could only return the
    // previous contents, and at the moment a run ended it could read empty and
    // flip a good document to "unavailable".
  }, [activity.busy, applyDocumentText, artifact?.id, artifact?.uri, documentWrites, title]);

  useEffect(() => {
    if (!documentDiff) return;
    const timer = window.setTimeout(
      () => setDocumentDiff(null),
      reduceMotion ? 1_100 : 2_050,
    );
    return () => window.clearTimeout(timer);
  }, [documentDiff, reduceMotion]);

  useEffect(() => {
    setSelectionDrafts({});
  }, [session?.id]);

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

  const toggleGuidedInterview = () => {
    if (guidedOpen) setGuidedPreview(null);
    else setSelection(null);
    setGuidedOpen(!guidedOpen);
  };

  return (
    <section data-qa="spec-workspace" className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-bg">
      <header className="flex min-h-[5.5rem] shrink-0 items-center gap-2 border-b border-border-subtle px-4 py-3 sm:px-6">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-sm leading-5">
            <span className="font-medium text-accent">Spec</span>
            <span className="text-ink-faint">/</span>
            <span className="truncate font-medium text-ink">{displayTitle}</span>
            {guidedOpen && (
              <span className="hidden text-ink-muted sm:inline">· {guidance.clear} of {guidance.total} guided decisions clear</span>
            )}
          </div>
          <div className="mt-1 flex items-center gap-2 text-xs leading-4 text-ink-faint">
            <span className="min-w-0 truncate">{specFilename(documentTitle ?? displayTitle, "md")}</span>
            <span className="hidden lg:inline [[data-text-size='150']_&]:hidden [[data-text-size='175']_&]:hidden [[data-text-size='200']_&]:hidden" aria-hidden>·</span>
            <span className="hidden shrink-0 items-center gap-1.5 lg:flex [[data-text-size='150']_&]:hidden [[data-text-size='175']_&]:hidden [[data-text-size='200']_&]:hidden">
              <span className={cn(
                "size-1.5 rounded-full",
                activity.busy
                  ? "breathe bg-accent"
                  : "bg-ink-faint",
              )} />
              {activity.busy ? "Working live" : hasDocument ? "Saved locally" : "Ready for your prompt"}
            </span>
          </div>
        </div>
        <nav aria-label="Specification actions" className="flex shrink-0 items-center gap-0.5 sm:gap-1">
          <button
            type="button"
            data-qa="spec-guided-toggle"
            onClick={toggleGuidedInterview}
            aria-pressed={guidedOpen}
            aria-label={guidedOpen ? "Close guided interview" : "Open guided interview"}
            title={guidedOpen ? "Close guided interview" : "Guide me through this spec"}
            className={cn(
              "flex h-9 items-center gap-2 rounded-lg px-2 text-xs font-semibold transition-colors lg:px-3",
              guidedOpen
                ? "bg-accent-subtle text-accent"
                : "text-ink-muted hover:bg-bg-hover hover:text-ink",
            )}
          >
            <Sparkles className="size-4" />
            <span className="hidden sm:inline [[data-text-size='175']_&]:hidden [[data-text-size='200']_&]:hidden">Guide me</span>
          </button>
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
              aria-label="Recent prompts"
              title="Recent prompts"
              className="flex h-9 items-center gap-2 rounded-lg px-2 text-xs font-medium text-ink-muted hover:bg-bg-hover hover:text-ink lg:px-3"
            >
              <Clock3 className="size-4" /> <span className="hidden lg:inline [[data-text-size='150']_&]:hidden [[data-text-size='175']_&]:hidden [[data-text-size='200']_&]:hidden">Prompts</span>
            </button>
            {historyOpen && (
              <SpecPromptHistoryMenu
                prompts={promptHistory}
                onPick={(text) => {
                  setComposerPrefill(text);
                  setHistoryOpen(false);
                }}
              />
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
              <SpecDownloadMenu
                documentTitle={documentTitle}
                onDownload={(format) => void downloadDocument(format)}
              />
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

      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden lg:flex-row">
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
        <main className="relative min-h-0 min-w-0 flex-1 overflow-y-auto px-5 pb-28 pt-6 sm:px-7">
          {hasDocument ? (
            <>
              {activity.busy && (
                <SpecRunProgress
                  status={runStatus}
                  activity={activity}
                  calls={runCalls}
                  compact
                />
              )}
              {/* Only a fragment being written earns a panel here. A full
                  rewrite is already named by the progress card, and stacking the
                  incoming document above the current one would show two specs at
                  once — the settled diff animation covers that handoff. */}
              {liveDraft?.kind === "revision" && (
                <SpecLiveDraft draft={liveDraft} className="mb-5" />
              )}
              <div
                ref={documentRef}
                data-qa="spec-document"
                aria-busy={documentInteraction.ariaBusy}
                onMouseUp={() => {
                  if (!documentInteraction.canSelect) return;
                  const next = selectionWithin(documentRef.current);
                  if (next) setSelection(next);
                }}
                onClick={(event) => {
                  if (!documentInteraction.canSelect) return;
                  const next = selectionWithin(documentRef.current)
                    ?? selectionFromClick(documentRef.current, event.target);
                  if (next) setSelection(next);
                }}
                onDoubleClick={(event) => {
                  if (!documentInteraction.canSelect) return;
                  const next = selectionWithin(documentRef.current)
                    ?? selectionFromClick(documentRef.current, event.target);
                  if (next) setSelection(next);
                }}
                className={cn(
                  MARKDOWN_CLASSES,
                  "mx-auto max-w-[44rem] pb-16 text-sm leading-7",
                  documentInteraction.className,
                  "[&_h1]:font-serif [&_h1]:text-4xl [&_h1]:font-semibold [&_h1]:tracking-[-0.035em]",
                  "[&_h2]:mt-8 [&_h2]:border-t [&_h2]:border-border-subtle [&_h2]:pt-6 [&_h2]:font-serif [&_h2]:text-xl",
                  "[&_h1]:cursor-pointer [&_h2]:cursor-pointer [&_h3]:cursor-pointer [&_li]:cursor-text [&_p]:cursor-text",
                  "[&_tbody_tr]:cursor-pointer [&_tbody_tr]:transition-colors [&_tbody_tr:hover]:bg-accent-subtle/60",
                  "[&_p]:rounded-md [&_p]:transition-colors [&_p:hover]:bg-accent-subtle/40",
                  "[&_li]:rounded-md [&_li]:transition-colors [&_li:hover]:bg-accent-subtle/40",
                  "selection:bg-accent/20 selection:text-ink",
                )}
              >
                <AnimatePresence initial={false} mode="wait">
                  {documentDiff ? (
                    <SpecDocumentDiff key={`diff:${documentDiff.revision}`} diff={documentDiff} />
                  ) : (
                    <m.div key={`stable:${documentRevision}`} {...documentMotion}>
                      {documentParts.title && <MarkdownContent diagrams>{documentParts.title}</MarkdownContent>}
                      {guidedOpen && (
                        <SpecGuidedDocumentCue report={guidance} preview={guidedPreview} busy={activity.busy} />
                      )}
                      {documentParts.body && <MarkdownContent diagrams>{documentParts.body}</MarkdownContent>}
                    </m.div>
                  )}
                </AnimatePresence>
              </div>
            </>
          ) : liveDraft ? (
            // The first draft, arriving as it is written. This is the stretch
            // that used to be a progress card over an empty page.
            <>
              {activity.busy && (
                <SpecRunProgress
                  status={runStatus}
                  activity={activity}
                  calls={runCalls}
                  compact
                />
              )}
              <SpecLiveDraft draft={liveDraft} />
            </>
          ) : (
            <SpecWorkingState
              activity={visibleActivity}
              status={runStatus}
              calls={runCalls}
              hasSubmittedPrompt={hasSubmittedPrompt}
              documentUnavailable={documentLoadState === "unavailable"}
            />
          )}
        </main>
        <AnimatePresence initial={false}>
          {guidedOpen && (
            <SpecGuidedInterview report={guidance} busy={activity.busy} onPreview={setGuidedPreview} />
          )}
        </AnimatePresence>
        {selection && (
          <SpecSelectionThread
            selection={selection}
            turns={selectionConversations[selection.key]?.turns ?? []}
            draft={selectionDrafts[selection.key] ?? ""}
            onDraftChange={(draft) => setSelectionDrafts((current) => ({
              ...current,
              [selection.key]: draft,
            }))}
            onClose={() => setSelection(null)}
          />
        )}
      </div>

      <div className={cn("shrink-0 border-t border-border-subtle bg-bg", selection && "hidden lg:block")}>
        {blockingPermission && (
          <div className="mx-auto max-w-[70rem] px-7 pt-3">
            <PermissionGate req={blockingPermission} />
          </div>
        )}
        <div className="mx-auto -mb-1 flex max-w-[70rem] items-center gap-2 px-7 pt-2 text-xs text-ink-faint">
          <span className="size-1.5 rounded-full bg-accent" />
          Shape the whole spec in your own words, or select any text for a focused discussion.
        </div>
        <Composer />
      </div>
    </section>
  );
}
