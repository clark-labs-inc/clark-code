/** `@clark-perf` is resolved by a Vite alias to either the real recorder
 *  installer or an empty stand-in, so its type is declared once here rather
 *  than depending on which file the current build selected. */
declare module "@clark-perf" {
  export function installPerfHooks(options?: {
    store: {
      subscribe: (listener: (state: unknown) => void) => () => void;
      getState: () => { snapshot?: { timeline?: unknown[]; tool_calls?: object } };
      setState: (partial: Record<string, unknown>) => void;
    };
    getBridge?: unknown;
  }): void;
}
