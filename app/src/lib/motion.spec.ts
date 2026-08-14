import { describe, expect, it } from "vitest";
import {
  CHAT_REDUCED_ROW_MOTION,
  CHAT_REDUCED_TEXT_ANIMATION,
  CHAT_TEXT_ANIMATION,
  DUR,
  EASE,
  EXPAND,
  EXPAND_REDUCED,
  REDUCED_EXIT,
  RISE_SMALL,
  SCREEN_FADE,
  SLIDE_LEFT,
  SLIDE_RIGHT,
  accessibleMotion,
  chatRowMotion,
  commitChatRowKeys,
  createChatRowMotionState,
  enteringChatRowKeys,
  indeterminateTransition,
  staggeredTransition,
} from "./motion";

describe("motion policy", () => {
  it("uses one expand policy for conversation transients", () => {
    expect(EXPAND.initial).toMatchObject({ opacity: 0, height: 0 });
    expect(EXPAND_REDUCED.initial).toBe(false);
    expect(EXPAND_REDUCED.transition.duration).toBe(0);
  });

  it("keeps reduced-motion transients as a short opacity-only exit fade", () => {
    // Enter snaps in (no spatial movement), but the exit stays a fade instead
    // of an invisible hard cut — vanishing surfaces read as a glitch.
    expect(EXPAND_REDUCED.exit).toMatchObject({ opacity: 0 });
    expect((EXPAND_REDUCED.exit as { transition: { duration: number } }).transition.duration)
      .toBeGreaterThan(0);
    expect(JSON.stringify(EXPAND_REDUCED.exit)).not.toContain("translate");
    expect(JSON.stringify(EXPAND_REDUCED.exit)).not.toContain("height");
  });

  it("exposes one reduced-motion exit preset for surfaces with bespoke props", () => {
    expect(REDUCED_EXIT).toMatchObject({ opacity: 0 });
    expect((REDUCED_EXIT as { transition: { duration: number } }).transition.duration)
      .toBeGreaterThan(0);
    expect(JSON.stringify(REDUCED_EXIT)).not.toContain("translate");
  });

  it("offers a small rise for dense rows matching the sidebar/work-line idiom", () => {
    expect(RISE_SMALL.initial).toMatchObject({ opacity: 0, transform: "translateY(4px)" });
    expect(RISE_SMALL.animate).toMatchObject({ opacity: 1, transform: "translateY(0)" });
    // Dense rows normlize their exit to a fade-only — no spatial drift leaving.
    expect(JSON.stringify(RISE_SMALL.exit)).not.toContain("translate");
    expect((RISE_SMALL.exit as { transition: { duration: number } }).transition.duration)
      .toBeGreaterThan(0);
  });

  it("offers directional slides for tab/stage panel swaps", () => {
    expect(SLIDE_LEFT.initial).toMatchObject({ opacity: 0, transform: "translateX(-5px)" });
    expect(SLIDE_RIGHT.initial).toMatchObject({ opacity: 0, transform: "translateX(5px)" });
    expect(SLIDE_LEFT.animate).toMatchObject({ transform: "translateX(0)" });
    expect(SLIDE_RIGHT.animate).toMatchObject({ transform: "translateX(0)" });
  });

  it("keeps primary screen changes spatially stable", () => {
    expect(SCREEN_FADE.initial).toEqual({ opacity: 0 });
    expect(SCREEN_FADE.animate).toEqual({ opacity: 1 });
    expect(SCREEN_FADE.exit).toMatchObject({ opacity: 0 });
    expect(JSON.stringify(SCREEN_FADE)).not.toContain("translate");
    expect(JSON.stringify(SCREEN_FADE)).not.toContain("scale");
  });

  it("staggeredTransition preserves per-index choreography and overrides", () => {
    const base = staggeredTransition(false, 2, 0.045);
    expect(base.duration).toBe(DUR.base);
    expect(base.ease).toEqual(EASE.out);
    expect(base.delay).toBeCloseTo(0.09, 10);

    const slow = staggeredTransition(false, 3, 0.05, { duration: DUR.slow });
    expect(slow.duration).toBe(DUR.slow);
    expect(slow.ease).toEqual(EASE.out);
    expect(slow.delay).toBeCloseTo(0.15, 10);

    // A fixed delay survives as an explicit override (not per-index math).
    expect(staggeredTransition(false, 0, 0.04, { delay: 0.08 }).delay).toBe(0.08);
  });

  it("staggeredTransition zeroes choreography under reduced motion", () => {
    expect(staggeredTransition(true, 5, 0.05)).toEqual({ duration: 0 });
    expect(staggeredTransition(true, 0, 0.04, { delay: 0.08 })).toEqual({ duration: 0 });
  });

  it("accessibleMotion keeps the new transform presets spatial-movement-free under reduce", () => {
    for (const preset of [RISE_SMALL, SLIDE_LEFT, SLIDE_RIGHT]) {
      const reduced = accessibleMotion(preset, true);
      expect(JSON.stringify(reduced)).not.toContain("translate");
      expect(reduced.transition).toMatchObject({ duration: 0 });
      expect(reduced.initial).toBe(false);
      expect(reduced.animate).toEqual({ opacity: 1 });
      expect(reduced.animate).not.toHaveProperty("transform");
    }
  });

  it("uses word-level fades instead of character-by-character typing", () => {
    expect(CHAT_TEXT_ANIMATION).toMatchObject({ animation: "fadeIn", sep: "word" });
    expect(CHAT_TEXT_ANIMATION.stagger).toBeGreaterThan(0);
    expect(CHAT_REDUCED_TEXT_ANIMATION).toBe(false);
  });

  it("uses one accessible cadence for indeterminate progress", () => {
    expect(indeterminateTransition(false)).toEqual({
      duration: 1.1,
      ease: EASE.inOut,
      repeat: Infinity,
    });
    expect(indeterminateTransition(false, 0.18)).toMatchObject({ delay: 0.18 });
    expect(indeterminateTransition(true, 0.18)).toEqual({ duration: 0 });
  });

  it("gives user and assistant rows distinct compositor-friendly entrances", () => {
    expect(chatRowMotion("user").initial).toMatchObject({
      opacity: 0,
      transform: expect.stringContaining("translate3d(8px"),
    });
    expect(chatRowMotion("agent").initial).toMatchObject({
      opacity: 0,
      transform: expect.stringContaining("translate3d(0, 7px"),
    });
  });

  it("keeps reduced motion as an opacity fade without spatial movement", () => {
    expect(CHAT_REDUCED_ROW_MOTION.initial).toEqual({ opacity: 0 });
    expect(CHAT_REDUCED_ROW_MOTION.animate).toEqual({ opacity: 1 });
    expect(JSON.stringify(CHAT_REDUCED_ROW_MOTION)).not.toContain("transform");
  });

  it("animates only rows appended after the current conversation is committed", () => {
    const state = createChatRowMotionState();

    expect(enteringChatRowKeys(state, "conversation-a", ["old-user", "old-agent"]).size)
      .toBe(0);
    commitChatRowKeys(state, "conversation-a", ["old-user", "old-agent"]);

    expect([...enteringChatRowKeys(state, "conversation-a", [
      "old-user",
      "old-agent",
      "new-user",
    ])]).toEqual(["new-user"]);
    commitChatRowKeys(state, "conversation-a", ["old-user", "old-agent", "new-user"]);
    expect(enteringChatRowKeys(
      state,
      "conversation-a",
      ["old-user", "old-agent", "new-user"],
    ).size).toBe(0);
  });

  it("treats a switched conversation's replayed history as settled", () => {
    const state = createChatRowMotionState();
    commitChatRowKeys(state, "conversation-a", ["a-user"]);

    expect(enteringChatRowKeys(state, "conversation-b", ["b-user", "b-agent"]).size)
      .toBe(0);
    commitChatRowKeys(state, "conversation-b", ["b-user", "b-agent"]);
    expect(enteringChatRowKeys(state, "conversation-b", ["b-user", "b-agent"]).size)
      .toBe(0);
  });
});
