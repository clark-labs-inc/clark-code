import { useEffect, useMemo, useState } from "react";
import { motion, useReducedMotion } from "motion/react";
import {
  FileText, ExternalLink, ArrowUpRight,
} from "lucide-react";
import type { Artifact } from "../../core-bridge/types";
import { readDocText, isLocalDocUri } from "../../lib/docs";
import { DUR, EASE } from "../../lib/motion";

/** True for an artifact we render as an inline markdown document. */
export function isMarkdownDoc(a: Artifact): boolean {
  if (a.mime_type === "text/markdown") return true;
  const name = `${a.title ?? ""} ${a.uri ?? ""}`.toLowerCase();
  return /\.(md|markdown|mdx)(?:[?#]|\s|$)/.test(name);
}

function excerpt(markdown: string): string {
  return markdown
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/[`*_>\[\]()-]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 220);
}

/** A compact source reference in chat. The full document opens in the artifact
 *  workspace, so the conversation keeps one scroll axis and remains skimmable. */
export function MarkdownDoc({
  artifact,
  active = false,
  onOpen,
}: {
  artifact: Artifact;
  active?: boolean;
  onOpen?: (artifact: Artifact) => void;
}) {
  const reduce = useReducedMotion();
  const [text, setText] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const uri = artifact.uri;

  useEffect(() => {
    let alive = true;
    setText(null);
    setFailed(false);
    readDocText(uri).then(
      (t) => {
        if (!alive) return;
        if (t == null) setFailed(true);
        else setText(t);
      },
      () => alive && setFailed(true),
    );
    return () => {
      alive = false;
    };
  }, [uri]);

  const preview = useMemo(() => (text ? excerpt(text) : ""), [text]);
  const external = !!uri && !isLocalDocUri(uri);

  return (
    <motion.div
      initial={reduce ? false : { opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: DUR.base, ease: EASE.out }}
      className={`overflow-hidden rounded-lg border bg-bg-elevated ${
        active ? "border-accent shadow-[inset_3px_0_0_var(--color-accent)]" : "border-border"
      }`}
    >
      <header className="flex items-center gap-2 px-3 py-2">
        <FileText className="size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium text-ink">{artifact.title}</div>
          <div className="text-xs uppercase tracking-wider text-ink-faint">
            Markdown
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
        {external && !onOpen && (
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

      <div className="border-t border-border-subtle px-3 py-3 text-sm leading-relaxed text-ink-muted">
        {text == null ? (
          failed ? "Preview unavailable. Open the artifact workspace for details." : "Loading preview…"
        ) : (
          <>
            {preview}{text.length > preview.length ? "…" : ""}
          </>
        )}
      </div>
    </motion.div>
  );
}
