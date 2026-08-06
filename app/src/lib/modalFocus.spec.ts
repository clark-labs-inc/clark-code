import { describe, expect, it, vi } from "vitest";
import { restoreModalFocus, type ModalFocusTarget } from "./modalFocus";

describe("restoreModalFocus", () => {
  it("returns focus to a connected opener without scrolling", () => {
    const focus = vi.fn();
    const target: ModalFocusTarget = { isConnected: true, focus };

    restoreModalFocus(target);

    expect(focus).toHaveBeenCalledWith({ preventScroll: true });
  });

  it("does not focus an opener that was removed while the modal was open", () => {
    const focus = vi.fn();
    const target: ModalFocusTarget = { isConnected: false, focus };

    restoreModalFocus(target);

    expect(focus).not.toHaveBeenCalled();
  });
});
