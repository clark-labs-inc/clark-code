import { useEffect, useState } from "react";
import { useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  Globe, Film, FileText, Image as ImageIcon, Presentation, ExternalLink, FileBox,
  ArrowUpRight,
} from "lucide-react";
import type { Artifact, ArtifactKind } from "../../core-bridge/types";
import { MarkdownDoc, isMarkdownDoc } from "./MarkdownDoc";
import { isLocalDocUri, readImageDataUrl } from "../../lib/docs";
import { RISE, accessibleMotion } from "../../lib/motion";

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
  const [src, setSrc] = useState<string | null>(isLocalDocUri(uri) ? null : uri);

  useEffect(() => {
    if (!isLocalDocUri(uri)) {
      setSrc(uri);
      return;
    }
    let alive = true;
    setSrc(null);
    readImageDataUrl(uri).then((data) => {
      if (!alive) return;
      if (data == null) onError();
      else setSrc(data);
    });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [uri]);

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
  active = false,
  onOpen,
}: {
  artifact: Artifact;
  active?: boolean;
  onOpen?: (artifact: Artifact) => void;
}) {
  const reduce = useReducedMotion();
  const [broke, setBroke] = useState(false);
  const uri = artifact.uri;

  useEffect(() => setBroke(false), [artifact.id, uri]);

  // Markdown stays compact in chat; its full reader lives in the workspace.
  if (isMarkdownDoc(artifact)) {
    return <MarkdownDoc artifact={artifact} active={active} onOpen={onOpen} />;
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
          className="max-h-80 w-full object-contain bg-bg-sunken"
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
          className="max-h-80 w-full bg-black"
          onError={() => setBroke(true)}
        />
      );
    }
    return null;
  })();

  return (
    <m.div
      {...accessibleMotion(RISE, reduce)}
      className={`overflow-hidden border-y bg-transparent ${
        active ? "border-accent shadow-[inset_3px_0_0_var(--color-accent)]" : "border-border-subtle"
      }`}
    >
      <header className="flex items-center gap-2 px-3 py-2">
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
        {uri && !onOpen && (
          <a
            href={uri}
            target="_blank"
            rel="noreferrer noopener"
            className="flex shrink-0 items-center gap-1 rounded-lg bg-bg-secondary px-2.5 py-1.5 text-xs font-medium text-ink-secondary ring-1 ring-border-subtle transition hover:bg-bg-hover hover:text-ink"
          >
            View {artifact.title} <ExternalLink className="size-3" />
          </a>
        )}
      </header>
      {body && <div className="border-t border-border-subtle">{body}</div>}
    </m.div>
  );
}
