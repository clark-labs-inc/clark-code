import { memo, useEffect, useState, type ReactNode } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { ChevronRight, Copy, Check, FileText, Pencil, Sparkles } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { cn } from "../lib/cn";
import { DUR, EASE } from "../lib/motion";
import { useCopy } from "../lib/clipboard";
import { userAttachmentBlocks, userTextBody } from "../lib/messageBlocks";
import { parseNarration, presentationKind } from "../lib/narration";
import { useSmoothText } from "../lib/useSmoothText";
import { highlight, resolveLang } from "../lib/highlight";
import { markdownUrlTransform } from "../lib/fileLinks";
import { MarkdownLink } from "./MarkdownLink";
import { Mermaid } from "./work/Mermaid";
import type { ContentBlock, MessagePhase, Role } from "../core-bridge/types";

function text(blocks: ContentBlock[]): string {
  return blocks
    .map((b) => {
      if (b.type === "text") return b.text;
      // A native reasoning block (GLM `delta.reasoning`) → wrap in the inline
      // `<thinking>` tag parseNarration splits into a collapsible Thinking row.
      if (b.type === "thinking") return `<thinking>${b.text}</thinking>`;
      return `\`[${b.type}]\``;
    })
    .join("");
}

export const MD_CLASSES =
  "text-ink [&_p]:my-2.5 [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 " +
  "[&_ul]:my-2.5 [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:marker:text-ink-faint [&_ol]:my-2.5 [&_ol]:list-decimal [&_ol]:pl-5 [&_ol]:marker:text-ink-faint [&_li]:my-1 " +
  "[&_h1]:mb-1.5 [&_h1]:mt-4 [&_h1]:text-lg [&_h1]:font-semibold [&_h1]:tracking-tight [&_h2]:mb-1.5 [&_h2]:mt-4 [&_h2]:font-semibold [&_h2]:tracking-tight [&_h3]:mb-1 [&_h3]:mt-3 [&_h3]:font-semibold " +
  "[&_a]:text-ink [&_a]:underline [&_a]:decoration-ink-faint [&_a]:underline-offset-2 hover:[&_a]:decoration-ink [&_strong]:font-semibold [&_strong]:text-ink " +
  "[&_pre]:my-2 [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:border [&_pre]:border-border-subtle [&_pre]:bg-bg-sunken [&_pre]:p-3 [&_pre]:font-mono [&_pre]:text-xs [&_pre]:leading-relaxed [&_pre>code]:bg-transparent [&_pre>code]:p-0 [&_pre>code]:border-0 " +
  "[&_:not(pre)>code]:rounded-[4px] [&_:not(pre)>code]:bg-chip [&_:not(pre)>code]:px-[0.3em] [&_:not(pre)>code]:py-[0.08em] [&_:not(pre)>code]:font-mono [&_:not(pre)>code]:text-[0.84em] [&_:not(pre)>code]:text-ink-secondary " +
  "[&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_blockquote]:text-ink-muted " +
  "[&_table]:my-2 [&_table]:w-full [&_table]:border-collapse [&_table]:table-fixed [&_table]:text-xs " +
  "[&_th]:border [&_th]:border-border-subtle [&_th]:px-2 [&_th]:py-1.5 [&_th]:text-left [&_th]:align-top [&_th]:font-medium [&_th]:text-ink-secondary [&_th]:break-words " +
  "[&_td]:border [&_td]:border-border-subtle [&_td]:px-2 [&_td]:py-1.5 [&_td]:align-top [&_td]:break-words [&_td]:overflow-wrap-anywhere";

/** A small icon button that copies and briefly confirms. */
function CopyButton({
  text,
  className,
  label = "Copy",
}: {
  text: string;
  className?: string;
  label?: string;
}) {
  const [copied, copy] = useCopy();
  return (
    <button
      type="button"
      onClick={() => copy(text)}
      aria-label={copied ? "Copied" : label}
      title={copied ? "Copied" : label}
      className={cn(
        "grid place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink-secondary",
        className,
      )}
    >
      {copied ? <Check className="size-3.5 text-success" /> : <Copy className="size-3.5" />}
    </button>
  );
}

/** Extract the language id and raw text from the `<code>` react-markdown hands
 *  to `pre`. Returns `null` when there's no fenced code child (e.g. indented
 *  code, or an unexpected structure) so the caller can fall back to plain. */
