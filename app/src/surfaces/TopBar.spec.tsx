import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { UpdatePillView } from "./TopBar";

describe("UpdatePill", () => {
  it("renders an actionable ready state as soon as an update is staged", () => {
    const html = renderToStaticMarkup(
      <UpdatePillView
        update={{ version: "0.1.65" }}
        progress={null}
        waiting={false}
        reduce
        onApply={async () => {}}
      />,
    );

    expect(html).toContain("<button");
    expect(html).toContain("Ready to update");
    expect(html).toContain("Ready to update Clark Code to 0.1.65; restart now");
  });

  it("shows download progress before the ready action", () => {
    const html = renderToStaticMarkup(
      <UpdatePillView
        update={null}
        progress={{ downloaded: 25, total: 100 }}
        waiting={false}
        reduce
        onApply={async () => {}}
      />,
    );

    expect(html).toContain("Downloading update 25%");
    expect(html).not.toContain("Ready to update");
  });
});
