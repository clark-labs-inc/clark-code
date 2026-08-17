import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";
import { emptySnapshot } from "../core-bridge/types";
import { useSessionStore } from "../store/sessionStore";
import { beginMessageEdit, Message } from "./Message";

beforeEach(() => {
  useSessionStore.setState({
    snapshot: emptySnapshot(),
    composerPrefill: null,
    notice: null,
  });
});

describe("user message actions", () => {
  it("does not enter edit mode while the current run is active", () => {
    useSessionStore.setState({
      snapshot: {
        ...emptySnapshot(),
        runs: { active: { id: "active", status: "running" } },
      },
    });

    expect(beginMessageEdit("co", 0)).toBe(false);
    expect(useSessionStore.getState().composerPrefill).toBeNull();
    expect(useSessionStore.getState().notice).toContain("Stop Clark before editing");
  });

  it("stages an edit after the run has stopped", () => {
    expect(beginMessageEdit("co", 0)).toBe(true);
    expect(useSessionStore.getState().composerPrefill).toEqual({
      text: "co",
      timelineIndex: 0,
    });
  });
});

describe("assistant message actions", () => {
  it("overlays Copy as Markdown without adding a footer row", () => {
    const body = "A response with a normal footer action.";
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: body }]}
        timelineIndex={0}
      />,
    );

    expect(markup).toContain("group/msg relative");
    expect(markup).toContain("absolute -top-1 right-0");
    expect(markup).not.toContain("mt-1 flex justify-end");
  });

  it("renders assistant output through the streaming-aware markdown layer", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: "A **streaming** response." }]}
        timelineIndex={0}
        streaming
      />,
    );

    expect(markup).toContain('data-sd-animate="true"');
    expect(markup).toContain('data-streaming-reply="true"');
    expect(markup.match(/reply-stream-bar/g)).toHaveLength(3);
    expect(markup).toContain("<strong><span");
    expect(markup).toContain(">streaming</span></strong>");
    expect(markup).not.toContain("--streamdown-caret");
  });

  it("trades reserved skeleton rows for streamed Markdown lines", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: "First line.\nSecond line.\nThird line." }]}
        timelineIndex={0}
        streaming
      />,
    );

    expect(markup).toContain('data-streaming-reply="true"');
    expect(markup.match(/reply-stream-bar/g)).toHaveLength(1);
    expect(markup).toContain(">First</span>");
    expect(markup).toContain(">Second</span>");
    expect(markup).toContain(">Third</span>");
  });

  it("keeps every assistant markdown layer on the full message rail", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: "First paragraph.\n\nSecond paragraph that must not collapse." }]}
        timelineIndex={0}
        streaming
      />,
    );

    expect(markup).toContain("min-w-0 w-full space-y-1.5");
    expect(markup).toContain("min-w-0 w-full text-base");
    expect(markup).toMatch(/space-y-4 whitespace-normal[^\"]*min-w-0 w-full/);
  });

  it("uses the softer semantic reading color with stronger emphasis", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: "A calmer **response**." }]}
        timelineIndex={0}
      />,
    );

    expect(markup).toContain("text-ink-secondary");
    expect(markup).toContain("text-ink");
    expect(markup).toContain("leading-[1.72]");
  });

  it("omits native reasoning blocks from the UI and copied Markdown", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[
          { type: "thinking", text: "Private chain of thought." },
          { type: "text", text: "The visible answer." },
        ]}
        timelineIndex={0}
      />,
    );

    expect(markup).toContain("The visible answer.");
    expect(markup).not.toContain("Thinking");
    expect(markup).not.toContain("Private chain of thought.");
  });

  it("omits legacy inline thinking spans without hiding adjacent answers", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{
          type: "text",
          text: "Before. <thinking>Private deliberation.</thinking> After.",
        }]}
        timelineIndex={0}
      />,
    );

    expect(markup).toContain("Before.");
    expect(markup).toContain("After.");
    expect(markup).not.toContain("Thinking");
    expect(markup).not.toContain("Private deliberation.");
  });

  it("renders no row for a reasoning-only assistant event", () => {
    const markup = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "thinking", text: "Private chain of thought." }]}
        timelineIndex={0}
      />,
    );

    expect(markup).toBe("");
  });

  it("marks new user and assistant rows with role-specific entrance motion", () => {
    const user = renderToStaticMarkup(
      <Message
        role="user"
        blocks={[{ type: "text", text: "Please animate this row." }]}
        timelineIndex={1}
        animateEntry
      />,
    );
    const assistant = renderToStaticMarkup(
      <Message
        role="agent"
        blocks={[{ type: "text", text: "This answer arrives progressively." }]}
        timelineIndex={2}
        animateEntry
      />,
    );

    expect(user).toContain('data-chat-message-role="user"');
    expect(user).toContain('data-chat-message-motion="enter"');
    expect(assistant).toContain('data-chat-message-role="assistant"');
    expect(assistant).toContain('data-chat-message-motion="enter"');
    expect(assistant).toContain('data-sd-animate="true"');
  });
});
