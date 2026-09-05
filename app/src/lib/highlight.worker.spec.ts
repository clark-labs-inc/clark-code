import { afterEach, describe, expect, it, vi } from "vitest";

class FakeWorker {
  static instance: FakeWorker;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onmessageerror: (() => void) | null = null;
  messages: Array<{ id: number; code: string; mode: string }> = [];
  terminate = vi.fn();
  constructor() { FakeWorker.instance = this; }
  postMessage(message: { id: number; code: string; mode: string }) { this.messages.push(message); }
  reply(index: number, result: unknown) {
    this.onmessage?.({ data: { id: this.messages[index].id, result } });
  }
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  vi.resetModules();
});

async function setup() {
  vi.stubGlobal("window", {});
  vi.stubGlobal("Worker", FakeWorker);
  return import("./highlight");
}

describe("background highlighting", () => {
  it("deduplicates identical work and associates out-of-order replies with their source", async () => {
    const { highlight } = await setup();
    const first = highlight("const first = 1", "ts");
    const duplicate = highlight("const first = 1", "ts");
    const second = highlight("const second = 2", "ts");
    const worker = FakeWorker.instance;
    expect(worker.messages).toHaveLength(2);
    const newer = { html: "second", lang: "typescript" };
    worker.reply(1, newer);
    expect(await second).toBe(newer);
    const older = { html: "first", lang: "typescript" };
    worker.reply(0, older);
    expect(await first).toBe(older);
    expect(await duplicate).toBe(older);
  });

  it("keeps line and block output distinct", async () => {
    const { highlight, highlightLines } = await setup();
    const block = highlight("let x", "ts");
    const lines = highlightLines("let x", "ts");
    const worker = FakeWorker.instance;
    expect(worker.messages.map((message) => message.mode)).toEqual(["html", "lines"]);
    worker.reply(0, { html: "block", lang: "typescript" });
    worker.reply(1, ["line"]);
    expect((await block).html).toBe("block");
    expect(await lines).toEqual(["line"]);
  });

  it("settles every pending call as plain code when the worker fails", async () => {
    const { highlight, highlightLines } = await setup();
    const block = highlight("let x", "ts");
    const lines = highlightLines("let y", "ts");
    FakeWorker.instance.onerror?.();
    expect(await block).toEqual({ html: null, lang: null });
    expect(await lines).toBeNull();
    expect(FakeWorker.instance.terminate).toHaveBeenCalledOnce();
    expect(await highlight("let z", "ts")).toEqual({ html: null, lang: null });
  });

  it("bounds a worker stall without blocking readable code", async () => {
    vi.useFakeTimers();
    const { highlight } = await setup();
    const output = highlight("let x", "ts");
    vi.advanceTimersByTime(15_000);
    expect(await output).toEqual({ html: null, lang: null });
    expect(FakeWorker.instance.terminate).toHaveBeenCalledOnce();
  });
});
