import { highlight, highlightLines } from "./highlightEngine";
import type { HighlightRequest, HighlightResponse } from "./highlight";

self.onmessage = async ({ data }: MessageEvent<HighlightRequest>) => {
  try {
    const result = data.mode === "html"
      ? await highlight(data.code, data.lang)
      : await highlightLines(data.code, data.lang);
    self.postMessage({ id: data.id, result } satisfies HighlightResponse);
  } catch {
    self.postMessage({ id: data.id, result: null } satisfies HighlightResponse);
  }
};
