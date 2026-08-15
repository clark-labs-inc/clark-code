import { useEffect, useState } from "react";
import { useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  Globe, Film, FileText, Image as ImageIcon, Presentation, FileBox,
  ArrowUpRight, Download,
} from "lucide-react";
import type { Artifact, ArtifactKind } from "../../core-bridge/types";
import { MarkdownDoc, isMarkdownDoc } from "./MarkdownDoc";
import { isLocalDocUri, readImageDataUrl } from "../../lib/docs";
import { RISE, accessibleMotion } from "../../lib/motion";
import { ArtifactFileActions } from "./ArtifactFileActions";
import { useSessionStore } from "../../store/sessionStore";
import { saveArtifactCopy } from "../../lib/fileLinks";

const KIND_ICON: Record<ArtifactKind, typeof Globe> = {
  website: Globe, video: Film, media: Film, image: ImageIcon,
  pdf: FileText, office: FileText, slides: Presentation,
  file: FileText, diff: FileText, search_results: FileText, other: FileBox,
};

const KIND_LABEL: Record<ArtifactKind, string> = {
  website: "Website", video: "Video", media: "Media", image: "Image",
  pdf: "PDF", office: "Document", slides: "Slides",
  file: "File", diff: "Diff", search_results: "Search", other: "Artifact",
};

function isVideo(a: Artifact): boolean {
  return a.kind === "video" || (a.kind === "media" && /video|\.(mp4|webm|mov)/i.test(a.uri ?? a.mime_type ?? ""));
}

