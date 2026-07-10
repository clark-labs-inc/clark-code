// Mermaid diagram rendering, lazy-loaded so the heavy mermaid bundle (~1MB) is
// only pulled in when a diagram actually appears — never on the streaming
// message path. Renders a fenced ```mermaid block to inline SVG.

import { useEffect, useState, useId } from "react";
import { AlertTriangle } from "lucide-react";

/** Render `code` (a Mermaid graph definition) to inline SVG. Shows a plain
 *  fallback while the library loads, and an error note if the diagram fails to
 *  parse (so a malformed block isn't a blank box). */
export function Mermaid({ code }: { code: string }) {
  const [svg, setSvg] = useState<string | null>(null);
  const [err, setErr] = useState(false);
  // Stable unique id for the render target — mermaid.render needs one.
  const rawId = useId();
  const id = `mmd-${rawId.replace(/[^a-zA-Z0-9]/g, "")}`;

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const mermaid = (await import("mermaid")).default;
        mermaid.initialize({ startOnLoad: false, securityLevel: "loose", theme: "default" });
        const { svg: out } = await mermaid.render(id, code);
        if (alive) setSvg(out);
      } catch {
        if (alive) setErr(true);
      }
    })();
    return () => {
      alive = false;
    };
  }, [code, id]);

  if (err) {
    return (
      <div className="flex items-center gap-2 rounded-md border border-border-subtle bg-bg-sunken px-3 py-2 text-xs text-ink-muted">
        <AlertTriangle className="size-3.5 shrink-0 text-warning" />
        <span>Couldn’t render this diagram.</span>
      </div>
    );
  }
  if (!svg) {
    // Skeleton while mermaid loads/renders.
    return <div className="skeleton h-32 w-full rounded-md" />;
  }
  return (
    <div
      className="mermaid-host my-2 flex justify-center overflow-x-auto"
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}
