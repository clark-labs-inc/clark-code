import type { ReactNode } from "react";
import { cn } from "../lib/cn";

export const STREAMING_REPLY_RESERVE_LINES = 4;
const APPROXIMATE_CHARACTERS_PER_LINE = 72;
const SKELETON_WIDTHS = ["w-[92%]", "w-[84%]", "w-[68%]", "w-[76%]"] as const;

/** Estimate only enough visible rows to trade one placeholder for one line of
 * content. The fixed minimum height below absorbs normal wrapping variance, so
 * this deliberately stays cheap on the per-token render path. */
export function streamingReplyPlaceholderCount(markdown: string): number {
  if (!markdown.trim()) return STREAMING_REPLY_RESERVE_LINES;

  const visibleLines = markdown.trimEnd().split("\n").reduce((count, line) => {
    const readable = line
      .replace(/^\s{0,3}(?:#{1,6}|[-*+] |\d+[.)] |>|\|)\s*/, "")
      .replace(/[*_`~]/g, "")
      .trim();
    return count + Math.max(1, Math.ceil(readable.length / APPROXIMATE_CHARACTERS_PER_LINE));
  }, 0);

  return Math.max(0, STREAMING_REPLY_RESERVE_LINES - visibleLines);
}

export function ReplySkeleton({
  lines = STREAMING_REPLY_RESERVE_LINES,
  startIndex = 0,
  className,
}: {
  lines?: number;
  startIndex?: number;
  className?: string;
}) {
  return (
    <div className={cn("reply-skeleton", className)} aria-hidden>
      {Array.from({ length: lines }, (_, index) => (
        <div className="reply-stream-line" key={index}>
          <div
            className={cn(
              "skeleton reply-stream-bar",
              SKELETON_WIDTHS[(startIndex + index) % SKELETON_WIDTHS.length],
            )}
          />
        </div>
      ))}
    </div>
  );
}

/** Keeps the live answer's geometry stable while Streamdown fills it. Each
 * estimated text line consumes one skeleton row; the minimum height relaxes
 * only after streaming ends, using the shared motion policy. */
export function StreamingReplyFrame({
  children,
  text,
  streaming,
}: {
  children: ReactNode;
  text: string;
  streaming: boolean;
}) {
  const placeholders = streaming ? streamingReplyPlaceholderCount(text) : 0;
  const revealed = STREAMING_REPLY_RESERVE_LINES - placeholders;

  return (
    <div
      className="streaming-reply-frame"
      data-streaming-reply={streaming ? "true" : "false"}
    >
      {children}
      {placeholders > 0 && <ReplySkeleton lines={placeholders} startIndex={revealed} />}
    </div>
  );
}
