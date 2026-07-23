import { useCallback, useLayoutEffect, useState } from "react";

/** Familiar browser-style stops: precise around the default, faster at the
 * extremes so repeated shortcuts do not become tedious. */
export const TEXT_SIZES = [75, 80, 90, 100, 110, 125, 150, 175, 200] as const;
export type TextSize = (typeof TEXT_SIZES)[number];

const LEGACY_TEXT_SIZES: Record<string, TextSize> = {
  compact: 90,
  default: 100,
  large: 110,
};

const STORAGE_KEY = "clark.text-size";
const DEFAULT_TEXT_SIZE: TextSize = 100;

export function isTextSize(value: number): value is TextSize {
  return TEXT_SIZES.some((size) => size === value);
}

export function parseTextSize(value: string | null): TextSize | null {
  if (value === null) return null;
  const legacy = LEGACY_TEXT_SIZES[value];
  if (legacy !== undefined) return legacy;
  const numeric = Number(value);
  return isTextSize(numeric) ? numeric : null;
}

export function loadTextSize(storage?: Pick<Storage, "getItem">): TextSize {
  try {
    const source = storage ?? (typeof localStorage === "undefined" ? undefined : localStorage);
    const value = source?.getItem(STORAGE_KEY) ?? null;
    return parseTextSize(value) ?? DEFAULT_TEXT_SIZE;
  } catch {
    return DEFAULT_TEXT_SIZE;
  }
}

export function saveTextSize(size: TextSize, storage?: Pick<Storage, "setItem">): void {
  try {
    const target = storage ?? (typeof localStorage === "undefined" ? undefined : localStorage);
    target?.setItem(STORAGE_KEY, String(size));
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
  if (typeof document === "undefined") return DEFAULT_TEXT_SIZE;
  const value = document.documentElement.dataset.textSize ?? null;
  return parseTextSize(value) ?? DEFAULT_TEXT_SIZE;
}

export function terminalFontSize(size: TextSize): number {
  return (14 * size) / 100;
}

/** Persisted application text scale. The root data attribute drives semantic
 * CSS type tokens; consumers with their own renderer (xterm) read the same
 * attribute so every surface follows one preference. */
export function useTextSize() {
  const [textSize, setTextSizeState] = useState<TextSize>(loadTextSize);

  useLayoutEffect(() => {
    document.documentElement.dataset.textSize = String(textSize);
    document.documentElement.style.setProperty("--text-size-scale", String(textSize / 100));
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
  const resetTextSize = useCallback(() => setTextSizeState(DEFAULT_TEXT_SIZE), []);

  return { textSize, setTextSize, increaseTextSize, decreaseTextSize, resetTextSize };
}
