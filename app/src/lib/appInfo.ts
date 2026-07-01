// The running app version, for display (e.g. the start-screen footer). Reads
// tauri.conf.json's version via the Tauri API; empty in the browser preview.

import { useEffect, useState } from "react";

let cached: string | null = null;

export function useAppVersion(): string {
  const [version, setVersion] = useState(cached ?? "");
  useEffect(() => {
    if (cached !== null) return;
    let alive = true;
    void (async () => {
      try {
        if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
          const { getVersion } = await import("@tauri-apps/api/app");
          cached = await getVersion();
        } else {
          cached = "";
        }
      } catch {
        cached = "";
      }
      if (alive && cached) setVersion(cached);
    })();
    return () => {
      alive = false;
    };
  }, []);
  return version;
}
