import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlignLeft,
  Check,
  ChevronDown,
  ChevronLeft,
  Clock3,
  Copy,
  Download,
  ExternalLink,
  FileBox,
  FileText,
  Film,
  Globe,
  History,
  Image as ImageIcon,
  Info,
  Link2,
  Loader2,
  MessageCircle,
  PanelRightClose,
  Plus,
  Presentation,
  Sparkles,
  X,
} from "lucide-react";
import type { Artifact, ArtifactKind, ToolCall } from "../../core-bridge/types";
import { useCopy } from "../../lib/clipboard";
import { cn } from "../../lib/cn";
import { readDocText, saveDocText } from "../../lib/docs";
import { openExternal } from "../../lib/account";
import {
  artifactAvailability,
  artifactLocationLabel,
  canOpenArtifactExternally,
  readableArtifactLocation,
} from "../../lib/artifactPresentation";
import { Md, MD_CLASSES } from "../Message";
import { LocalArtifactImage } from "./ArtifactCard";
import { isMarkdownDoc } from "./MarkdownDoc";

const KIND_ICON: Record<ArtifactKind, typeof FileBox> = {
  website: Globe,
  video: Film,
  media: Film,
  image: ImageIcon,
  pdf: FileText,
  office: FileText,
  slides: Presentation,
  file: FileText,
  diff: FileText,
  search_results: FileText,
  other: FileBox,
};

const KIND_LABEL: Record<ArtifactKind, string> = {
  website: "Website",
  video: "Video",
  media: "Media",
  image: "Image",
  pdf: "PDF",
  office: "Document",
  slides: "Slides",
  file: "File",
  diff: "Diff",
  search_results: "Search results",
  other: "Artifact",
};

type ContextPanel = "details" | "versions" | "comments" | "source";

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function isVideoArtifact(artifact: Artifact): boolean {
  return (
    artifact.kind === "video" ||
    (artifact.kind === "media" && /video|\.(mp4|webm|mov)/i.test(artifact.uri ?? artifact.mime_type ?? ""))
  );
}

function sourceTitle(call?: ToolCall): string {
  if (!call) return "Produced during this conversation";
  if (call.kind === "research") {
    const query = (call.raw_input as { query?: string } | undefined)?.query;
    return query ? `Researched ${query}` : call.title.replace(/^clark_research:\s*/i, "Researched ");
  }
  return call.title;
}

function artifactDomId(prefix: string, artifactId: string): string {
  return `${prefix}-${artifactId.replace(/[^a-z0-9_-]+/gi, "-")}`;
}

function ArtifactTab({
  artifact,
  active,
  tabId,
  panelId,
  onSelect,
  onClose,
}: {
  artifact: Artifact;
  active: boolean;
  tabId: string;
  panelId: string;
  onSelect: () => void;
  onClose: () => void;
}) {
  const Icon = isMarkdownDoc(artifact) ? FileText : (KIND_ICON[artifact.kind] ?? FileBox);
  return (
    <div
      className={cn(
        "group flex h-10 min-w-[10rem] max-w-[15rem] shrink-0 items-center border-r border-border-subtle px-2.5",
        active ? "bg-bg-elevated text-ink" : "bg-bg-secondary/45 text-ink-muted hover:bg-bg-hover/60",
      )}
    >
      <button
        type="button"
        id={tabId}
        role="tab"
        onClick={onSelect}
        aria-selected={active}
        aria-controls={panelId}
        tabIndex={active ? 0 : -1}
        className="flex min-w-0 flex-1 items-center gap-2 text-left"
      >
        <Icon className={cn("size-3.5 shrink-0", active ? "text-accent" : "text-ink-faint")} />
        <span className="truncate text-xs font-medium">{artifact.title}</span>
      </button>
      <button
        type="button"
        onClick={(event) => {
          event.stopPropagation();
          onClose();
        }}
        aria-label={`Close ${artifact.title}`}
        className="ml-1 grid size-6 shrink-0 place-items-center rounded-md text-ink-faint opacity-70 transition hover:bg-bg-hover hover:text-ink group-hover:opacity-100"
      >
        <X className="size-3" />
      </button>
    </div>
  );
}

