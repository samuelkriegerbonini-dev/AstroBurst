import { describe, expect, it } from "vitest";
import { chunkStatsRequests, MAX_REGIONS_PER_STATS_CALL } from "../useRegionStats";
import type { RegionStatsRequest } from "../../services/regions";

function requests(count: number): RegionStatsRequest[] {
  return Array.from({ length: count }, (_, i) => ({
    id: `r${i}`,
    shape: { shape: "circle" as const, x: i, y: i, r: 1 },
    background: null,
  }));
}

describe("chunkStatsRequests", () => {
  it("returns no chunk for an empty region set", () => {
    expect(chunkStatsRequests([])).toEqual([]);
  });

  it("keeps a set at the backend limit in a single call", () => {
    const chunks = chunkStatsRequests(requests(MAX_REGIONS_PER_STATS_CALL));
    expect(chunks).toHaveLength(1);
    expect(chunks[0]).toHaveLength(MAX_REGIONS_PER_STATS_CALL);
  });

  it("splits one region past the limit into two calls", () => {
    const chunks = chunkStatsRequests(requests(MAX_REGIONS_PER_STATS_CALL + 1));
    expect(chunks).toHaveLength(2);
    expect(chunks[0]).toHaveLength(MAX_REGIONS_PER_STATS_CALL);
    expect(chunks[1]).toHaveLength(1);
  });

  it("keeps every chunk within the backend limit for an imported catalogue", () => {
    const chunks = chunkStatsRequests(requests(700));
    expect(chunks.every((c) => c.length <= MAX_REGIONS_PER_STATS_CALL)).toBe(true);
  });

  it("preserves every region exactly once and in order", () => {
    const all = requests(1300);
    const flattened = chunkStatsRequests(all).flat();
    expect(flattened).toHaveLength(all.length);
    expect(flattened.map((r) => r.id)).toEqual(all.map((r) => r.id));
  });
});
