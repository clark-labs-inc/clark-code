// Node 25 exposes a partial `localStorage` global unless it receives a valid
// `--localstorage-file`; touching it in isolated Vitest workers emits warnings
// and does not provide the Storage API. Install a deterministic in-memory
// browser-compatible substitute before application modules load.
if (typeof window === "undefined") {
  const values = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    writable: true,
    value: {
      get length() { return values.size; },
      clear: () => values.clear(),
      getItem: (key: string) => values.get(key) ?? null,
      key: (index: number) => [...values.keys()][index] ?? null,
      removeItem: (key: string) => { values.delete(key); },
      setItem: (key: string, value: string) => { values.set(key, String(value)); },
    } satisfies Storage,
  });
}
