import { useCallback, useEffect, useState } from "react";

/** Light (warm papyrus, default) ↔ dark (clarkDark), persisted. */
export function useTheme() {
  const [dark, setDark] = useState<boolean>(() => {
    try {
      // Default to the dark Clark brand; light (warm papyrus) is opt-in.
      return localStorage.getItem("clark.theme") !== "light";
    } catch {
      return true;
    }
  });

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    try {
      localStorage.setItem("clark.theme", dark ? "dark" : "light");
    } catch {
      /* ignore */
    }
  }, [dark]);

  const toggle = useCallback(() => setDark((d) => !d), []);
  return { dark, toggle };
}