function artifactFormatLabel(artifact: Artifact): string {
  const extension = artifact.title.match(/\.([a-z0-9]{2,5})(?:[?#]|$)/i)?.[1];
  if (extension) return extension.toUpperCase();
  return KIND_LABEL[artifact.kind];
}

function artifactDisplayTitle(artifact: Artifact): string {
  const withoutExtension = artifact.title.replace(/\.[a-z0-9]{2,5}(?:[?#]|$)/i, "");
  const words = withoutExtension.replace(/[-_]+/g, " ").replace(/\s+/g, " ").trim();
  if (!words) return artifact.title;
  return words.replace(/\b\p{L}/gu, (letter) => letter.toLocaleUpperCase());
}

/** Renders an artifact image `uri`, which may be a remote `http(s)` URL
 *  (product cloud — usable directly) or a local absolute path (a local-agent
 *  screenshot — no `assetProtocol` scope is configured, so it's fetched as a
 *  `data:` URL on demand, mirroring `MarkdownDoc`'s async-fetch idiom for
 *  `read_doc_text`). */
export function LocalArtifactImage({
  uri, alt, className, onError,
}: {
  uri: string; alt: string; className: string; onError: () => void;
}) {
  const sessionId = useSessionStore((state) => state.session?.id);
  const [src, setSrc] = useState<string | null>(isLocalDocUri(uri) ? null : uri);

  useEffect(() => {
    if (!isLocalDocUri(uri)) {
      setSrc(uri);
      return;
    }
    let alive = true;
    setSrc(null);
    readImageDataUrl(uri, sessionId).then((data) => {
      if (!alive) return;
      if (data == null) onError();
      else setSrc(data);
    });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId, uri]);

  if (!src) return null;
  return (
    <img
      src={src}
      alt={alt}
      loading="lazy"
      decoding="async"
      className={className}
      onError={onError}
    />
  );
}

/** Renders a produced artifact inline in the conversation. */
export function ArtifactCard({
  artifact,
  onOpen,
}: {
  artifact: Artifact;
  onOpen?: (artifact: Artifact) => void;
}) {
  const reduce = useReducedMotion();
  const [broke, setBroke] = useState(false);
  const flashNotice = useSessionStore((state) => state.flashNotice);
  const uri = artifact.uri;

  useEffect(() => setBroke(false), [artifact.id, uri]);

  // Markdown stays compact in chat; its full reader lives in the workspace.
  if (isMarkdownDoc(artifact)) {
    return <MarkdownDoc artifact={artifact} onOpen={onOpen} />;
  }

  const Icon = KIND_ICON[artifact.kind] ?? FileBox;

  // Only embed media we can actually render inline (images, video). Websites are
  // never iframed — local/preview URLs and X-Frame-Options make that an unreliable
  // broken box; we show a compact card with a workspace action instead. Any media that
  // fails to load collapses to the same compact card.
  const body = (() => {
    if (broke) return null;
    if (artifact.kind === "image" && uri) {
      return (
        <LocalArtifactImage
          uri={uri}
          alt={artifact.title}
          className="max-h-[28rem] w-full rounded-2xl object-contain bg-bg-sunken"
          onError={() => setBroke(true)}
        />
      );
    }
    if (isVideo(artifact) && uri) {
      return (
        // eslint-disable-next-line jsx-a11y/media-has-caption
        <video
          src={uri}
          controls
          preload="metadata"
          className="max-h-[28rem] w-full rounded-2xl bg-black"
          onError={() => setBroke(true)}
        />
      );
    }
    return null;
  })();

  if (body && uri) {
    const displayTitle = artifactDisplayTitle(artifact);
    const saveCopy = () => {
      void saveArtifactCopy(uri, artifact.title).then(
        (saved) => {
          if (saved) flashNotice("Artifact copy saved.");
        },
        (error: unknown) => {
          const detail = error instanceof Error ? error.message : String(error);
          flashNotice(`Could not save ${artifact.title}: ${detail}`);
        },
      );
    };

    return (
      <m.article
        {...accessibleMotion(RISE, reduce)}
        className="py-4"
        data-qa="artifact-canvas-drop"
      >
        <header className="flex min-w-0 items-center gap-2 px-1 pb-3 transition-colors group-focus-visible/artifact:text-ink">
          <Icon className="size-4 shrink-0 text-accent" />
          <div className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <h3 className="truncate text-sm font-semibold text-ink">{displayTitle}</h3>
            <span className="text-xs uppercase tracking-wider text-ink-faint">
              {artifactFormatLabel(artifact)}
            </span>
            <span aria-hidden="true" className="text-border-strong">·</span>
            <span className="text-xs text-ink-muted">Ready to review</span>
            {displayTitle !== artifact.title && (
              <span className="text-xs text-ink-faint">
                from <span className="text-accent">{artifact.title}</span>
              </span>
            )}
          </div>
        </header>

        <div className="relative">
          <div className="overflow-hidden rounded-2xl bg-bg-sunken shadow-[0_22px_60px_-42px_rgba(0,0,0,0.9)]">
            {body}
          </div>

          <div className="relative z-10 -mt-5 flex flex-wrap justify-end gap-2 px-4">
            {onOpen && (
              <button
                type="button"
                onClick={() => onOpen(artifact)}
                aria-label={`Open ${artifact.title} in workspace`}
                className="flex h-10 items-center gap-2 rounded-xl bg-accent px-4 text-sm font-semibold text-on-accent shadow-[0_12px_28px_-16px_var(--color-accent)] transition duration-fast ease-agent hover:bg-accent-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-focus"
              >
                <ArrowUpRight className="size-4" /> Open workspace
              </button>
            )}
            <button
              type="button"
              onClick={saveCopy}
              aria-label={`Save a copy of ${artifact.title}`}
              className="flex h-10 items-center gap-2 rounded-xl bg-bg-elevated px-4 text-sm font-semibold text-ink-secondary ring-1 ring-border-subtle shadow-[0_12px_28px_-18px_rgba(0,0,0,0.85)] transition duration-fast ease-agent hover:bg-bg-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-focus"
            >
              <Download className="size-4" /> Save a copy
            </button>
          </div>
        </div>
      </m.article>
    );
  }

  return (
    <m.div
      {...accessibleMotion(RISE, reduce)}
      className="overflow-hidden border-y border-border-subtle bg-transparent"
    >
      <header className="flex items-center gap-2 px-3 py-2 transition-colors group-focus-visible/artifact:bg-accent-subtle">
        <Icon className="size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium text-ink">{artifact.title}</div>
          <div className="text-xs uppercase tracking-wider text-ink-faint">
            {KIND_LABEL[artifact.kind]}
          </div>
        </div>
        {onOpen && (
          <button
            type="button"
            onClick={() => onOpen(artifact)}
            aria-label={`View ${artifact.title}`}
            className="flex shrink-0 items-center gap-1.5 rounded-lg bg-bg-secondary px-2.5 py-1.5 text-xs font-medium text-ink-secondary ring-1 ring-border-subtle transition hover:bg-bg-hover hover:text-ink"
          >
            View <ArrowUpRight className="size-3" />
          </button>
        )}
      </header>
      {body && <div className="border-t border-border-subtle">{body}</div>}
      {uri && (
        <ArtifactFileActions
          artifact={artifact}
          compact
          className="border-t border-border-subtle px-2 py-1.5"
        />
      )}
    </m.div>
  );
}
