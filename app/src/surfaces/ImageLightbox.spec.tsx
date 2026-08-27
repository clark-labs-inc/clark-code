import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Lightbox, ZoomableImage, clampZoom } from "./ImageLightbox";

const PNG = "data:image/png;base64,iVBORw0KGgo=";

describe("clampZoom", () => {
  it("keeps zoom within 1x–5x", () => {
    expect(clampZoom(0.4)).toBe(1);
    expect(clampZoom(9)).toBe(5);
    expect(clampZoom(2)).toBe(2);
  });

  it("rounds to two decimals so the label stays stable", () => {
    expect(clampZoom(1.234567)).toBe(1.23);
  });
});

describe("Lightbox", () => {
  const onClose = () => {};

  it("renders a modal viewer with fit-scale controls and the image", () => {
    const markup = renderToStaticMarkup(
      <Lightbox src={PNG} alt="spectro.png" onClose={onClose} />,
    );

    expect(markup).toContain('role="dialog"');
    expect(markup).toContain('aria-modal="true"');
    expect(markup).toContain(`src="${PNG}"`);
    expect(markup).toContain("Image viewer: spectro.png");
    // Starts at fit (100%) with both directions reachable.
    expect(markup).toContain(">100%</span>");
    expect(markup).toContain('aria-label="Zoom in"');
    expect(markup).toContain('aria-label="Zoom out"');
    expect(markup).toContain('aria-label="Fit to screen"');
    expect(markup).toContain('aria-label="Close image viewer"');
  });

  it("opens at fit scale with the zoom-in affordance on the image", () => {
    const markup = renderToStaticMarkup(
      <Lightbox src={PNG} alt="shot.png" onClose={onClose} />,
    );

    expect(markup).toContain("cursor-zoom-in");
    expect(markup).toContain("scale(1)");
  });
});

describe("ZoomableImage", () => {
  it("renders a click target around the thumbnail and no open viewer by default", () => {
    const markup = renderToStaticMarkup(
      <ZoomableImage src={PNG} alt="photo.png" className="size-8 rounded-lg object-cover" />,
    );

    expect(markup).toContain('aria-label="View photo.png full size"');
    expect(markup).toContain(`src="${PNG}"`);
    expect(markup).toContain("cursor-zoom-in");
    expect(markup).not.toContain('role="dialog"');
  });
});
