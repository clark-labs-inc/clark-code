import { useEffect, useRef, useState } from "react";
import { ExternalLink, FileWarning, Loader2 } from "lucide-react";
import {
  cleanupDocumentPreview,
  readDocumentPreview,
  readDocumentPreviewPage,
  type DocumentPreview as Preview,
} from "../../lib/docs";
import { BrowserPdfPreview } from "./BrowserPdfPreview";

export function DocumentPreview({
  title,
  uri,
  mimeType,
  onOpen,
}: {
  title: string;
  uri?: string;
  mimeType?: string;
  onOpen?: () => void;
}) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    let previewId: string | null = null;
    setPreview(null);
    setLoading(true);
    void readDocumentPreview(uri, title, mimeType).then((value) => {
      if (value?.kind === "pages") previewId = value.preview_id;
      if (!alive) {
        if (previewId) void cleanupDocumentPreview(previewId).catch(() => undefined);
        return;
      }
      setPreview(value);
      setLoading(false);
    });
    return () => {
      alive = false;
      if (previewId) void cleanupDocumentPreview(previewId).catch(() => undefined);
    };
  }, [mimeType, title, uri]);

  if (loading) {
    return (
      <div className="grid min-h-full place-items-center text-sm text-ink-faint">
        <span className="flex items-center gap-2">
          <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" /> Rendering document…
        </span>
      </div>
    );
  }

  if (!preview) {
    return (
      <div className="grid min-h-full place-items-center px-8 py-12 text-center">
        <div>
          <FileWarning className="mx-auto size-8 text-ink-faint" />
          <p className="mt-3 text-sm font-medium text-ink-secondary">Preview unavailable</p>
          <p className="mt-1 text-xs text-ink-faint">
            {uri
              ? "Clark Code couldn’t render this file inline. Use the available file actions to open or save it."
              : "This artifact has no attached file. Ask Clark Code to recreate it."}
          </p>
          {onOpen && (
            <button
              type="button"
              onClick={onOpen}
              className="mt-5 inline-flex h-9 items-center gap-2 rounded-lg bg-accent px-3.5 text-sm font-medium text-on-accent transition hover:bg-accent-hover"
            >
              Open {title} <ExternalLink className="size-3.5" />
            </button>
          )}
        </div>
      </div>
    );
  }

  if (preview.kind === "pages") {
    return (
      <div className="space-y-5 bg-bg-sunken/60 p-4 sm:p-7">
        {Array.from({ length: preview.page_count }, (_, index) => (
          <PreviewPage
            key={`${title}-page-${index + 1}`}
            previewId={preview.preview_id}
            page={index}
            title={title}
          />
        ))}
      </div>
    );
  }

  if (preview.kind === "direct") {
    return (
      <div className="h-full min-h-[40rem] bg-bg-sunken/45 p-3 sm:p-5">
        <BrowserPdfPreview uri={preview.uri} title={title} />
      </div>
    );
  }

  return (
    <div className="h-full min-h-[40rem] bg-bg-sunken/45 p-3 sm:p-5">
      <iframe
        title={`Preview of ${title}`}
        srcDoc={preview.html}
        sandbox=""
        className="h-full min-h-[38rem] w-full rounded-lg border border-border bg-white shadow-soft"
      />
    </div>
  );
}

function PreviewPage({ previewId, page, title }: { previewId: string; page: number; title: string }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(page === 0);
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (visible || !containerRef.current) return;
    if (typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "1200px 0px" },
    );
    observer.observe(containerRef.current);
    return () => observer.disconnect();
  }, [visible]);

  useEffect(() => {
    if (!visible) return;
    let alive = true;
    let objectUrl: string | null = null;
    void readDocumentPreviewPage(previewId, page)
      .then((value) => {
        objectUrl = value;
        if (alive) setUrl(value);
        else URL.revokeObjectURL(value);
      })
      .catch(() => {
        if (alive) setFailed(true);
      });
    return () => {
      alive = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [page, previewId, visible]);

  return (
    <div
      ref={containerRef}
      className="mx-auto grid min-h-80 w-full max-w-5xl place-items-center bg-white shadow-lifted"
    >
      {url ? (
        <img src={url} alt={`${title}, page ${page + 1}`} className="block h-auto w-full" />
      ) : failed ? (
        <span className="text-xs text-ink-faint">Page {page + 1} unavailable</span>
      ) : (
        <Loader2 className="size-4 animate-[spin_1s_linear_infinite] text-ink-faint" />
      )}
    </div>
  );
}