function codeFromPreChild(child: ReactNode): { lang?: string; code: string } | null {
  // react-markdown passes <pre> a single <code> child (or an array containing
  // one); unwrap to that element.
  const el = Array.isArray(child) ? child.find((c) => typeof c === "object" && c !== null && "props" in c) : child;
  if (typeof el !== "object" || el === null || !("props" in el)) return null;
  const props = (el as { props: { className?: string; children?: ReactNode } }).props;
  const className = props.className ?? "";
  const lang = /language-(\S+)/.exec(className)?.[1];
  const inner = props.children;
  const code = typeof inner === "string" ? inner : Array.isArray(inner) ? inner.join("") : "";
  return { lang, code };
}

/** A fenced code block with syntax highlighting and a hover-reveal copy button.
 *  Highlights via the Shiki core singleton (JS regex engine — no WASM); renders
 *  plain monospace while it warms or for an unknown language, then upgrades in
 *  place once the result is ready. */
function CodeBlock({ lang, code }: { lang?: string; code: string }) {
  const [copied, copy] = useCopy();
  const resolved = resolveLang(lang);
  const [html, setHtml] = useState<string | null>(null);
  useEffect(() => {
    if (!resolved) return;
    let alive = true;
    highlight(code, lang).then((r) => {
      if (alive && r.html) setHtml(r.html);
    });
    return () => {
      alive = false;
    };
  }, [code, lang, resolved]);

  return (
    <div className="group/code relative">
      {html ? (
        <div
          className="shiki-host"
          // Shiki emits its own <pre class="shiki …"> with line spans + dual-theme
          // CSS variables. index.css overrides its inline bg with the canonical
          // code surface and switches tokens to --shiki-dark under dark.
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre>{code}</pre>
      )}
      <button
        type="button"
        onClick={() => copy(code)}
        aria-label={copied ? "Copied" : "Copy code"}
        title={copied ? "Copied" : "Copy code"}
        className="absolute right-2 top-2 grid size-7 place-items-center rounded-md bg-bg-elevated text-ink-faint opacity-0 ring-1 ring-border-subtle transition hover:text-ink group-hover/code:opacity-100"
      >
        {copied ? <Check className="size-3.5 text-success" /> : <Copy className="size-3.5" />}
      </button>
    </div>
  );
}

export function Md({ children, math = false, diagrams = false }: {
  children: string;
  math?: boolean;
  diagrams?: boolean;
}) {
  const remark = math ? [remarkGfm, remarkMath] : [remarkGfm];
  const rehype = math ? [rehypeKatex] : [];
  return (
    <Markdown
      remarkPlugins={remark}
      rehypePlugins={rehype}
      urlTransform={markdownUrlTransform}
      components={{
        a: ({ node: _node, ...props }) => <MarkdownLink {...props} />,
        pre: ({ node: _node, children }) => {
          const parsed = codeFromPreChild(children);
          if (parsed) {
            // A ```mermaid fence in a diagrams-enabled surface renders as a
            // diagram (lazy-loaded), not a code block.
            if (diagrams && parsed.lang && /mermaid/i.test(parsed.lang)) {
              return <Mermaid code={parsed.code} />;
            }
            return <CodeBlock lang={parsed.lang} code={parsed.code} />;
          }
          // Indented or non-fenced code — render plainly without highlighting.
          return <pre>{children}</pre>;
        },
        // Wrap tables in a horizontal-scroll container so a wide table scrolls
        // instead of overflowing the message column (fixed layout + wrapping
        // handles most cases, but a many-column table can still exceed the width).
        table: ({ node: _node, ...props }) => (
          <div className="overflow-x-auto">
            <table {...props} />
          </div>
        ),
      }}
    >
      {children}
    </Markdown>
  );
}

/** Memoized markdown: re-parses only when its text actually changes. Used for the
 *  "settled" prefix of a streaming message so the bulk isn't re-parsed per frame. */
const StableMd = memo(function StableMd({ children }: { children: string }) {
  return <Md>{children}</Md>;
});

/** Split streaming markdown into a stable prefix (complete blocks) and a live
 *  tail (the block being written). The boundary is the last blank line that sits
 *  outside any open code fence, so we never cut a fence mid-stream. */
