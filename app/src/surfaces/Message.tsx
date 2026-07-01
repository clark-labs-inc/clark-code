import { memo, useRef, useState, type ReactNode } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Brain, ChevronRight, Copy, Check } from "lucide-react";
import { cn } from "../lib/cn";
import { useCopy } from "../lib/clipboard";
import { parseNarration } from "../lib/narration";
import type { ContentBlock, Role } from "../core-bridge/types";

function text(blocks: ContentBlock[]): string {
  return blocks.map((b) => (b.type === "text" ? b.text : `\`[${b.type}]\``)).join("");
}

export const MD_CLASSES =
  "text-ink [&_p]:my-2 [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 " +
  "[&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:marker:text-ink-faint [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_ol]:marker:text-ink-faint [&_li]:my-1 " +
  "[&_h1]:mb-1.5 [&_h1]:mt-3 [&_h1]:text-lg [&_h1]:font-semibold [&_h1]:tracking-tight [&_h2]:mb-1.5 [&_h2]:mt-3 [&_h2]:font-semibold [&_h2]:tracking-tight [&_h3]:mb-1 [&_h3]:mt-2.5 [&_h3]:font-semibold " +
  "[&_a]:text-ink [&_a]:underline [&_a]:decoration-ink-faint [&_a]:underline-offset-2 hover:[&_a]:decoration-ink [&_strong]:font-semibold [&_strong]:text-ink " +
  "[&_pre]:my-2.5 [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:border [&_pre]:border-border-subtle [&_pre]:bg-bg-sunken [&_pre]:p-3 [&_pre]:font-mono [&_pre]:text-xs [&_pre]:leading-relaxed [&_pre>code]:bg-transparent [&_pre>code]:p-0 [&_pre>code]:border-0 " +
  "[&_:not(pre)>code]:rounded-[5px] [&_:not(pre)>code]:border [&_:not(pre)>code]:border-border-subtle [&_:not(pre)>code]:bg-chip [&_:not(pre)>code]:px-[0.32em] [&_:not(pre)>code]:py-[0.12em] [&_:not(pre)>code]:font-mono [&_:not(pre)>code]:text-[0.85em] [&_:not(pre)>code]:text-ink " +
  "[&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_blockquote]:text-ink-muted " +
  "[&_table]:my-2 [&_table]:w-full [&_table]:border-collapse [&_th]:border [&_th]:border-border-subtle [&_th]:px-2.5 [&_th]:py-1 [&_th]:text-left [&_th]:font-medium [&_th]:text-ink-secondary [&_td]:border [&_td]:border-border-subtle [&_td]:px-2.5 [&_td]:py-1";

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

/** A fenced code block with a hover-reveal copy button. Reads the rendered text
 *  off the DOM so it captures exactly what the user sees. */
function CodeBlock({ children }: { children?: ReactNode }) {
  const ref = useRef<HTMLPreElement>(null);
  const [copied, copy] = useCopy();
  return (
    <div className="group/code relative">
      <pre ref={ref}>{children}</pre>
      <button
        type="button"
        onClick={() => copy(ref.current?.textContent ?? "")}
        aria-label={copied ? "Copied" : "Copy code"}
        title={copied ? "Copied" : "Copy code"}
        className="absolute right-2 top-2 grid size-7 place-items-center rounded-md bg-bg-elevated text-ink-faint opacity-0 ring-1 ring-border-subtle transition hover:text-ink group-hover/code:opacity-100"
      >
        {copied ? <Check className="size-3.5 text-success" /> : <Copy className="size-3.5" />}
      </button>
    </div>
  );
}

export function Md({ children }: { children: string }) {
  return (
    <Markdown
      remarkPlugins={[remarkGfm]}
      components={{
        a: ({ node: _node, ...props }) => <a {...props} target="_blank" rel="noreferrer noopener" />,
        pre: ({ node: _node, children }) => <CodeBlock>{children}</CodeBlock>,
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
    <div className="overflow-hidden rounded-md border border-border-subtle bg-bg-secondary/50">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex w-full items-center gap-1.5 px-2.5 py-1 text-xs text-ink-muted hover:bg-bg-hover/50"
      >
        <Brain className="size-3.5" />
        <span className="font-medium">Thinking</span>
        <ChevronRight className={cn("ml-auto size-3.5 transition-transform", open && "rotate-90")} />
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={reduce ? false : { height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={reduce ? { opacity: 0 } : { height: 0, opacity: 0 }}
            transition={{ duration: 0.18 }}
            className="overflow-hidden border-t border-border-subtle"
          >
            <div className="max-h-52 overflow-auto whitespace-pre-wrap px-2.5 py-2 text-xs leading-relaxed text-ink-muted">
              {text}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

function MessageImpl({
  role,
  blocks,
  streaming = false,
}: {
  role: Role;
  blocks: ContentBlock[];
  /** True for the assistant message currently being streamed — enables the
   *  cheaper prefix-memoized markdown path. */
  streaming?: boolean;
}) {
  const reduce = useReducedMotion();
  const body = text(blocks);

  const inner = (() => {
    if (role === "user") {
      // Codex form: a quiet right-aligned pill, not a loud accent bubble.
      return (
        <div className="flex justify-end">
          <div className="max-w-[80%] whitespace-pre-wrap [overflow-wrap:anywhere] rounded-2xl rounded-br-md border border-border-subtle bg-bg-tertiary px-3.5 py-2 text-sm text-ink">
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
    // thinking spans.
    const spans = parseNarration(body);
    return (
      <div className="min-w-0 space-y-2">
        {spans.map((span, i) => {
          if (span.kind === "thinking") return <ThinkingBlock key={i} text={span.text} />;
          return (
            <div
              key={i}
              className={cn(
                "text-[0.9375rem] leading-relaxed [overflow-wrap:anywhere]",
                MD_CLASSES,
                span.kind === "narrate" && "text-ink-secondary",
              )}
            >
              {streaming ? <StreamingMd text={span.text} /> : <Md>{span.text}</Md>}
            </div>
          );
        })}
      </div>
    );
  })();

  return (
    <motion.div
      initial={reduce ? false : { opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: [0.4, 0, 0.2, 1] }}
    >
      {role === "agent" && !streaming && body.trim() ? (
        <div className="group/msg relative">
          {inner}
          <CopyButton
            text={body}
            label="Copy message"
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
    if (blk.type === "text") return blk.text === (other as { text: string }).text;
    return JSON.stringify(blk) === JSON.stringify(other);
  });
}

export const Message = memo(
  MessageImpl,
  (a, b) => a.role === b.role && a.streaming === b.streaming && sameBlocks(a.blocks, b.blocks),
);
