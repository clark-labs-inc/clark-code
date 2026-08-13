import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ArtifactFileActions } from "./ArtifactFileActions";

describe("ArtifactFileActions", () => {
  it("makes the complete local-file action set visible without a context menu", () => {
    const markup = renderToStaticMarkup(
      <ArtifactFileActions artifact={{
        id: "spectro",
        title: "spectro.png",
        kind: "image",
        mime_type: "image/png",
        uri: "/Users/test/.agent/workspace/task/spectro.png",
      }} />,
    );

    expect(markup).toContain("Open");
    expect(markup).toContain("Show in File Manager");
    expect(markup).toContain("Save a Copy");
    expect(markup).toContain("Copy Path");
  });

  it("offers a download for embedded generated images", () => {
    const markup = renderToStaticMarkup(
      <ArtifactFileActions artifact={{
        id: "generated",
        title: "generated.png",
        kind: "image",
        mime_type: "image/png",
        uri: "data:image/png;base64,iVBORw0KGgo=",
      }} />,
    );

    expect(markup).toContain("Save a Copy");
    expect(markup).not.toContain("Copy Path");
  });
});
