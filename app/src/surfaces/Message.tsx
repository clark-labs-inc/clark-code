import { useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Brain, ChevronRight } from "lucide-react";
import { cn } from "../lib/cn";
import { parseNarration } from "../lib/narration";
import type { ContentBlock, Role } from "../core-bridge/types";

function text(blocks: ContentBlock[]): string {
  return blocks.map((b) => (b.type === "text" ? b.text : `\`[${b.type}]\``)).join("");
}

const MD_CLASSES =
  "text-ink [&_p]:my-2 [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 " +
  "[&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5 [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_li]:my-0.5 " +
  "[&_h1]:mb-1.5 [&_h1]:mt-3 [&_h1]:text-lg [&_h1]:font-semibold [&_h2]:mb-1.5 [&_h2]:mt-3 [&_h2]:font-semibold [&_h3]:mb-1 [&_h3]:mt-2.5 [&_h3]:font-semibold " +
  "[&_a]:text-info [&_a]:underline [&_a]:underline-offset-2 [&_strong]:font-semibold " +
  "[&_pre]:my-2 [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:border [&_pre]:border-border-subtle [&_pre]:bg-bg-sunken [&_pre]:p-3 [&_pre]:font-mono [&_pre]:text-xs " +
  "[&_:not(pre)>code]:rounded [&_:not(pre)>code]:bg-bg-tertiary [&_:not(pre)>code]:px-1 [&_:not(pre)>code]:py-0.5 [&_:not(pre)>code]:font-mono [&_:not(pre)>code]:text-[0.85em] " +
  "[&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_blockquote]:text-ink-muted " +
  "[&_table]:my-2 [&_table]:w-full [&_th]:border [&_th]:border-border [&_th]:px-2 [&_th]:py-1 [&_th]:text-left [&_td]:border [&_td]:border-border [&_td]:px-2 [&_td]:py-1";

function Md({ children }: { children: string }) {
  return (
    <Markdown
      remarkPlugins={[remarkGfm]}
      components={{
        a: ({ node: _node, ...props }) => <a {...props} target="_blank" rel="noreferrer noopener" />,
      }}
    >
      {children}
    </Markdown>
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

export function Message({ role, blocks }: { role: Role; blocks: ContentBlock[] }) {
  const reduce = useReducedMotion();
  const body = text(blocks);

  const inner = (() => {
    if (role === "user") {
      return (
        <div className="flex justify-end">
          <div className="max-w-[85%] whitespace-pre-wrap rounded-xl rounded-br-sm bg-accent px-3.5 py-2 text-sm text-on-accent">
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
    // Assistant: split into answer / narration / thinking spans.
    const spans = parseNarration(body);
    return (
      <div className="flex gap-2.5">
        <div className="mt-0.5 grid size-5 shrink-0 place-items-center rounded-full bg-accent/10 text-[0.65rem] font-semibold text-accent">
          C
        </div>
        <div className="min-w-0 flex-1 space-y-2">
          {spans.map((span, i) => {
            if (span.kind === "thinking") return <ThinkingBlock key={i} text={span.text} />;
            return (
              <div
                key={i}
                className={cn(
                  "text-[0.9375rem] leading-relaxed",
                  MD_CLASSES,
                  span.kind === "narrate" && "text-ink-secondary",
                )}
              >
                <Md>{span.text}</Md>
              </div>
            );
          })}
        </div>
      </div>
    );
  })();

  return (
    <motion.div
      initial={reduce ? false : { opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: [0.4, 0, 0.2, 1] }}
    >
      {inner}
    </motion.div>
  );
}
