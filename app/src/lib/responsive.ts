import { useEffect, useState } from "react";

/** True when the viewport is narrower than `maxPx`; live-updates on resize. */
export function useIsNarrow(maxPx: number): boolean {
  const [narrow, setNarrow] = useState(
    () => typeof window !== "undefined" && window.innerWidth < maxPx,
  );
  useEffect(() => {
    const mq = window.matchMedia(`(max-width: ${maxPx - 1}px)`);
    const on = () => setNarrow(mq.matches);
    on();
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, [maxPx]);
  return narrow;
}
