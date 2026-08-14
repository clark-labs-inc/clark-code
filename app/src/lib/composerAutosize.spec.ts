import { describe, expect, it, vi } from "vitest";
import {
  COMPOSER_MAX_HEIGHT,
  observeTextareaWidth,
  resizeComposerTextarea,
  settleComposerTextareaSize,
} from "./composerAutosize";

describe("composer autosizing", () => {
  it("clears a stale inline height before measuring content", () => {
    const style = { height: "200px" } as CSSStyleDeclaration;
    const textarea = {
      style,
      get scrollHeight() {
        return style.height === "auto" ? 24 : 200;
      },
    } as Pick<HTMLTextAreaElement, "scrollHeight" | "style">;

    resizeComposerTextarea(textarea);

    expect(style.height).toBe("24px");
  });

  it("expands ordinary long text up to the maximum height", () => {
    const style = { height: "" } as CSSStyleDeclaration;
    resizeComposerTextarea({ style, scrollHeight: 600 });
    expect(style.height).toBe(`${COMPOSER_MAX_HEIGHT}px`);
  });

  it("settles a stale WebKit measurement after a programmatic value update", () => {
    const style = { height: "" } as CSSStyleDeclaration;
    let measurement = 0;
    const textarea = {
      style,
      get scrollHeight() {
        measurement += 1;
        return measurement <= 2 ? COMPOSER_MAX_HEIGHT : 24;
      },
    } as HTMLTextAreaElement;
    const scheduled: FrameRequestCallback[] = [];
    const cancel = vi.fn();

    resizeComposerTextarea(textarea);
    const stop = settleComposerTextareaSize(textarea, (callback) => {
      scheduled.push(callback);
      return 7 + scheduled.length;
    }, cancel);

    expect(style.height).toBe(`${COMPOSER_MAX_HEIGHT}px`);
    scheduled.shift()?.(0);
    expect(style.height).toBe(`${COMPOSER_MAX_HEIGHT}px`);
    scheduled.shift()?.(16);
    expect(style.height).toBe("24px");
    stop();
    expect(cancel).toHaveBeenCalledWith(8);
  });

  it("remeasures after width changes but ignores height-only notifications", () => {
    const textarea = {} as HTMLTextAreaElement;
    const resize = vi.fn();
    let deliver: ResizeObserverCallback = () => undefined;
    let scheduled: FrameRequestCallback = () => {
      throw new Error("no animation frame scheduled");
    };
    const disconnect = vi.fn();
    const observe = vi.fn();
    const stop = observeTextareaWidth(textarea, resize, (callback) => {
      deliver = callback;
      return { observe, disconnect };
    }, (callback) => {
      scheduled = callback;
      return 1;
    }, vi.fn());
    const entry = (width: number) =>
      ({ target: textarea, contentRect: { width } }) as unknown as ResizeObserverEntry;

    deliver([entry(0)], {} as ResizeObserver);
    expect(resize).not.toHaveBeenCalled();
    scheduled(0);
    deliver([entry(640)], {} as ResizeObserver);
    scheduled(0);
    deliver([entry(640)], {} as ResizeObserver);

    expect(observe).toHaveBeenCalledWith(textarea);
    expect(resize).toHaveBeenCalledTimes(2);
    stop();
    expect(disconnect).toHaveBeenCalledOnce();
  });
});