function GenericPreview({ artifact }: { artifact: Artifact }) {
  const Icon = KIND_ICON[artifact.kind] ?? FileBox;
  const location = readableArtifactLocation(artifact);
  const availability = artifactAvailability(artifact);
  return (
    <div className="mx-auto flex min-h-full max-w-3xl items-center justify-center px-8 py-12">
      <div className="w-full rounded-xl border border-border bg-bg-elevated px-7 py-8 text-center shadow-soft">
        <span className="mx-auto grid size-14 place-items-center rounded-xl bg-accent-subtle text-accent">
          <Icon className="size-6" />
        </span>
        <h1 className="mt-4 font-display text-2xl text-ink">{artifact.title}</h1>
        <p className="mt-1 text-sm text-ink-muted">
          {KIND_LABEL[artifact.kind]} · {artifactLocationLabel(artifact)}
        </p>
        {location && <p className="mx-auto mt-3 max-w-lg truncate font-mono text-xs text-ink-faint">{location}</p>}
        <p className="mx-auto mt-5 max-w-md text-sm leading-relaxed text-ink-muted">
          {availability === "unavailable"
            ? "This artifact does not currently have a readable file or link. Its source remains available for context."
            : "Clark keeps this artifact in your workspace without embedding an unreliable preview. Its source and location remain visible here."}
        </p>
        {canOpenArtifactExternally(artifact) && (
          <button
            type="button"
            onClick={() => void openExternal(artifact.uri!)}
            className="mt-6 inline-flex h-10 items-center gap-2 rounded-lg bg-accent px-4 text-sm font-medium text-on-accent transition hover:bg-accent-hover"
          >
            View {artifact.title} <ExternalLink className="size-3.5" />
          </button>
        )}
      </div>
    </div>
  );
}

function ArtifactPreview({
  artifact,
  text,
  loading,
  presenting,
}: {
  artifact: Artifact;
  text: string | null;
  loading: boolean;
  presenting: boolean;
}) {
  const [imageFailed, setImageFailed] = useState(false);
  useEffect(() => setImageFailed(false), [artifact.id, artifact.uri]);

  if (isMarkdownDoc(artifact)) {
    if (loading) {
      return (
        <div className="grid min-h-full place-items-center text-sm text-ink-faint">
          <span className="flex items-center gap-2"><Loader2 className="size-4 animate-spin" /> Loading document…</span>
        </div>
      );
    }
    if (text == null) return <GenericPreview artifact={artifact} />;
    return (
      <article
        className={cn(
          "mx-auto w-full px-8 py-8 lg:px-12 lg:py-10",
          presenting ? "max-w-5xl" : "max-w-4xl",
        )}
      >
        <div
          className={cn(
            "text-base leading-[1.7]",
            MD_CLASSES,
            "[&_h1]:font-display [&_h1]:text-2xl [&_h1]:font-semibold [&_h1]:tracking-[-0.025em]",
            "[&_h2]:mt-8 [&_h2]:font-display [&_h2]:text-xl [&_h2]:font-semibold",
            "[&_h3]:mt-6 [&_h3]:font-display [&_h3]:text-lg",
            "[&_table]:mt-4 [&_table]:text-sm",
          )}
        >
          <Md math diagrams>{text}</Md>
        </div>
      </article>
    );
  }

  if (artifact.kind === "image" && artifact.uri && !imageFailed) {
    return (
      <div className="grid min-h-full place-items-center bg-bg-sunken/45 p-8">
        <LocalArtifactImage
          uri={artifact.uri}
          alt={artifact.title}
          className="max-h-full max-w-full rounded-lg object-contain shadow-soft"
          onError={() => setImageFailed(true)}
        />
      </div>
    );
  }

  if (isVideoArtifact(artifact) && artifact.uri) {
    return (
      <div className="flex min-h-full flex-col items-center justify-center gap-3 bg-black p-8">
        {/* Artifact events do not yet carry a captions track; the note below makes that limitation explicit. */}
        {/* eslint-disable-next-line jsx-a11y/media-has-caption */}
        <video src={artifact.uri} controls preload="metadata" className="max-h-[70vh] max-w-full" />
        <p className="text-xs text-white/70">Captions are not available for this video artifact.</p>
      </div>
    );
  }

  return <GenericPreview artifact={artifact} />;
}

