import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ArtifactJsonPreview, isJsonArtifact } from "./ArtifactJsonPreview";

describe("JSON artifact preview", () => {
  it("recognizes research tree files and renders their content as escaped, readable text", () => {
    expect(isJsonArtifact({ id: "tree", title: "site-research.json", kind: "file" })).toBe(true);
    const html = renderToStaticMarkup(
      <ArtifactJsonPreview text={'{"title":"<script>alert(1)</script>","passes":[1,2]}'} loading={false} onFallback={() => null} />,
    );
    expect(html).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
    expect(html).toContain("&quot;passes&quot;: [");
    expect(html).not.toContain("<script>");
  });
});
