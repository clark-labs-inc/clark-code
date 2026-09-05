import { highlightCacheKey, resolveLang } from "./highlightLanguage";
import type { HighlightResult } from "./highlightEngine";

export { highlightCacheKey, resolveLang } from "./highlightLanguage";
export type { HighlightResult } from "./highlightEngine";

type Output = HighlightResult | string[] | null;
type Mode = "html" | "lines";
export interface HighlightRequest {
  id: number;
  mode: Mode;
  code: string;
  lang: string | undefined;
}
export interface HighlightResponse { id: number; result: Output }

let worker: Worker | null = null;
let unavailable = false;
let sequence = 0;
const pending = new Map<number, { finish: (result: Output) => void; timer: ReturnType<typeof setTimeout> }>();
// Keep fulfilled promises too: repeated renders share both IPC and output.
const cache = new Map<string, Promise<Output>>();
let serverEngine: typeof import("./highlightEngine") | null = null;

async function getServerEngine() {
  return serverEngine ??= await import("./highlightEngine");
}

function failWorker() {
  unavailable = true;
  worker?.terminate();
  worker = null;
  for (const { finish, timer } of pending.values()) {
    clearTimeout(timer);
    finish(null);
  }
  pending.clear();
  cache.clear();
}

function getWorker(): Worker | null {
  if (unavailable) return null;
  if (worker) return worker;
  try {
    worker = new Worker(new URL("./highlight.worker.ts", import.meta.url), { type: "module" });
    worker.onmessage = ({ data }: MessageEvent<HighlightResponse>) => {
      const request = pending.get(data.id);
      if (!request) return;
      pending.delete(data.id);
      clearTimeout(request.timer);
      request.finish(data.result);
    };
    worker.onerror = failWorker;
    worker.onmessageerror = failWorker;
    return worker;
  } catch {
    failWorker();
    return null;
  }
}

function request(mode: Mode, code: string, lang: string | undefined): Promise<Output> {
  if (!resolveLang(lang)) return Promise.resolve(null);
  const key = `${mode}\u0000${highlightCacheKey(lang, code)}`;
  const cached = cache.get(key);
  if (cached) return cached;
  const target = getWorker();
  if (!target) return Promise.resolve(null);
  const result = new Promise<Output>((finish) => {
    const id = ++sequence;
    // A failed worker must never leave components waiting indefinitely.
    const timer = setTimeout(failWorker, 15_000);
    pending.set(id, { finish, timer });
    try {
      target.postMessage({ id, mode, code, lang } satisfies HighlightRequest);
    } catch {
      failWorker();
    }
  });
  cache.set(key, result);
  if (cache.size > 64) cache.delete(cache.keys().next().value!);
  return result;
}

/** Browser tokenization lives entirely off the UI thread. If workers are not
 * available, keep readable plain code instead of blocking user interaction. */
export async function highlight(code: string, lang: string | undefined): Promise<HighlightResult> {
  if (typeof window === "undefined") return (await getServerEngine()).highlight(code, lang);
  const result = await request("html", code, lang);
  return result && !Array.isArray(result) ? result : { html: null, lang: null };
}

export async function highlightLines(code: string, lang: string | undefined): Promise<string[] | null> {
  if (typeof window === "undefined") return (await getServerEngine()).highlightLines(code, lang);
  const result = await request("lines", code, lang);
  return Array.isArray(result) ? result : null;
}

/** Test/cache reset; an in-flight result cannot repopulate this cache. */
export function clearHighlightCaches(): void {
  cache.clear();
  serverEngine?.clearHighlightCaches();
}