function ContextPopover({
  panel,
  artifact,
  conversationTitle,
  sourceCall,
  onJumpToSource,
  onClose,
}: {
  panel: ContextPanel;
  artifact: Artifact;
  conversationTitle: string;
  sourceCall?: ToolCall;
  onJumpToSource: () => void;
  onClose: () => void;
}) {
  const heading = panel[0].toUpperCase() + panel.slice(1);
  const availability = artifactAvailability(artifact);
  const location = readableArtifactLocation(artifact);
  return (
    <section
      aria-label={`${heading} for ${artifact.title}`}
      className="absolute bottom-3 left-3 right-[4.25rem] top-auto z-20 max-h-[calc(100%-1.5rem)] w-auto overflow-auto rounded-xl border border-border bg-bg-elevated shadow-lifted xl:bottom-auto xl:left-auto xl:right-14 xl:top-28 xl:w-[19rem]"
    >
      <header className="flex h-11 items-center gap-2 border-b border-border-subtle px-3.5">
        {panel === "source" ? <Link2 className="size-3.5" /> : panel === "details" ? <Info className="size-3.5" /> : panel === "versions" ? <History className="size-3.5" /> : <MessageCircle className="size-3.5" />}
        <h2 className="text-sm font-semibold text-ink">{heading}</h2>
        <button
          type="button"
          onClick={onClose}
          aria-label={`Close ${heading.toLowerCase()}`}
          className="ml-auto grid size-7 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink"
        >
          <X className="size-3.5" />
        </button>
      </header>
      {panel === "source" && (
        <div className="space-y-3 p-4">
          <div>
            <div className="text-sm font-semibold text-ink">{conversationTitle}</div>
            <div className="mt-2 flex items-start gap-2 text-xs leading-relaxed text-ink-muted">
              <Sparkles className="mt-0.5 size-3.5 shrink-0 text-accent" />
              <span>{sourceTitle(sourceCall)}</span>
            </div>
          </div>
          <button
            type="button"
            onClick={onJumpToSource}
            className="flex h-10 w-full items-center justify-center gap-2 rounded-lg bg-accent px-3 text-sm font-medium text-on-accent transition hover:bg-accent-hover"
          >
            Jump to source message <ExternalLink className="size-3.5" />
          </button>
        </div>
      )}
      {panel === "details" && (
        <dl className="space-y-3 p-4 text-xs">
          <div className="flex items-center justify-between gap-4"><dt className="text-ink-faint">Kind</dt><dd className="text-right text-ink-secondary">{isMarkdownDoc(artifact) ? "Markdown" : KIND_LABEL[artifact.kind]}</dd></div>
          <div className="flex items-center justify-between gap-4"><dt className="text-ink-faint">Location</dt><dd className="text-right text-ink-secondary">{artifactLocationLabel(artifact)}</dd></div>
          <div className="flex items-center justify-between gap-4">
            <dt className="text-ink-faint">Status</dt>
            <dd className={cn("flex items-center gap-1.5", availability === "unavailable" ? "text-ink-muted" : "text-success")}>
              <span className={cn("size-1.5 rounded-full", availability === "unavailable" ? "bg-ink-faint" : "bg-success")} />
              {availability === "unavailable" ? "Unavailable" : "Ready"}
            </dd>
          </div>
          {location && <div className="max-h-24 overflow-auto break-all border-t border-border-subtle pt-3 font-mono text-[11px] leading-relaxed text-ink-faint">{location}</div>}
        </dl>
      )}
      {panel === "versions" && (
        <div className="p-4">
          <div className="rounded-lg border border-border-subtle bg-bg-primary p-3">
            <div className="flex items-center gap-2 text-sm font-medium text-ink">
              <span className={cn("size-2 rounded-full", availability === "unavailable" ? "bg-ink-faint" : "bg-success")} />
              {availability === "unavailable" ? "No available version" : "Current"}
            </div>
            <p className="mt-1 pl-4 text-xs text-ink-faint">
              {availability === "unavailable" ? "Waiting for a file or link" : "Latest available artifact"}
            </p>
          </div>
          <p className="mt-3 text-xs leading-relaxed text-ink-faint">Earlier versions will appear here when the artifact contract includes revision history.</p>
        </div>
      )}
      {panel === "comments" && (
        <div className="p-5 text-center">
          <MessageCircle className="mx-auto size-5 text-ink-faint" />
          <p className="mt-2 text-sm font-medium text-ink-secondary">No comments yet</p>
          <p className="mt-1 text-xs leading-relaxed text-ink-faint">Comments stay attached to this artifact while you work.</p>
        </div>
      )}
    </section>
  );
}

