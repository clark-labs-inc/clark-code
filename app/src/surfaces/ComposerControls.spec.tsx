import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

import { QuickChatModelLabel } from "./ComposerControls";

describe("Quick Chat model control", () => {
  it("renders Free as a static label without tier-selection affordances", () => {
    const html = renderToStaticMarkup(<QuickChatModelLabel />);

    expect(html).toContain(">Free</span>");
    expect(html).toContain("Quick Chat uses the Free tier");
    expect(html).not.toContain("<button");
    expect(html).not.toContain("aria-haspopup");
  });
});
