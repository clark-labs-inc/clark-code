import { useCallback, useEffect, useState } from "react";

/** Light (warm papyrus, default) ↔ dark (clarkDark), persisted. A colorblind
 *  (daltonized) variant can be layered on top of either theme — it swaps the
 *  red/green success/danger tokens for blue/orange so status is legible to
 *  red-green colorblind users. */
export function useTheme() {
  const [dark, setDark] = useState<boolean>(() => {
    try {
      return localStorage.getItem("clark.theme") === "dark";
    } catch {
      return false;
    }
  });
  const [colorblind, setColorblind] = useState<boolean>(() => {
    try {
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
