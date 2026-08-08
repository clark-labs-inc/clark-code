import type { ComponentPropsWithoutRef, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: ReactNode }) => children,
  useReducedMotion: () => true,
}));

vi.mock("motion/react-m", () => ({
  div: ({
    initial,
    animate: _animate,
    transition: _transition,
    ...props
  }: ComponentPropsWithoutRef<"div"> & {
    initial?: unknown;
    animate?: unknown;
    transition?: unknown;
  }) => <div data-motion-initial={String(initial)} {...props} />,
}));

import { Message } from "./Message";

describe("message reduced motion", () => {
  it("keeps a short opacity fade while removing spatial movement", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: "A calm accessible response." }]}
        timelineIndex={2}
        streaming
        animateEntry
      />,
    );

    expect(markup).toContain('data-motion-initial="[object Object]"');
    expect(markup).toContain('data-chat-message-motion="fade"');
    expect(markup).toContain('data-sd-animate="true"');
    expect(markup).not.toContain("translate3d");
    expect(markup).toContain(">accessible</span>");
  });
});
