import { describe, expect, it, vi } from "vitest";

import type { CompanyScoutMap } from "../../lib/specialistCloud";
import { scoutLoadedCutLabel, scoutSequenceLabel } from "./ScoutCanvas";

function companyMap(overrides: Partial<CompanyScoutMap> = {}): CompanyScoutMap {
  return {
    id: "company-map-1",
    organization_id: "organization-1",
    stable_key: "company-scout",
    display_name: "Example Company Scout",
    status: "active",
    latest_change_sequence: 0,
    source_count: 0,
    active_machine_count: 0,
    run_count: 0,
    simulation_count: 0,
    updated_at_ms: Date.now() - 15 * 24 * 60 * 60 * 1_000,
    ...overrides,
  };
}

describe("Scout evidence freshness copy", () => {
  it("does not present sequence zero as a latest evidence receipt", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-15T12:00:00Z"));
    const empty = companyMap({
      updated_at_ms: new Date("2026-07-31T12:00:00Z").getTime(),
    });

    expect(scoutSequenceLabel(empty.latest_change_sequence)).toBe("None yet");
    expect(scoutLoadedCutLabel(empty)).toBe(
      "no accepted evidence changes · company map updated 15d ago",
    );
    vi.useRealTimers();
  });

  it("names the exact loaded evidence cut when one exists", () => {
    expect(scoutSequenceLabel(42)).toBe("#42");
    expect(scoutLoadedCutLabel(companyMap({ latest_change_sequence: 42 }))).toContain(
      "loaded evidence cut #42",
    );
  });
});
