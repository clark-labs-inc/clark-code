import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { SpecialistConversationRow } from "./SpecialistNavigation";

const conversation = {
  id: "spec-1",
  title: "Customer segmentation",
  provider: "specialist" as const,
  createdAt: 1,
  updatedAt: 2,
  specialist: { kind: "spec" as const },
};

describe("SpecialistConversationRow", () => {
  it("marks the active spec as selected", () => {
    const markup = renderToStaticMarkup(
      <SpecialistConversationRow
        conversation={conversation}
        selected
        opening={false}
        running={false}
        deleting={false}
        confirmingDelete={false}
        onOpen={vi.fn()}
        onRequestDelete={vi.fn()}
        onConfirmDelete={vi.fn()}
        onCancelDelete={vi.fn()}
      />,
    );

    expect(markup).toContain('aria-current="page"');
    expect(markup).toContain("Customer segmentation, selected");
    expect(markup).toContain("bg-accent-subtle font-medium text-ink");
  });

  it("animates and announces real work on the active spec", () => {
    const markup = renderToStaticMarkup(
      <SpecialistConversationRow
        conversation={conversation}
        selected
        opening={false}
        running
        deleting={false}
        confirmingDelete={false}
        onOpen={vi.fn()}
        onRequestDelete={vi.fn()}
        onConfirmDelete={vi.fn()}
        onCancelDelete={vi.fn()}
      />,
    );

    expect(markup).toContain('aria-busy="true"');
    expect(markup).toContain("Customer segmentation, selected, Clark is working");
    expect(markup).toContain("animate-[spin_1s_linear_infinite]");
    expect(markup).toContain("breathe");
    expect(markup).not.toContain(">Working<");
  });

  it("offers an inline permanent-delete confirmation", () => {
    const hidden = renderToStaticMarkup(
      <SpecialistConversationRow
        conversation={conversation}
        selected={false}
        opening={false}
        running={false}
        deleting={false}
        confirmingDelete={false}
        onOpen={vi.fn()}
        onRequestDelete={vi.fn()}
        onConfirmDelete={vi.fn()}
        onCancelDelete={vi.fn()}
      />,
    );
    const confirming = renderToStaticMarkup(
      <SpecialistConversationRow
        conversation={conversation}
        selected={false}
        opening={false}
        running={false}
        deleting={false}
        confirmingDelete
        onOpen={vi.fn()}
        onRequestDelete={vi.fn()}
        onConfirmDelete={vi.fn()}
        onCancelDelete={vi.fn()}
      />,
    );

    expect(hidden).toContain('aria-label="Delete Customer segmentation"');
    expect(hidden).not.toContain("Permanently delete Customer segmentation");
    expect(confirming).toContain("Permanently delete Customer segmentation");
    expect(confirming).toContain('aria-label="Cancel delete"');
  });
});
