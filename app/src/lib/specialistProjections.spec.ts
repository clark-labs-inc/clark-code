import { describe, expect, it } from "vitest";

import {
  parseResearchOverview,
  parseRsiOverview,
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
    expect(parseRsiOverview({
      worlds: [],
      evaluations: [],
      runs: [],
      counterexamples: [],
      familyCoverage: {},
      evidenceCount: 0,
      lineage: { nodes: [], edges: [] },
    }).evidenceCount).toBe(0);
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
    expect(() => parseRsiOverview({
      worlds: [],
      evaluations: [],
      runs: [],
      counterexamples: [],
      familyCoverage: {},
      evidenceCount: 0,
      lineage: { nodes: [], edges: [] },
      internalTrajectory: [],
    })).toThrow("v1 projection schema");
  });
});
