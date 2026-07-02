import { useEffect, useState } from "react";
import { motion, useReducedMotion } from "motion/react";
import {
  Globe, Film, FileText, Image as ImageIcon, Presentation, ExternalLink, FileBox,
} from "lucide-react";
import type { Artifact, ArtifactKind } from "../../core-bridge/types";
import { MarkdownDoc, isMarkdownDoc } from "./MarkdownDoc";
import { isLocalDocUri, readImageDataUrl } from "../../lib/docs";

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
 *  (Clark cloud — usable directly) or a local absolute path (a local-agent
 *  screenshot — no `assetProtocol` scope is configured, so it's fetched as a
 *  `data:` URL on demand, mirroring `MarkdownDoc`'s async-fetch idiom for
 *  `read_doc_text`). */
function LocalImage({
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
  return <img src={src} alt={alt} className={className} onError={onError} />;
}

/** Renders a produced artifact inline in the conversation. */
export function ArtifactCard({ artifact }: { artifact: Artifact }) {
  const reduce = useReducedMotion();
  const [broke, setBroke] = useState(false);

  // Markdown documents render as a rich inline viewer (scrollable doc + a slide
  // "present" mode) rather than a bare "Open" card.
  if (isMarkdownDoc(artifact)) return <MarkdownDoc artifact={artifact} />;

  const Icon = KIND_ICON[artifact.kind] ?? FileBox;
  const uri = artifact.uri;

  // Only embed media we can actually render inline (images, video). Websites are
  // never iframed — local/preview URLs and X-Frame-Options make that an unreliable
  // broken box; we show a compact card with an "Open" link instead. Any media that
  // fails to load collapses to the same compact card.
  const body = (() => {
    if (broke) return null;
    if (artifact.kind === "image" && uri) {
      return (
        <LocalImage
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
          className="max-h-80 w-full bg-black"
          onError={() => setBroke(true)}
        />
      );
    }
    return null;
  })();

  return (
    <motion.div
      initial={reduce ? false : { opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2 }}
      className="overflow-hidden rounded-lg border border-border bg-bg-elevated"
    >
      <header className="flex items-center gap-2 px-3 py-2">
        <Icon className="size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium text-ink">{artifact.title}</div>
          <div className="text-[0.7rem] uppercase tracking-wider text-ink-faint">
            {KIND_LABEL[artifact.kind]}
          </div>
        </div>
        {uri && (
          <a
            href={uri}
            target="_blank"
            rel="noreferrer noopener"
            className="flex shrink-0 items-center gap-1 rounded-lg bg-accent px-2.5 py-1.5 text-xs font-medium text-on-accent transition hover:bg-accent-hover"
          >
            Open <ExternalLink className="size-3" />
          </a>
        )}
      </header>
      {body && <div className="border-t border-border-subtle">{body}</div>}
    </motion.div>
  );
}
