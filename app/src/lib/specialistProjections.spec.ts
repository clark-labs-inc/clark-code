import { describe, expect, it } from "vitest";

import {
  parseResearchOverview,
} from "./specialistProjections";

describe("specialist projection contracts", () => {
  it("accepts the empty v1 cloud projections", () => {
    expect(parseResearchOverview({
      programs: [],
      campaigns: [],
      experiments: [],
      runs: [],
      evidenceCount: 0,
      supportedClaimCount: 0,
    })).toEqual({
      programs: [],
      campaigns: [],
      experiments: [],
      runs: [],
      evidenceCount: 0,
      supportedClaimCount: 0,
    });
  });

  it("fails closed on malformed or drifted projection data", () => {
    expect(() => parseResearchOverview({
      programs: [],
      campaigns: [],
      experiments: [],
      runs: [],
      evidenceCount: -1,
      supportedClaimCount: 0,
    })).toThrow("non-negative safe integer");
  });
});
