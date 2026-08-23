/// <reference types="vite/client" />

/** Build-time performance-instrumentation flag. A literal `false` in every
 *  normal build (see `perfHooks` in vite.config.ts), so the instrumentation is
 *  removed by dead-code elimination rather than merely unreachable. */
declare const __CLARK_PERF__: boolean;
