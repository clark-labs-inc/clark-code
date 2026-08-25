import { describe, expect, it } from "vitest";
import { specialistReadRoots } from "./specialists";

describe("specialistReadRoots", () => {
  it("preserves Scout's account-scoped census roots", () => {
    expect(specialistReadRoots(
      { kind: "scout" },
      ["/repos/payments", "/repos/identity"],
    )).toEqual(["/repos/payments", "/repos/identity"]);
  });

  it("grants no filesystem roots to unregistered lenses", () => {
    expect(specialistReadRoots({ kind: "rsi" }, ["/repos/recent"])).toEqual([]);
  });
});
