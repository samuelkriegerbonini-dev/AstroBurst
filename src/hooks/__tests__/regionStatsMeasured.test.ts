import { describe, expect, it } from "vitest";
import { MAX_REGIONS_PER_STATS_CALL, measuredOf, measuredOfChunks, notesOfChunks, statsCalls, unitsOfChunks } from "../useRegionStats";
import type { Region, RegionShape, RegionStatsResult } from "../../shared/types/regions";

function chunk(extra: Partial<RegionStatsResult>): RegionStatsResult {
  return { regions: [], masked: false, dq_excluded: null, region_excluded: 0, elapsed_ms: 0, ...extra };
}

function region(id: string, shape: RegionShape, include = true, backgroundId: string | null = null): Region {
  return { id, shape, props: { color: null, width: null, text: null, dash: null, include }, backgroundId };
}

describe("measuredOfChunks", () => {
  it("takes dq_excluded once, ORs masked and sums elapsed", () => {
    expect(measuredOfChunks([chunk({ dq_excluded: 10, masked: true, elapsed_ms: 5 }), chunk({ dq_excluded: 10, masked: true, elapsed_ms: 7 })])).toEqual({
      masked: true,
      dqExcluded: 10,
      regionExcluded: 0,
      elapsedMs: 12,
    });
    expect(measuredOfChunks([chunk({ masked: false }), chunk({ masked: false })])).toEqual({ masked: false, dqExcluded: null, regionExcluded: 0, elapsedMs: 0 });
    expect(measuredOfChunks([chunk({ masked: false, elapsed_ms: 1 }), chunk({ masked: true, dq_excluded: 3, elapsed_ms: 2 })])).toEqual({
      masked: true,
      dqExcluded: 3,
      regionExcluded: 0,
      elapsedMs: 3,
    });
    expect(measuredOfChunks([])).toEqual({ masked: false, dqExcluded: null, regionExcluded: 0, elapsedMs: 0 });
  });

  it("takes region_excluded from the first chunk measured with the exclude mask, not summed", () => {
    expect(
      measuredOfChunks([
        chunk({ masked: true, region_excluded: 81, elapsed_ms: 2 }),
        chunk({ masked: true, region_excluded: 81, elapsed_ms: 2 }),
        chunk({ masked: false, region_excluded: 0, elapsed_ms: 1 }),
      ]),
    ).toEqual({ masked: true, dqExcluded: null, regionExcluded: 81, elapsedMs: 5 });
  });
});

describe("notesOfChunks", () => {
  it("notesOfChunks keeps each note once in the order first seen", () => {
    expect(notesOfChunks([chunk({ notes: ["a"] }), chunk({ notes: ["b", "a"] }), chunk({})])).toEqual(["a", "b"]);
    expect(notesOfChunks([])).toEqual([]);
    expect(notesOfChunks([chunk({})])).toEqual([]);
  });
});

describe("unitsOfChunks", () => {
  it("unitsOfChunks takes the units of the first chunk that names them", () => {
    expect(unitsOfChunks([chunk({}), chunk({ bunit: "MJy/sr", summed_unit: "MJy/sr x pix" }), chunk({ bunit: "adu" })])).toEqual({
      bunit: "MJy/sr",
      summedUnit: "MJy/sr x pix",
    });
    expect(unitsOfChunks([chunk({ bunit: null, summed_unit: null })])).toEqual({ bunit: null, summedUnit: null });
    expect(unitsOfChunks([])).toEqual({ bunit: null, summedUnit: null });
  });
});

describe("measuredOf", () => {
  it("measuredOf joins the chunk facts with the notes and the units", () => {
    const note = "DQ exclusion requested but not applied: this image has no DQ plane";
    const r = region("a", { shape: "circle", x: 5, y: 5, r: 2 });
    const regions = [r];
    const measured = measuredOf(regions, true, { sigma: 2.5, maxiters: null }, [
      chunk({ dq_excluded: null, masked: false, notes: [note], bunit: "adu", summed_unit: "adu", elapsed_ms: 4 }),
    ]);
    expect(measured).toEqual({
      regions: [r],
      excludeDq: true,
      sigma: 2.5,
      maxiters: null,
      masked: false,
      dqExcluded: null,
      regionExcluded: 0,
      elapsedMs: 4,
      notes: [note],
      bunit: "adu",
      summedUnit: "adu",
    });
    expect(measured.regions).toBe(regions);
  });
});

describe("statsCalls", () => {
  const outer: RegionShape = { shape: "circle", x: 32, y: 32, r: 20 };
  const hole: RegionShape = { shape: "circle", x: 30, y: 30, r: 5 };
  const ring: RegionShape = { shape: "annulus", x: 32, y: 32, r_inner: 25, r_outer: 30 };

  it("sends every region in one call with no exclusion when nothing is marked exclude", () => {
    expect(statsCalls([region("a", outer), region("bg", ring), region("b", hole, true, "bg")])).toEqual([
      {
        requests: [
          { id: "a", shape: outer, background: null },
          { id: "bg", shape: ring, background: null },
          { id: "b", shape: hole, background: ring },
        ],
        exclude: [],
      },
    ]);
    expect(statsCalls([])).toEqual([]);
  });

  it("measures the include regions with the exclude shapes and each exclude region on its own pixels", () => {
    expect(statsCalls([region("outer", outer), region("inner", hole, false), region("bg", ring)])).toEqual([
      {
        requests: [
          { id: "outer", shape: outer, background: null },
          { id: "bg", shape: ring, background: null },
        ],
        exclude: [hole],
      },
      { requests: [{ id: "inner", shape: hole, background: null }], exclude: [] },
    ]);
  });

  it("keeps the per-call region limit on both groups", () => {
    const many = Array.from({ length: MAX_REGIONS_PER_STATS_CALL + 1 }, (_, i) => region(`r${i}`, outer));
    const calls = statsCalls([...many, region("inner", hole, false)]);
    expect(calls.map((c) => c.requests.length)).toEqual([MAX_REGIONS_PER_STATS_CALL, 1, 1]);
    expect(calls.map((c) => c.exclude.length)).toEqual([1, 1, 0]);
  });
});
