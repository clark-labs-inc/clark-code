import { Loader2 } from "lucide-react";
import type { ReactNode } from "react";
import type { Artifact } from "../../core-bridge/types";

export function isJsonArtifact(artifact: Artifact): boolean {
  return artifact.mime_type?.toLowerCase() === "application/json"
    || /\.json(?:[?#]|\s|$)/i.test(`${artifact.title} ${artifact.uri ?? ""}`);
}

/** Keep generated JSON readable without rendering its contents as markup. */
export function ArtifactJsonPreview({ text, loading, onFallback }: {
  text: string | null;
  loading: boolean;
  onFallback: () => ReactNode;
}) {
  if (loading) {
    return <div className="grid min-h-full place-items-center text-sm text-ink-faint"><Loader2 className="size-4 animate-spin" /> Loading JSON…</div>;
  }
  if (text == null) return onFallback();
  let visible = text;
  try {
    visible = JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    // Show the source verbatim if a file has a JSON suffix but is incomplete.
  }
  return (
    <div className="min-h-full bg-bg px-5 py-6">
      <pre className="overflow-x-auto whitespace-pre-wrap break-words rounded-xl border border-border-subtle bg-bg-secondary p-5 font-mono text-xs leading-5 text-ink-secondary" aria-label="JSON artifact preview">{visible}</pre>
    </div>
  );
}
