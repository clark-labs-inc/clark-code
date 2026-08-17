import type { ComponentPropsWithoutRef, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { ToolCall } from "../../core-bridge/types";

vi.mock("motion/react", () => {
  return {
    AnimatePresence: ({ children }: { children: ReactNode }) => children,
    useReducedMotion: () => true,
  };
});

vi.mock("motion/react-m", () => {
  function MotionSection({
    initial,
    animate: _animate,
    exit: _exit,
    transition: _transition,
    ...props
  }: ComponentPropsWithoutRef<"section"> & {
    initial?: unknown;
    animate?: unknown;
    exit?: unknown;
    transition?: unknown;
  }) {
    return <section data-motion-initial={String(initial)} {...props} />;
  }

  function MotionDiv({
    initial,
    animate: _animate,
    exit: _exit,
    transition: _transition,
    ...props
  }: ComponentPropsWithoutRef<"div"> & {
    initial?: unknown;
    animate?: unknown;
    exit?: unknown;
    transition?: unknown;
  }) {
    return <div data-motion-initial={String(initial)} {...props} />;
  }

  return {
    section: MotionSection,
    div: MotionDiv,
  };
});

import { ResearchWork } from "./ResearchWork";

describe("ResearchWork reduced motion", () => {
  it("mounts the live card with low-motion progress feedback", () => {
    const call: ToolCall = {
      id: "research-reduced-motion",
      title: "brokered_research: Verify official sources",
      kind: "research",
      status: "in_progress",
      raw_input: { query: "Verify official sources" },
      locations: [],
      content: [],
      progress: {
        revision: 2,
        status: "in_progress",
        latest_activity: "Reading official documentation",
        phases: [
          {
            id: "verify",
            title: "Verify sources",
            status: "in_progress",
            steps: [],
          },
        ],
        agents: [],
      },
    };

    const markup = renderToStaticMarkup(<ResearchWork call={call} active />);

    expect(markup).toContain('data-motion-initial="[object Object]"');
    expect(markup).toContain("animate-[spin_1s_linear_infinite]");
    expect(markup).toContain('data-clark-work-receipt="true"');
    expect(markup).not.toContain("Verify sources");
    expect(markup).not.toContain("translateY");
  });
});