function splitStable(text: string): [string, string] {
  const lines = text.split("\n");
  let fence = false;
  let lastSafe = -1;
  for (let i = 0; i < lines.length; i++) {
    if (/^\s*(```|~~~)/.test(lines[i])) fence = !fence;
    else if (!fence && lines[i].trim() === "") lastSafe = i;
  }
  if (lastSafe <= 0) return ["", text];
  return [lines.slice(0, lastSafe).join("\n"), lines.slice(lastSafe).join("\n")];
}

/** Render markdown while it streams in: the completed-block prefix is memoized
 *  (parsed once), only the trailing in-progress block re-parses each frame. This
 *  turns an O(n²) per-token re-parse of a long answer into ~O(n). */
function StreamingMd({ text }: { text: string }) {
  const [prefix, tail] = splitStable(text);
  return (
    <>
      {prefix && <StableMd>{prefix}</StableMd>}
      <Md>{tail}</Md>
    </>
  );
}

/** Collapsible reasoning ("thinking") block — Manus-style. */
function ThinkingBlock({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  const reduce = useReducedMotion();
  return (
    <div>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="group/think flex items-center gap-1 rounded-md py-0.5 text-xs text-ink-faint transition hover:text-ink-muted"
      >
        <span className="font-medium">Thinking</span>
        <ChevronRight className={cn("size-3 opacity-60 transition-transform group-hover/think:opacity-100", open && "rotate-90")} />
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={reduce ? false : { height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { height: 0, opacity: 0 }}
            transition={{ duration: DUR.base, ease: EASE.inOut }}
            className="overflow-hidden"
          >
            <div className={cn("mt-1 max-h-52 overflow-auto border-l border-border-subtle pl-3 text-xs leading-relaxed text-ink-muted", MD_CLASSES)}>
              <Md>{text}</Md>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** Attachment echoes on a user turn: image thumbnails and quiet file chips,
 *  rendered above the text inside the bubble. */
function UserAttachments({ blocks }: { blocks: ContentBlock[] }) {
  return (
    <div className="flex flex-wrap justify-end gap-1.5">
      {blocks.map((block, i) => {
        if (block.type === "image") {
          return (
            <img
              key={i}
              src={`data:${block.mime_type};base64,${block.data}`}
              alt="Attachment"
              className="max-h-40 max-w-full rounded-xl border border-border-subtle object-cover"
            />
          );
        }
        if (block.type === "resource_link") {
          return (
            <span
              key={i}
              className="flex max-w-full items-center gap-1.5 rounded-lg bg-bg-sunken px-2 py-1 text-xs text-ink-secondary"
            >
              <FileText className="size-3.5 shrink-0 text-ink-muted" />
              <span className="max-w-48 truncate">{block.name ?? block.uri}</span>
            </span>
          );
        }
        if (block.type === "skill_reference") {
          return (
            <span
              key={i}
              className="flex max-w-full items-center gap-1.5 rounded-lg bg-bg-sunken px-2 py-1 text-xs text-ink-secondary"
              title={`Pinned skill revision ${block.revision}`}
            >
              <Sparkles className="size-3.5 shrink-0 text-ink-muted" />
              <span className="max-w-48 truncate">{block.name}</span>
            </span>
          );
        }
        return null;
      })}
    </div>
  );
}

function MessageImpl({
  role,
  blocks,
  phase,
  timelineIndex,
  streaming = false,
}: {
  role: Role;
  blocks: ContentBlock[];
  phase?: MessagePhase;
  timelineIndex: number;
  /** True for the assistant message currently being streamed — enables the
   *  cheaper prefix-memoized markdown path. */
  streaming?: boolean;
}) {
  const reduce = useReducedMotion();
  // User turns carry attachment echo blocks (image / resource_link) alongside
  // the text; keep those out of the flattened copy/edit body.
  const body = role === "user" ? userTextBody(blocks) : text(blocks);
  // Streamed tokens arrive in uneven bursts; reveal them at a steady
  // left-to-right pace so the reply reads as continuous typing. Honors
  // reduced-motion by showing text as it arrives.
  const smoothed = useSmoothText(body, streaming && role === "agent" && !reduce);

  const inner = (() => {
    if (role === "user") {
      // Keep this a quiet right-aligned pill, not a loud accent bubble.
      // Hover reveals copy + edit. The timeline identity lets submit replace
      // this turn and the abandoned suffix instead of appending a duplicate.
      const attachments = userAttachmentBlocks(blocks);
      return (
        <div className="group/user flex items-center justify-end gap-1.5">
          <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition group-hover/user:opacity-100">
            <button
              type="button"
              onClick={() =>
                useSessionStore.getState().setComposerPrefill(body, timelineIndex)
              }
              aria-label="Edit and resend"
              title="Edit & resend"
              className="grid size-7 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink-secondary"
            >
              <Pencil className="size-3.5" />
            </button>
            <CopyButton text={body} label="Copy message" className="size-7" />
          </div>
          <div className="max-w-[80%] whitespace-pre-wrap [overflow-wrap:anywhere] rounded-2xl rounded-br-md border border-border-subtle bg-bg-tertiary px-3.5 py-1.5 text-sm text-ink">
            {attachments.length > 0 && (
              <div className={body ? "mb-1.5" : ""}>
                <UserAttachments blocks={attachments} />
              </div>
            )}
            {body}
          </div>
        </div>
      );
    }
    if (role === "system") {
      return (
        <div className="border-l-2 border-border pl-3 text-sm italic text-ink-muted">{body}</div>
      );
    }
    // Assistant: full-width text (no avatar), split into answer / narration /
    // thinking spans. Always render through `StreamingMd` — the SAME component
    // type whether streaming or settled — so when `streaming` flips off the
    // markdown subtree is NOT unmounted and rebuilt (which flashed a re-parse).
    // `StreamingMd` renders identical DOM to a bare `<Md>` once the text is
    // whole, so the last streamed frame and the settled frame are the same.
    const spans = parseNarration(streaming ? smoothed : body);
    return (
      <div className="min-w-0 space-y-1.5">
        {spans.map((span, i) => {
          const kind = presentationKind(span.kind, phase);
          if (kind === "thinking") return <ThinkingBlock key={i} text={span.text} />;
          const lastSpan = i === spans.length - 1;
          return (
            <div
              key={i}
              className={cn(
                "text-base leading-[1.6] [overflow-wrap:anywhere]",
                MD_CLASSES,
                kind === "narrate" && "text-ink-secondary",
              )}
            >
              <StreamingMd text={span.text} />
              {streaming && lastSpan && (
                <span
                  aria-hidden
                  className="stream-caret ml-0.5 inline-block h-[1em] w-[7px] translate-y-[0.15em] rounded-[1px] bg-accent/80"
                />
              )}
            </div>
          );
        })}
      </div>
    );
  })();

  // The wrapper is stable across the whole message lifecycle: the agent's
  // group/copy container and the content-visibility containment are applied
  // regardless of `streaming`, so nothing remounts or recalculates when the
  // reply settles. The copy button stays hover-only (invisible mid-stream).
  return (
    <motion.div
      initial={reduce ? false : { opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: DUR.base, ease: EASE.out }}
      className="[content-visibility:auto] [contain-intrinsic-size:auto_120px]"
    >
      {role === "agent" ? (
        <div className="group/msg relative">
          {inner}
          <CopyButton
            text={body}
            label="Copy as Markdown"
            className="absolute -top-1 right-0 size-7 bg-bg-elevated opacity-0 ring-1 ring-border-subtle group-hover/msg:opacity-100"
          />
        </div>
      ) : (
        inner
      )}
    </motion.div>
  );
}

/** The host re-emits a fully-cloned snapshot on every streamed token, so without
 *  memoization every message re-parses its markdown each token (the jank). Only
 *  re-render a message when its role or text content actually changes. */
function sameBlocks(a: ContentBlock[], b: ContentBlock[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((blk, i) => {
    const other = b[i];
    if (blk.type !== other.type) return false;
    if (blk.type === "text" || blk.type === "thinking")
      return blk.text === (other as { text: string }).text;
    // Attachment image echoes carry large base64 payloads — compare fields
    // directly instead of JSON.stringify-ing them on every streamed token.
    if (blk.type === "image") {
      const o = other as { mime_type: string; data: string; uri?: string };
      return blk.mime_type === o.mime_type && blk.data === o.data && blk.uri === o.uri;
    }
    return JSON.stringify(blk) === JSON.stringify(other);
  });
}

export const Message = memo(
  MessageImpl,
  (a, b) =>
    a.role === b.role &&
    a.phase === b.phase &&
    a.timelineIndex === b.timelineIndex &&
    a.streaming === b.streaming &&
    sameBlocks(a.blocks, b.blocks),
);