export function ArtifactWorkspace({
  artifacts,
  activeArtifactId,
  conversationTitle,
  toolCalls,
  onSelect,
  onClose,
  onJumpToSource,
}: {
  artifacts: Artifact[];
  activeArtifactId: string;
  conversationTitle: string;
  toolCalls: Record<string, ToolCall>;
  onSelect: (id: string) => void;
  onClose: () => void;
  onJumpToSource: (artifact: Artifact) => void;
}) {
  const [openArtifactIds, setOpenArtifactIds] = useState<Set<string>>(
    () => new Set([activeArtifactId]),
  );
  const [pickerOpen, setPickerOpen] = useState(false);
  const [contextPanel, setContextPanel] = useState<ContextPanel | null>(null);
  const [text, setText] = useState<string | null>(null);
  const [loadingText, setLoadingText] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [saved, setSaved] = useState(false);
  const [presenting, setPresenting] = useState(false);
  const [copied, copy] = useCopy();
  const pickerContainerRef = useRef<HTMLDivElement>(null);
  const pickerMenuRef = useRef<HTMLDivElement>(null);
  const pickerTriggerRef = useRef<HTMLButtonElement>(null);

  const openArtifacts = useMemo(
    () => artifacts.filter((artifact) => openArtifactIds.has(artifact.id)),
    [artifacts, openArtifactIds],
  );
  const active = openArtifacts.find((artifact) => artifact.id === activeArtifactId) ?? openArtifacts[0];
  const activeId = active?.id;
  const activeUri = active?.uri;
  const activeIsMarkdown = active ? isMarkdownDoc(active) : false;
  const sourceCall = active?.tool_call ? toolCalls[active.tool_call] : undefined;
  const byteSize = text == null ? null : new TextEncoder().encode(text).byteLength;

  useEffect(() => {
    setOpenArtifactIds((current) => {
      if (current.has(activeArtifactId)) return current;
      const next = new Set(current);
      next.add(activeArtifactId);
      return next;
    });
  }, [activeArtifactId]);

  useEffect(() => {
    setText(null);
    setSaved(false);
    setPresenting(false);
    if (!activeId || !activeIsMarkdown) {
      setLoadingText(false);
      return;
    }
    let alive = true;
    setLoadingText(true);
    readDocText(activeUri).then((value) => {
      if (!alive) return;
      setText(value);
      setLoadingText(false);
    });
    return () => {
      alive = false;
    };
  }, [activeId, activeIsMarkdown, activeUri]);

  useEffect(() => {
    if (!pickerOpen) return;
    const frame = requestAnimationFrame(() => {
      pickerMenuRef.current?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [pickerOpen]);

  useEffect(() => {
    const closePickerOnOutsideClick = (event: PointerEvent) => {
      if (pickerOpen && !pickerContainerRef.current?.contains(event.target as Node)) setPickerOpen(false);
    };
    const closeTransientUi = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (pickerOpen) {
        event.preventDefault();
        setPickerOpen(false);
        pickerTriggerRef.current?.focus();
        return;
      }
      if (contextPanel) {
        event.preventDefault();
        setContextPanel(null);
        return;
      }
      if (presenting) {
        event.preventDefault();
        setPresenting(false);
      }
    };
    document.addEventListener("pointerdown", closePickerOnOutsideClick);
    document.addEventListener("keydown", closeTransientUi);
    return () => {
      document.removeEventListener("pointerdown", closePickerOnOutsideClick);
      document.removeEventListener("keydown", closeTransientUi);
    };
  }, [contextPanel, pickerOpen, presenting]);

  if (!active) return null;

  const activeTabId = artifactDomId("artifact-tab", active.id);
  const activePanelId = artifactDomId("artifact-panel", active.id);
  const availability = artifactAvailability(active);
  const availabilityLabel = availability === "saved" ? "Saved" : availability === "available" ? "Available" : "Unavailable";

  const closeTab = (artifact: Artifact) => {
    const next = openArtifacts.filter((item) => item.id !== artifact.id);
    if (next.length === 0) {
      onClose();
      return;
    }
    setOpenArtifactIds((current) => {
      const updated = new Set(current);
      updated.delete(artifact.id);
      return updated;
    });
    if (artifact.id === active.id) onSelect(next[Math.max(0, openArtifacts.indexOf(artifact) - 1)]?.id ?? next[0].id);
  };

  const download = () => {
    if (text == null || downloading) return;
    setDownloading(true);
    void saveDocText(text, active.title)
      .then((ok) => {
        if (ok) {
          setSaved(true);
          setTimeout(() => setSaved(false), 1800);
        }
      })
      .finally(() => setDownloading(false));
  };

  return (
    <section
      aria-label={presenting ? `Presenting ${active.title}` : "Artifact workspace"}
      role={presenting ? "dialog" : undefined}
      aria-modal={presenting || undefined}
      className={cn("relative flex min-w-0 flex-1 flex-col bg-bg-elevated", presenting && "fixed inset-0 z-50")}
    >
      <div className={cn("flex h-10 shrink-0 items-stretch border-b border-border-subtle bg-bg-secondary/45", presenting && "hidden")}>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close artifact workspace"
          title="Close artifact workspace"
          className="grid w-10 shrink-0 place-items-center border-r border-border-subtle text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <ChevronLeft className="size-4" />
        </button>
        <div role="tablist" aria-label="Open artifacts" className="flex min-w-0 flex-1 overflow-x-auto">
          {openArtifacts.map((artifact) => (
            <ArtifactTab
              key={artifact.id}
              artifact={artifact}
              active={artifact.id === active.id}
              tabId={artifactDomId("artifact-tab", artifact.id)}
              panelId={artifactDomId("artifact-panel", artifact.id)}
              onSelect={() => onSelect(artifact.id)}
              onClose={() => closeTab(artifact)}
            />
          ))}
        </div>
        <div ref={pickerContainerRef} className="relative flex shrink-0 items-center border-l border-border-subtle px-1">
          <button
            ref={pickerTriggerRef}
            type="button"
            onClick={() => setPickerOpen((open) => !open)}
            aria-label="Open another artifact"
            aria-expanded={pickerOpen}
            aria-haspopup="menu"
            aria-controls="artifact-picker-menu"
            className="grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink"
          >
            <Plus className="size-4" />
          </button>
          <button
            type="button"
            onClick={() => setPickerOpen((open) => !open)}
            aria-label="Show artifact list"
            aria-expanded={pickerOpen}
            aria-haspopup="menu"
            aria-controls="artifact-picker-menu"
            className="grid size-7 place-items-center rounded-lg text-ink-faint transition hover:bg-bg-hover hover:text-ink"
          >
            <ChevronDown className="size-3.5" />
          </button>
          {pickerOpen && (
            <div
              ref={pickerMenuRef}
              id="artifact-picker-menu"
              role="menu"
              aria-label="Artifacts in this task"
              className="absolute right-1 top-9 z-30 w-72 overflow-hidden rounded-xl border border-border bg-bg-elevated p-1.5 shadow-lifted"
            >
              <div className="px-2 py-1.5 text-xs font-medium text-ink-faint">Artifacts in this task</div>
              {artifacts.map((artifact) => {
                const Icon = isMarkdownDoc(artifact) ? FileText : (KIND_ICON[artifact.kind] ?? FileBox);
                return (
                  <button
                    key={artifact.id}
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setOpenArtifactIds((current) => {
                        const next = new Set(current);
                        next.add(artifact.id);
                        return next;
                      });
                      onSelect(artifact.id);
                      setPickerOpen(false);
                    }}
                    className="flex w-full items-center gap-2 rounded-lg px-2 py-2 text-left text-xs text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
                  >
                    <Icon className="size-3.5 shrink-0 text-ink-faint" />
                    <span className="min-w-0 flex-1 truncate">{artifact.title}</span>
                    <span className="text-[11px] text-ink-faint">{KIND_LABEL[artifact.kind]}</span>
                  </button>
                );
              })}
            </div>
          )}
        </div>
      </div>

      <div className="flex h-12 shrink-0 items-center gap-2 border-b border-border-subtle px-4">
        <FileText className="size-3.5 text-ink-faint" />
        <span className="text-xs font-medium text-ink-secondary">Artifacts</span>
        <span className="text-ink-faint">/</span>
        <span className="text-xs text-ink-muted">{isMarkdownDoc(active) ? "Markdown" : KIND_LABEL[active.kind]}</span>
        <span className="ml-2 hidden items-center gap-1.5 text-xs text-ink-faint xl:flex">
          {availability === "unavailable" ? <X className="size-3.5" /> : <Check className="size-3.5 text-success" />}
          {availabilityLabel}
          {byteSize != null && <><span className="mx-1">·</span>{formatBytes(byteSize)}</>}
        </span>
        <div className="ml-auto flex items-center gap-0.5">
          {text != null && (
            <button
              type="button"
              onClick={() => copy(text)}
              className="flex h-8 items-center gap-1.5 rounded-lg px-2 text-xs text-ink-muted transition hover:bg-bg-hover hover:text-ink"
            >
              {copied ? <Check className="size-3.5 text-success" /> : <Copy className="size-3.5" />} {copied ? "Copied" : "Copy"}
            </button>
          )}
          {text != null && (
            <button
              type="button"
              onClick={download}
              disabled={downloading}
              className="flex h-8 items-center gap-1.5 rounded-lg px-2 text-xs text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:opacity-50"
            >
              {downloading ? <Loader2 className="size-3.5 animate-spin" /> : saved ? <Check className="size-3.5 text-success" /> : <Download className="size-3.5" />} {saved ? "Saved" : "Download"}
            </button>
          )}
          {text != null && (
            <button
              type="button"
              onClick={() => {
                setContextPanel(null);
                setPresenting((value) => !value);
              }}
              aria-pressed={presenting}
              aria-label={presenting ? "Exit presentation" : "Present artifact"}
              className={cn(
                "flex h-8 items-center gap-1.5 rounded-lg px-2 text-xs transition",
                presenting ? "bg-accent-soft text-accent" : "text-ink-muted hover:bg-bg-hover hover:text-ink",
              )}
            >
              {presenting ? <AlignLeft className="size-3.5" /> : <Presentation className="size-3.5" />} {presenting ? "Exit presentation" : "Present"}
            </button>
          )}
          {canOpenArtifactExternally(active) && (
            <button
              type="button"
              onClick={() => void openExternal(active.uri!)}
              aria-label={`View ${active.title} externally`}
              title={`View ${active.title} externally`}
              className="grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink"
            >
              <ExternalLink className="size-3.5" />
            </button>
          )}
        </div>
      </div>

      <div className="relative flex min-h-0 flex-1">
        <div
          id={activePanelId}
          role="tabpanel"
          aria-labelledby={activeTabId}
          tabIndex={0}
          className="min-w-0 flex-1 overflow-y-auto outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent"
        >
          <ArtifactPreview artifact={active} text={text} loading={loadingText} presenting={presenting} />
        </div>
        <nav aria-label="Artifact context" className={cn("flex w-14 shrink-0 flex-col items-center gap-1 border-l border-border-subtle bg-bg-primary py-2", presenting && "hidden")}>
          {([
            ["details", Info, "Details"],
            ["versions", Clock3, "Versions"],
            ["comments", MessageCircle, "Comments"],
            ["source", Link2, "Source"],
          ] as const).map(([value, Icon, label]) => (
            <button
              key={value}
              type="button"
              onClick={() => setContextPanel((current) => current === value ? null : value)}
              aria-pressed={contextPanel === value}
              className={cn(
                "flex w-12 flex-col items-center gap-1 rounded-lg py-2 text-[10px] leading-none transition",
                contextPanel === value ? "bg-accent-soft text-accent" : "text-ink-faint hover:bg-bg-hover hover:text-ink-secondary",
              )}
            >
              <Icon className="size-4" />
              {label}
            </button>
          ))}
          <button type="button" onClick={onClose} aria-label="Close artifact workspace" className="mt-auto grid size-9 place-items-center rounded-lg text-ink-faint transition hover:bg-bg-hover hover:text-ink"><PanelRightClose className="size-4" /></button>
        </nav>
        {contextPanel && (
          <ContextPopover
            panel={contextPanel}
            artifact={active}
            conversationTitle={conversationTitle}
            sourceCall={sourceCall}
            onJumpToSource={() => onJumpToSource(active)}
            onClose={() => setContextPanel(null)}
          />
        )}
      </div>
    </section>
  );
}
