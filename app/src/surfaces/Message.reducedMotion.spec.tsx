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
  it("keeps short opacity cues while removing spatial and staggered word motion", () => {
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
    expect(markup).toContain("--sd-duration:120ms");
    expect(markup).not.toContain("translate3d");
    expect(markup).toContain(">A</span>");
    expect(markup).toContain(">response.</span>");
  });
});
