import { beforeEach, describe, expect, it, vi } from "vitest";

const productAccessSnapshot = vi.hoisted(() => vi.fn());
const hooks = vi.hoisted(() => {
  type Cleanup = void | (() => void);
  type Slot<T> = { deps: readonly unknown[]; value: T };

  const state: unknown[] = [];
  const refs: Array<{ current: unknown }> = [];
  const callbacks: Array<Slot<unknown>> = [];
  const effects: Array<Slot<Cleanup> | undefined> = [];
  const pendingEffects: Array<() => void> = [];
  let stateCursor = 0;
  let refCursor = 0;
  let callbackCursor = 0;
  let effectCursor = 0;

  const depsMatch = (left: readonly unknown[], right: readonly unknown[]) =>
    left.length === right.length && left.every((value, index) => Object.is(value, right[index]));

  return {
    beginRender() {
      stateCursor = 0;
      refCursor = 0;
      callbackCursor = 0;
      effectCursor = 0;
    },
    flushEffects() {
      while (pendingEffects.length > 0) pendingEffects.shift()?.();
    },
    reset() {
      for (const effect of effects) {
        if (typeof effect?.value === "function") effect.value();
      }
      state.length = 0;
      refs.length = 0;
      callbacks.length = 0;
      effects.length = 0;
      pendingEffects.length = 0;
      this.beginRender();
    },
    useState<T>(initial: T) {
      const slot = stateCursor++;
      if (!(slot in state)) state[slot] = initial;
      const setState = (next: T | ((current: T) => T)) => {
        const current = state[slot] as T;
        state[slot] = typeof next === "function"
          ? (next as (current: T) => T)(current)
          : next;
      };
      return [state[slot] as T, setState] as const;
    },
    useRef<T>(initial: T) {
      const slot = refCursor++;
      if (!refs[slot]) refs[slot] = { current: initial };
      return refs[slot] as { current: T };
    },
    useCallback<T>(callback: T, deps: readonly unknown[]) {
      const slot = callbackCursor++;
      const previous = callbacks[slot] as Slot<T> | undefined;
      if (!previous || !depsMatch(previous.deps, deps)) {
        callbacks[slot] = { deps, value: callback };
      }
      return (callbacks[slot] as Slot<T>).value;
    },
    useEffect(effect: () => Cleanup, deps: readonly unknown[]) {
      const slot = effectCursor++;
      const previous = effects[slot];
      if (previous && depsMatch(previous.deps, deps)) return;
      pendingEffects.push(() => {
        if (typeof previous?.value === "function") previous.value();
        effects[slot] = { deps, value: effect() };
      });
    },
  };
});

vi.mock("react", () => ({
  useCallback: hooks.useCallback,
  useEffect: hooks.useEffect,
  useRef: hooks.useRef,
  useState: hooks.useState,
}));
vi.mock("./productAccess", () => ({ productAccessSnapshot }));

import { useProductAccess } from "./useProductAccess";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

async function flushPromises() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

function render(ownerKey: string) {
  hooks.beginRender();
  const result = useProductAccess(true, ownerKey);
  hooks.flushEffects();
  return result;
}

beforeEach(() => {
  hooks.reset();
  productAccessSnapshot.mockReset();
});

describe("product access ownership", () => {
  it("hides a resolved prior-account snapshot immediately after an account switch", async () => {
    productAccessSnapshot.mockResolvedValueOnce({ schema_version: 1, account: "one" });

    render("account-1");
    await flushPromises();
    expect(render("account-1").access).toEqual({ schema_version: 1, account: "one" });

    const switched = render("account-2");
    expect(switched.access).toBeNull();
    expect(switched.loading).toBe(true);
  });

  it("ignores a late prior-account result after the next account resolves", async () => {
    const first = deferred<unknown>();
    const second = deferred<unknown>();
    productAccessSnapshot
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    render("account-1");
    render("account-2");
    second.resolve({ schema_version: 1, account: "two" });
    await flushPromises();
    expect(render("account-2").access).toEqual({ schema_version: 1, account: "two" });

    first.resolve({ schema_version: 1, account: "one" });
    await flushPromises();
    expect(render("account-2").access).toEqual({ schema_version: 1, account: "two" });
  });
});
