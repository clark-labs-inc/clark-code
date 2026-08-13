import { useEffect, useMemo, useState, type ImgHTMLAttributes } from "react";
import { readImageDataUrl } from "../lib/docs";
import { localPathFromHref } from "../lib/fileLinks";
import { useSessionStore } from "../store/sessionStore";
import { ArtifactFileActions } from "./work/ArtifactFileActions";

/** Resolve agent-authored local Markdown images through the same confined
 * native reader as artifact images. A failed preview stays actionable. */
export function MarkdownImage({ src, alt, className, ...props }: ImgHTMLAttributes<HTMLImageElement>) {
  const cwd = useSessionStore((state) => state.activeProjectRoot ?? state.localSettings.cwd);
  const sessionId = useSessionStore((state) => state.session?.id);
  const remote = useSessionStore((state) => state.activeRemote !== null);
  const path = remote ? null : localPathFromHref(src, cwd);
  const [resolvedSrc, setResolvedSrc] = useState<string | null>(path ? null : (src ?? null));
  const [failed, setFailed] = useState(false);
  const title = alt?.trim() || (path?.split(/[\\/]/).pop() ?? "Image");
  const artifact = useMemo(() => ({
    id: `markdown-image:${path ?? src ?? title}`,
    title,
    kind: "image" as const,
    mime_type: "image/*",
    uri: path ?? src,
  }), [path, src, title]);

  useEffect(() => {
    setFailed(false);
    if (!path) {
      setResolvedSrc(src ?? null);
      return;
    }
    let alive = true;
    setResolvedSrc(null);
    void readImageDataUrl(path, sessionId).then((data) => {
      if (!alive) return;
      if (data) setResolvedSrc(data);
      else setFailed(true);
    });
    return () => {
      alive = false;
    };
  }, [path, sessionId, src]);

  if (!path) {
    return resolvedSrc ? (
      <img
        {...props}
        src={resolvedSrc}
        alt={title}
        className={className}
        loading="lazy"
        decoding="async"
        onError={() => setFailed(true)}
      />
    ) : null;
  }

  return (
    <figure data-local-image={path} className="my-3 overflow-hidden rounded-xl border border-border-subtle bg-bg-secondary/40">
      {resolvedSrc && !failed ? (
        <img
          {...props}
          src={resolvedSrc}
          alt={title}
          className={`max-h-[32rem] w-full object-contain bg-bg-sunken ${className ?? ""}`}
          loading="lazy"
          decoding="async"
          onError={() => setFailed(true)}
        />
      ) : (
        <div className="grid min-h-28 place-items-center px-4 py-8 text-sm text-ink-muted">
          {failed ? "Image preview unavailable" : "Loading image…"}
        </div>
      )}
      <figcaption className="flex flex-wrap items-center gap-2 border-t border-border-subtle px-3 py-2">
        <span className="min-w-0 flex-1 truncate text-xs font-medium text-ink-secondary">{title}</span>
        <ArtifactFileActions artifact={artifact} compact />
      </figcaption>
    </figure>
  );
}
