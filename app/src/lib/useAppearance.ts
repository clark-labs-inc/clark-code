import { useCallback, useEffect, useLayoutEffect, useState } from "react";

export const INTERFACE_CONTRASTS = ["low", "medium", "high", "extra-high"] as const;
export type InterfaceContrast = (typeof INTERFACE_CONTRASTS)[number];
export const DEFAULT_INTERFACE_CONTRAST: InterfaceContrast = "medium";

const CONTRAST_STORAGE_KEY = "agent-desktop.interface-contrast";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function syncNativeTheme(dark: boolean): Promise<void> {
  if (!isTauri()) return;
  try {
    const { setTheme } = await import("@tauri-apps/api/app");
    await setTheme(dark ? "dark" : "light");
  } catch {
    // Appearance is still correct in the webview if an older host lacks the
    // capability; do not let native chrome synchronization break theme use.
  }
}

export function parseInterfaceContrast(value: string | null): InterfaceContrast | null {
  return INTERFACE_CONTRASTS.find((contrast) => contrast === value) ?? null;
}

export function loadInterfaceContrast(
  storage?: Pick<Storage, "getItem">,
): InterfaceContrast {
  try {
    const source = storage ?? (typeof localStorage === "undefined" ? undefined : localStorage);
    return parseInterfaceContrast(source?.getItem(CONTRAST_STORAGE_KEY) ?? null)
      ?? DEFAULT_INTERFACE_CONTRAST;
  } catch {
    return DEFAULT_INTERFACE_CONTRAST;
  }
}

/** Owns the persisted visual appearance of the entire application. The HTML
 * bootstrap applies the same preferences before CSS loads; this hook adopts
 * that first paint and remains the sole runtime authority afterward. */
export function useAppearance() {
  const [dark, setDark] = useState<boolean>(() => {
    try {
      if (document.documentElement.classList.contains("dark")) return true;
      return localStorage.getItem("agent-desktop.theme") === "dark";
    } catch {
      return false;
    }
  });
  const [colorblind, setColorblind] = useState<boolean>(() => {
    try {
      if (document.documentElement.classList.contains("colorblind")) return true;
      return localStorage.getItem("agent-desktop.colorblind") === "1";
    } catch {
      return false;
    }
  });
  const [interfaceContrast, setInterfaceContrast] = useState<InterfaceContrast>(() => {
    const painted = typeof document === "undefined"
      ? null
      : parseInterfaceContrast(document.documentElement.dataset.interfaceContrast ?? null);
    return painted ?? loadInterfaceContrast();
  });

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    document
      .querySelector('meta[name="theme-color"]')
      ?.setAttribute("content", dark ? "#0D0D0D" : "#F7F5F1");
    void syncNativeTheme(dark);
    try {
      localStorage.setItem("agent-desktop.theme", dark ? "dark" : "light");
    } catch {
      /* ignore */
    }
  }, [dark]);

  useEffect(() => {
    document.documentElement.classList.toggle("colorblind", colorblind);
    try {
      localStorage.setItem("agent-desktop.colorblind", colorblind ? "1" : "0");
    } catch {
      /* ignore */
    }
  }, [colorblind]);

  useLayoutEffect(() => {
    document.documentElement.dataset.interfaceContrast = interfaceContrast;
    try {
      localStorage.setItem(CONTRAST_STORAGE_KEY, interfaceContrast);
    } catch {
      /* ignore */
    }
  }, [interfaceContrast]);

  const toggleTheme = useCallback(() => setDark((value) => !value), []);
  const toggleColorblind = useCallback(() => setColorblind((value) => !value), []);
  return {
    dark,
    toggleTheme,
    colorblind,
    toggleColorblind,
    interfaceContrast,
    setInterfaceContrast,
  };
}
