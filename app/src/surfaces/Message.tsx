import { memo } from "react";
import { useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { Copy, Check, FileText, Pencil, Sparkles } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import {
  CHAT_REDUCED_ROW_MOTION,
  CHAT_REDUCED_TEXT_ANIMATION,
  CHAT_TEXT_ANIMATION,
  chatRowMotion,
} from "../lib/motion";
import { cn } from "../lib/cn";
import { useCopy } from "../lib/clipboard";
import { currentActivity } from "../lib/activity";
import { userAttachmentBlocks, userTextBody } from "../lib/messageBlocks";
import { parseNarration, presentationKind } from "../lib/narration";
import { MarkdownContent, MARKDOWN_CLASSES } from "./MarkdownContent";
import { StreamingReplyFrame } from "./StreamingReply";
import type { ContentBlock, MessagePhase, Role } from "../core-bridge/types";

function text(blocks: ContentBlock[]): string {
  return blocks
    .map((b) => {
      if (b.type === "text") return b.text;
      // Reasoning remains available in the durable trajectory for provider
      // continuity, but it is deliberately absent from the user-facing UI.
      if (b.type === "thinking") return "";
      return `\`[${b.type}]\``;
    })
    .join("");
}

/** Editing replaces the selected turn and every later turn, so it cannot be
 * staged as a queued follow-up while that suffix is still being produced. */
export function beginMessageEdit(body: string, timelineIndex: number): boolean {
  const state = useSessionStore.getState();
  if (currentActivity(state.snapshot).busy) {
    state.flashNotice("Stop Clark before editing an earlier message.");
    return false;
  }
  state.setComposerPrefill(body, timelineIndex);
  return true;
}

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
        if (block.type === "audio") {
          return (
            <audio key={i} controls src={`data:${block.mime_type};base64,${block.data}`} />
          );
        }
        if (block.type === "resource") {
          return (
            <span
              key={i}
              className="flex max-w-full items-center gap-1.5 rounded-lg bg-bg-sunken px-2 py-1 text-xs text-ink-secondary"
            >
              <FileText className="size-3.5 shrink-0 text-ink-muted" />
              <span className="max-w-48 truncate">{block.uri.replace("attachment://", "")}</span>
            </span>
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
  animateEntry = false,
}: {
  role: Role;
  blocks: ContentBlock[];
  phase?: MessagePhase;
  timelineIndex: number;
  /** True for the assistant message currently being streamed — enables the
   *  cheaper prefix-memoized markdown path. */
  streaming?: boolean;
  /** True only when this row was appended during the visible conversation.
   * Replayed history stays settled when a conversation opens or switches. */
  animateEntry?: boolean;
}) {
  const reduce = useReducedMotion();
  // User turns carry attachment echo blocks (image / resource_link) alongside
  // the text; keep those out of the flattened copy/edit body.
  const source = role === "user" ? "" : text(blocks);
  const assistantSpans = role === "agent"
    ? parseNarration(source)
      .map((span) => ({ ...span, kind: presentationKind(span.kind, phase) }))
      .filter((span) => span.kind !== "thinking")
    : [];
  const body = role === "user"
    ? userTextBody(blocks)
    : role === "agent"
      ? assistantSpans.map((span) => span.text).join("\n\n")
      : source;

  // A reasoning-only assistant event is retained in state but has no visual
  // row, spacing, copy action, or accessibility surface.
  if (role === "agent" && assistantSpans.length === 0) return null;

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
              onClick={() => beginMessageEdit(body, timelineIndex)}
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
    // Assistant: full-width text (no avatar), split into answer and narration
    // spans. Private reasoning was filtered before this render boundary.
    // Streamdown repairs and memoizes incomplete Markdown while animating only
    // the newly arrived words in each live span.
    const spans = assistantSpans;
    return (
      <div className="min-w-0 w-full space-y-1.5">
        {spans.map((span, i) => {
          const kind = span.kind;
          const live = streaming && i === spans.length - 1;
          return (
            <div
              key={i}
              className={cn(
                "min-w-0 w-full text-base leading-[1.72] [overflow-wrap:anywhere]",
                MARKDOWN_CLASSES,
                kind === "narrate" && "text-ink-secondary",
              )}
            >
              <StreamingReplyFrame text={span.text} streaming={live}>
                <MarkdownContent
                  mode={live ? "streaming" : "static"}
                  className="min-w-0 w-full"
                  animated={reduce ? CHAT_REDUCED_TEXT_ANIMATION : CHAT_TEXT_ANIMATION}
                  isAnimating={live || animateEntry}
                >
                  {span.text}
                </MarkdownContent>
              </StreamingReplyFrame>
            </div>
          );
        })}
      </div>
    );
  })();

  // The wrapper is stable across the whole message lifecycle, so the markdown
  // subtree is not remounted when streaming settles. Avoid content-visibility
  // here: transcript rows have highly variable heights, and substituting an
  // intrinsic estimate for an unseen row makes upward scrolling jump when its
  // real height is first measured.
  const rowMotion = reduce ? CHAT_REDUCED_ROW_MOTION : chatRowMotion(role);
  return (
    <m.div
      initial={animateEntry ? rowMotion.initial : false}
      animate={rowMotion.animate}
      transition={animateEntry ? rowMotion.transition : { duration: 0 }}
      data-chat-message-role={role === "agent" ? "assistant" : role}
      data-chat-message-motion={
        animateEntry ? (reduce ? "fade" : "enter") : "settled"
      }
    >
      {role === "agent" ? (
        <div className="group/msg relative">
          {inner}
          <CopyButton
            text={body}
            label="Copy as Markdown"
            className="absolute -top-1 right-0 size-7 bg-bg-elevated opacity-0 ring-1 ring-border-subtle transition-opacity group-hover/msg:opacity-100 focus-visible:opacity-100"
          />
        </div>
      ) : (
        inner
      )}
    </m.div>
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
    a.animateEntry === b.animateEntry &&
    sameBlocks(a.blocks, b.blocks),
);
