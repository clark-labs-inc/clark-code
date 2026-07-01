import { useEffect, useMemo, useState } from "react";
import { motion, useReducedMotion } from "motion/react";
import {
  FileText, ExternalLink, Copy, Check, Presentation, AlignLeft, ChevronLeft, ChevronRight,
} from "lucide-react";
import type { Artifact } from "../../core-bridge/types";
import { cn } from "../../lib/cn";
import { useCopy } from "../../lib/clipboard";
import { readDocText, isLocalDocUri } from "../../lib/docs";
import { Md, MD_CLASSES } from "../Message";

/** True for an artifact we render as an inline markdown document. */
export function isMarkdownDoc(a: Artifact): boolean {
  if (a.mime_type === "text/markdown") return true;
  const name = `${a.title ?? ""} ${a.uri ?? ""}`.toLowerCase();
  return /\.(md|markdown|mdx)(?:[?#]|\s|$)/.test(name);
}

/** Split a document into slides on thematic-break lines (`---`, `***`, `___`)
 *  that stand alone between blank lines — the slide-deck convention. Requiring a
 *  leading blank line avoids splitting on a Setext `---` heading underline. */
function splitSlides(md: string): string[] {
  const parts = md
    .split(/\n[ \t]*\n[ \t]*(?:-{3,}|\*{3,}|_{3,})[ \t]*(?:\n|$)/)
    .map((s) => s.trim())
    .filter(Boolean);
  return parts.length > 1 ? parts : [md.trim()];
}

/** Renders a produced markdown file inline: a scrollable rendered document with a
 *  "Present" toggle that pages through it as slides. Reads the file off disk; if
 *  it can't (browser preview / remote URL / unreadable), shows a compact card. */
export function MarkdownDoc({ artifact }: { artifact: Artifact }) {
  const reduce = useReducedMotion();
  const [text, setText] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [present, setPresent] = useState(false);
  const [slide, setSlide] = useState(0);
  const [copied, copy] = useCopy();
  const uri = artifact.uri;

  useEffect(() => {
    let alive = true;
    setText(null);
    setFailed(false);
    setSlide(0);
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

  const slides = useMemo(() => splitSlides(text ?? ""), [text]);
  const multi = slides.length > 1;
  const at = Math.min(slide, slides.length - 1);
  const external = !!uri && !isLocalDocUri(uri);

  return (
    <motion.div
      initial={reduce ? false : { opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2 }}
      className="overflow-hidden rounded-lg border border-border bg-bg-elevated"
    >
      <header className="flex items-center gap-2 px-3 py-2">
        <FileText className="size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium text-ink">{artifact.title}</div>
          <div className="text-[0.7rem] uppercase tracking-wider text-ink-faint">
            {present && multi ? `Slide ${at + 1} / ${slides.length}` : "Markdown"}
          </div>
        </div>
        {text != null && (
          <button
            type="button"
            onClick={() => copy(text)}
            aria-label={copied ? "Copied" : "Copy as Markdown"}
            title={copied ? "Copied" : "Copy as Markdown"}
            className="grid size-7 shrink-0 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink-secondary"
          >
            {copied ? <Check className="size-3.5 text-success" /> : <Copy className="size-3.5" />}
          </button>
        )}
        {text != null && multi && (
          <button
            type="button"
            onClick={() => setPresent((p) => !p)}
            aria-label={present ? "Read as document" : "Present as slides"}
            title={present ? "Read as document" : "Present as slides"}
            className="grid size-7 shrink-0 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink-secondary"
          >
            {present ? <AlignLeft className="size-3.5" /> : <Presentation className="size-3.5" />}
          </button>
        )}
        {external && (
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

      {text == null ? (
        <div className="border-t border-border-subtle px-3 py-3 text-xs text-ink-faint">
          {failed ? "Preview unavailable." : "Loading…"}
        </div>
      ) : present && multi ? (
        <div className="border-t border-border-subtle">
          <div className={cn("min-h-[8rem] px-4 py-4 text-[0.9375rem] leading-relaxed", MD_CLASSES)}>
            <Md>{slides[at]}</Md>
          </div>
          <div className="flex items-center justify-between border-t border-border-subtle px-3 py-1.5">
            <button
              type="button"
              onClick={() => setSlide((s) => Math.max(0, s - 1))}
              disabled={at === 0}
              aria-label="Previous slide"
              className="grid size-7 place-items-center rounded-md text-ink-faint transition enabled:hover:bg-bg-hover enabled:hover:text-ink disabled:opacity-30"
            >
              <ChevronLeft className="size-4" />
            </button>
            <span className="font-mono text-xs text-ink-faint">
              {at + 1} / {slides.length}
            </span>
            <button
              type="button"
              onClick={() => setSlide((s) => Math.min(slides.length - 1, s + 1))}
              disabled={at === slides.length - 1}
              aria-label="Next slide"
              className="grid size-7 place-items-center rounded-md text-ink-faint transition enabled:hover:bg-bg-hover enabled:hover:text-ink disabled:opacity-30"
            >
              <ChevronRight className="size-4" />
            </button>
          </div>
        </div>
      ) : (
        <div
          className={cn(
            "max-h-[30rem] overflow-y-auto border-t border-border-subtle px-4 py-3 text-[0.9375rem] leading-relaxed",
            MD_CLASSES,
          )}
        >
          <Md>{text}</Md>
        </div>
      )}
    </motion.div>
  );
}
