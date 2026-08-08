import { describe, expect, it } from "vitest";
import {
  conversationScrollTarget,
  isConversationAtBottom,
  nextPinnedScrollTop,
  shouldFollowConversation,
} from "./conversationScroll";

describe("conversation scroll state", () => {
  it("opens new and previously pinned conversations at the latest output", () => {
    expect(conversationScrollTarget(undefined, false, 900)).toBe(900);
    expect(conversationScrollTarget({ scrollTop: 240, atBottom: true }, false, 900)).toBe(900);
  });

  it("opens a running conversation at the latest output", () => {
    expect(conversationScrollTarget({ scrollTop: 240, atBottom: false }, true, 900)).toBe(900);
  });

  it("restores deliberate scrollback for an idle conversation", () => {
    expect(conversationScrollTarget({ scrollTop: 240, atBottom: false }, false, 900)).toBe(240);
  });

  it("uses the same near-bottom threshold as the jump-to-latest control", () => {
    expect(isConversationAtBottom(1_000, 305, 600)).toBe(true);
    expect(isConversationAtBottom(1_000, 304, 600)).toBe(false);
  });

  it("stops following as soon as a user scrolls upward near the bottom", () => {
    expect(shouldFollowConversation(400, 380, true, false, true)).toBe(false);
    expect(shouldFollowConversation(400, 380, true, true, true)).toBe(false);
  });

  it("resumes following after the user scrolls back to the bottom", () => {
    expect(shouldFollowConversation(380, 400, true)).toBe(true);
  });

  it("keeps an explicit jump-to-latest animation pinned while it travels", () => {
    expect(shouldFollowConversation(200, 300, false, true)).toBe(true);
  });

  it("keeps following when a layout clamp lowers scrollTop at the bottom", () => {
    expect(shouldFollowConversation(830, 800, true)).toBe(true);
  });

  it("eases ordinary streamed growth instead of snapping to the new bottom", () => {
    expect(nextPinnedScrollTop(400, 440)).toBeCloseTo(408.8);
    expect(nextPinnedScrollTop(439.8, 440)).toBe(440);
  });

  it("caps tall content jumps while always moving forward", () => {
    expect(nextPinnedScrollTop(100, 1_000)).toBe(172);
    expect(nextPinnedScrollTop(100, 101)).toBe(101);
  });
});
