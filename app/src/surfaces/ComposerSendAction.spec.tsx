import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { ComposerSendAction } from "./ComposerSendAction";

const baseProps = {
  submitting: false,
  busy: false,
  editing: false,
  hasContent: true,
  canSend: true,
  shouldPickProjectFolder: false,
  startsScoutRun: false,
  queuedTitle: "Queue message",
  onCancel: () => {},
  onSubmit: () => {},
};

describe("ComposerSendAction", () => {
  it("shows a non-repeatable sending state while admission is pending", () => {
    const html = renderToStaticMarkup(
      <ComposerSendAction {...baseProps} submitting busy />,
    );

    expect(html).toContain('aria-label="Sending message"');
    expect(html).toContain('aria-busy="true"');
    expect(html).toContain("disabled");
    expect(html).not.toContain('aria-label="Queue message"');
    expect(html).not.toContain('aria-label="Stop"');
  });

  it("offers Stop only after admission completes and the next draft is empty", () => {
    const html = renderToStaticMarkup(
      <ComposerSendAction {...baseProps} busy hasContent={false} />,
    );

    expect(html).toContain('aria-label="Stop"');
    expect(html).not.toContain('aria-busy="true"');
  });

  it("offers Queue message for a genuine follow-up typed during an active run", () => {
    const html = renderToStaticMarkup(
      <ComposerSendAction {...baseProps} busy />,
    );

    expect(html).toContain('aria-label="Queue message"');
  });

  it("offers Stop instead of Queue when an edit races with an active run", () => {
    const html = renderToStaticMarkup(
      <ComposerSendAction {...baseProps} busy editing />,
    );

    expect(html).toContain('aria-label="Stop Clark to continue editing"');
    expect(html).not.toContain('aria-label="Queue message"');
  });
});
