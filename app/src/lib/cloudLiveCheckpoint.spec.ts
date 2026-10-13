import { describe, expect, it } from "vitest";
import { CloudLiveCheckpointBounds } from "./cloudLiveCheckpoint";
import { emptySnapshot, type Snapshot } from "../core-bridge/types";
describe("cloud publication ordering", () => {
  it("does not stop streaming solely because an unpaged conversation has hundreds of small turns", () => {
    const bounds = new CloudLiveCheckpointBounds();
    const snapshot: Snapshot = { ...emptySnapshot(), timeline: Array.from({ length: 500 }, (_, index) => ({
      item: "message", role: "agent", run: "run", blocks: [{ type: "text", text: `turn-${index}` }],
    })) };
    expect(bounds.deferLive("chat", snapshot)).toBe(false);
    snapshot.timeline[0] = { item: "message", role: "user", run: "user", blocks: [{ type: "text", text: "x".repeat(1024 * 1024 + 1) }] };
    expect(bounds.deferLive("chat", snapshot)).toBe(true);
    bounds.record("chat", 1);
    expect(bounds.deferLive("chat", snapshot)).toBe(false);
  });
  it("advances the snapshot rev when consecutive publications share a clock tick", () => {
    const bounds = new CloudLiveCheckpointBounds();
    expect(bounds.nextRevision("chat", 1000, 10)).toBe(1000);
    expect(bounds.nextRevision("chat", 1000, 10)).toBe(1001);
    expect(bounds.nextRevision("chat", 999, 1005)).toBe(1006);
    bounds.clear();
    expect(bounds.nextRevision("chat", 1000, 1006)).toBe(1007);
  });
});
