import { useCallback, useLayoutEffect, useState } from "react";

export const TEXT_SIZES = ["compact", "default", "large"] as const;
export type TextSize = (typeof TEXT_SIZES)[number];

export const TEXT_SIZE_LABELS: Record<TextSize, string> = {
  compact: "Compact",
  default: "Default",
  large: "Large",
};

export const TERMINAL_FONT_SIZES: Record<TextSize, number> = {
  compact: 12.5,
  default: 14,
  large: 16,
};

const STORAGE_KEY = "clark.text-size";

export function isTextSize(value: string | null): value is TextSize {
  return TEXT_SIZES.some((size) => size === value);
}

export function loadTextSize(storage?: Pick<Storage, "getItem">): TextSize {
  try {
    const source = storage ?? (typeof localStorage === "undefined" ? undefined : localStorage);
    const value = source?.getItem(STORAGE_KEY) ?? null;
    return isTextSize(value) ? value : "default";
  } catch {
    return "default";
  }
}

export function saveTextSize(size: TextSize, storage?: Pick<Storage, "setItem">): void {
  try {
    const target = storage ?? (typeof localStorage === "undefined" ? undefined : localStorage);
    target?.setItem(STORAGE_KEY, size);
  } catch {
    /* localStorage can be unavailable in a locked-down webview. */
  }
}

export function stepTextSize(size: TextSize, direction: -1 | 1): TextSize {
  const current = TEXT_SIZES.indexOf(size);
  const next = Math.min(TEXT_SIZES.length - 1, Math.max(0, current + direction));
  return TEXT_SIZES[next];
}

export function documentTextSize(): TextSize {
  if (typeof document === "undefined") return "default";
  const value = document.documentElement.dataset.textSize ?? null;
  return isTextSize(value) ? value : "default";
}

/** Persisted application text scale. The root data attribute drives semantic
 * CSS type tokens; consumers with their own renderer (xterm) read the same
 * attribute so every surface follows one preference. */
export function useTextSize() {
  const [textSize, setTextSizeState] = useState<TextSize>(loadTextSize);

  useLayoutEffect(() => {
    document.documentElement.dataset.textSize = textSize;
    saveTextSize(textSize);
  }, [textSize]);

  const setTextSize = useCallback((size: TextSize) => setTextSizeState(size), []);
  const increaseTextSize = useCallback(
    () => setTextSizeState((size) => stepTextSize(size, 1)),
    [],
  );
  const decreaseTextSize = useCallback(
    () => setTextSizeState((size) => stepTextSize(size, -1)),
    [],
  );
  const resetTextSize = useCallback(() => setTextSizeState("default"), []);

  return { textSize, setTextSize, increaseTextSize, decreaseTextSize, resetTextSize };
}
