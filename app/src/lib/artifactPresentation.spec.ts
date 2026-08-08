import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Artifact } from "../core-bridge/types";
import {
  artifactAvailability,
  artifactLocationLabel,
  canOpenArtifactExternally,
  readableArtifactLocation,
} from "./artifactPresentation";
import { installProductModule, neutralProduct } from "../product/productModule";

beforeEach(() => installProductModule({
  ...neutralProduct,
  artifacts: {
    ...neutralProduct.artifacts,
    isCloudUri: (uri) => /^\/product-artifacts\/[^/]+$/.test(uri),
  },
}));
afterEach(() => installProductModule(neutralProduct));

function artifact(uri?: string): Artifact {
  return {
    id: "artifact-1",
    title: "report.md",
    kind: "file",
    mime_type: "text/markdown",
    uri,
  };
}

describe("artifact presentation metadata", () => {
  it("keeps unavailable, saved, and available states distinct", () => {
    expect(artifactAvailability(artifact())).toBe("unavailable");
    expect(artifactAvailability(artifact("/tmp/report.md"))).toBe("saved");
    expect(artifactAvailability(artifact("https://example.com/report.md"))).toBe("available");
  });

  it("describes embedded data without exposing its payload", () => {
    const embedded = artifact("data:text/markdown,%23%20Private%20report");
    expect(artifactLocationLabel(embedded)).toBe("Embedded");
    expect(readableArtifactLocation(embedded)).toBe("Embedded in this task");
    expect(canOpenArtifactExternally(embedded)).toBe(false);
  });

  it("keeps external locations concise and openable", () => {
    const remote = artifact("https://example.com/reports/report.md?token=secret");
    expect(artifactLocationLabel(remote)).toBe("Remote");
    expect(readableArtifactLocation(remote)).toBe("example.com/reports/report.md");
    expect(canOpenArtifactExternally(remote)).toBe(true);
  });

  it("labels product cloud and pending workspace artifacts without unsafe external links", () => {
    const cloud = artifact("/product-artifacts/artifact-1");
    expect(artifactLocationLabel(cloud)).toBe("Product cloud");
    expect(readableArtifactLocation(cloud)).toBe("Saved securely in product cloud");
    expect(canOpenArtifactExternally(cloud)).toBe(false);

    const pending = artifact("workspace-artifact://desk-1/report.md");
    expect(artifactAvailability(pending)).toBe("saved");
    expect(artifactLocationLabel(pending)).toBe("Local");
    expect(canOpenArtifactExternally(pending)).toBe(false);
  });
});
