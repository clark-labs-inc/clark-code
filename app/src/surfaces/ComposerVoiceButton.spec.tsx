import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import { installProductModule, neutralProduct } from "../product/productModule";
import { ComposerVoiceButton } from "./ComposerVoiceButton";

describe("composer voice dictation", () => {
  afterEach(() => installProductModule(neutralProduct));

  it("exposes dictation when the product supplies the Clark realtime stream", () => {
    installProductModule({
      ...neutralProduct,
      voice: {
        stream: {
          start: vi.fn(async () => ({ id: "voice-1" })),
          send: vi.fn(async () => ({ text: "An editable dictated" })),
          finish: vi.fn(async () => ({ text: "An editable dictated idea." })),
          cancel: vi.fn(async () => undefined),
        },
      },
    });

    const markup = renderToStaticMarkup(
      <ComposerVoiceButton onTranscript={vi.fn()} onError={vi.fn()} />,
    );

    expect(markup).toContain('aria-label="Start voice dictation"');
    expect(markup).toContain("Narrate your idea");
  });

  it("keeps the neutral foundation free of an unavailable voice control", () => {
    const markup = renderToStaticMarkup(
      <ComposerVoiceButton onTranscript={vi.fn()} onError={vi.fn()} />,
    );
    expect(markup).toBe("");
  });
});
