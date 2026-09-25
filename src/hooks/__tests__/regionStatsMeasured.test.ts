import { describe, expect, it } from "vitest";
import { measuredOfChunks } from "../useRegionStats";
import type { RegionStatsResult } from "../../shared/types/regions";

function chunk(extra: Partial<RegionStatsResult>): RegionStatsResult {
  return { regions: [], masked: false, dq_excluded: null, elapsed_ms: 0, ...extra };
}

describe("measuredOfChunks", () => {
  it("takes dq_excluded once, ORs masked and sums elapsed", () => {
    expect(measuredOfChunks([chunk({ dq_excluded: 10, masked: true, elapsed_ms: 5 }), chunk({ dq_excluded: 10, masked: true, elapsed_ms: 7 })])).toEqual({
      masked: true,
      dqExcluded: 10,
      elapsedMs: 12,
    });
    expect(measuredOfChunks([chunk({ masked: false }), chunk({ masked: false })])).toEqual({ masked: false, dqExcluded: null, elapsedMs: 0 });
    expect(measuredOfChunks([chunk({ masked: false, elapsed_ms: 1 }), chunk({ masked: true, dq_excluded: 3, elapsed_ms: 2 })])).toEqual({
      masked: true,
      dqExcluded: 3,
      elapsedMs: 3,
    });
    expect(measuredOfChunks([])).toEqual({ masked: false, dqExcluded: null, elapsedMs: 0 });
  });
});
