import { useCallback, useEffect, useState } from "react";

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

/** Light (warm papyrus, default) ↔ dark (clarkDark), persisted. A colorblind
 *  (daltonized) variant can be layered on top of either theme — it swaps the
 *  red/green success/danger tokens for blue/orange so status is legible to
 *  red-green colorblind users. */
export function useTheme() {
  const [dark, setDark] = useState<boolean>(() => {
    try {
      // `index.html` applies this class before CSS loads. Reading it first
      // makes React adopt the already-painted state instead of correcting it
      // after the first render.
      if (document.documentElement.classList.contains("dark")) return true;
      return localStorage.getItem("clark.theme") === "dark";
    } catch {
      return false;
    }
  });
  const [colorblind, setColorblind] = useState<boolean>(() => {
    try {
      if (document.documentElement.classList.contains("colorblind")) return true;
      return localStorage.getItem("clark.colorblind") === "1";
    } catch {
      return false;
    }
  });

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    document
      .querySelector('meta[name="theme-color"]')
      ?.setAttribute("content", dark ? "#0D0D0D" : "#F7F5F1");
    void syncNativeTheme(dark);
    try {
      localStorage.setItem("clark.theme", dark ? "dark" : "light");
    } catch {
      /* ignore */
    }
  }, [dark]);

  useEffect(() => {
    document.documentElement.classList.toggle("colorblind", colorblind);
    try {
      localStorage.setItem("clark.colorblind", colorblind ? "1" : "0");
    } catch {
      /* ignore */
    }
  }, [colorblind]);

  const toggle = useCallback(() => setDark((d) => !d), []);
  const toggleColorblind = useCallback(() => setColorblind((c) => !c), []);
  return { dark, toggle, colorblind, toggleColorblind };
}
