import { useEffect, useRef, useState } from "react";
import type { PDFDocumentProxy } from "pdfjs-dist";
import { FileWarning, Loader2 } from "lucide-react";

type PdfLoadingTask = {
  promise: Promise<PDFDocumentProxy>;
  destroy: () => Promise<void>;
};

export function BrowserPdfPreview({ uri, title }: { uri: string; title: string }) {
  const [pdf, setPdf] = useState<PDFDocumentProxy | null>(null);
  const [error, setError] = useState(false);

  useEffect(() => {
    let alive = true;
    let loadingTask: PdfLoadingTask | null = null;
    setPdf(null);
    setError(false);

    void (async () => {
      try {
        const [pdfjs, response] = await Promise.all([import("pdfjs-dist"), fetch(uri)]);
        if (!response.ok) throw new Error(`PDF request failed with ${response.status}`);
        pdfjs.GlobalWorkerOptions.workerSrc = new URL(
          "pdfjs-dist/build/pdf.worker.min.mjs",
          import.meta.url,
        ).toString();
        const bytes = new Uint8Array(await response.arrayBuffer());
        loadingTask = pdfjs.getDocument({ data: bytes });
        const value = await loadingTask.promise;
        if (alive) setPdf(value);
        else await loadingTask.destroy().catch(() => undefined);
      } catch {
        if (alive) setError(true);
      }
    })();

    return () => {
      alive = false;
      if (loadingTask) void loadingTask.destroy().catch(() => undefined);
    };
  }, [uri]);

  if (error) {
    return (
      <div className="grid min-h-[38rem] place-items-center px-8 text-center">
        <div>
          <FileWarning className="mx-auto size-8 text-ink-faint" />
          <p className="mt-3 text-sm font-medium text-ink-secondary">PDF preview unavailable</p>
          <p className="mt-1 text-xs text-ink-faint">Use the file actions to open or save it.</p>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-5 bg-bg-sunken/60 p-4 sm:p-7" aria-label={`Preview of ${title}`}>
      {pdf ? (
        Array.from({ length: pdf.numPages }, (_, index) => (
          <BrowserPdfPage key={index + 1} pdf={pdf} page={index + 1} title={title} />
        ))
      ) : (
        <LoadingPage />
      )}
    </div>
  );
}

function BrowserPdfPage({
  pdf,
  page,
  title,
}: {
  pdf: PDFDocumentProxy;
  page: number;
  title: string;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(page === 1);
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
    void (async () => {
      try {
        const pdfPage = await pdf.getPage(page);
        const viewport = pdfPage.getViewport({ scale: 1.6 });
        const canvas = globalThis.document.createElement("canvas");
        canvas.width = Math.ceil(viewport.width);
        canvas.height = Math.ceil(viewport.height);
        const context = canvas.getContext("2d", { alpha: false });
        if (!context) throw new Error("Canvas rendering is unavailable");
        await pdfPage.render({ canvas, canvasContext: context, viewport }).promise;
        objectUrl = URL.createObjectURL(await canvasToBlob(canvas));
        pdfPage.cleanup();
        if (alive) setUrl(objectUrl);
        else URL.revokeObjectURL(objectUrl);
      } catch {
        if (alive) setFailed(true);
      }
    })();
    return () => {
      alive = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [page, pdf, visible]);

  return (
    <div ref={containerRef} className="mx-auto grid min-h-80 w-full max-w-5xl place-items-center">
      {url ? (
        <img
          src={url}
          alt={`${title}, page ${page}`}
          className="block h-auto w-full bg-media-page shadow-lifted"
        />
      ) : failed ? (
        <span className="text-xs text-ink-faint">Page {page} unavailable</span>
      ) : (
        <LoadingPage />
      )}
    </div>
  );
}

function LoadingPage() {
  return (
    <div className="grid min-h-80 place-items-center text-sm text-ink-faint">
      <span className="flex items-center gap-2">
        <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" /> Rendering PDF…
      </span>
    </div>
  );
}

function canvasToBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob) resolve(blob);
      else reject(new Error("PDF page could not be encoded"));
    }, "image/png");
  });
}
