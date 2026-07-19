import { describe, expect, it } from "vitest";
import {
  conversationScrollTarget,
  isConversationAtBottom,
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
});
