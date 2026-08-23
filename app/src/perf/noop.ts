/** Stand-in for the performance recorder in every normal build.
 *
 *  `vite.config.ts` aliases `@clark-perf` here unless `VITE_PERF_HOOKS=1`, so
 *  the recorder's code is absent from the bundle rather than merely unreachable.
 *  Keep this file free of imports and side effects — there must be nothing for
 *  a bundler to retain. */
export function installPerfHooks(): void {}
