import { describe, it, expect } from "vitest";
import { DEFAULT_DBE_CONFIG, dbeRunBlock, pointSamplesFromDoc, validateDbeParams } from "../dbeSamples";
import type { RegionDoc } from "../regionStore";
import type { Region, RegionShape } from "../../shared/types/regions";

function region(id: string, shape: RegionShape, include = true): Region {
  return { id, shape, props: { color: null, width: null, text: null, dash: null, include }, backgroundId: null };
}

function doc(regions: Region[]): RegionDoc {
  return { regions, selectedId: null, version: 0 };
}

describe("pointSamplesFromDoc", () => {
  it("returns only included, finite point regions in document order", () => {
    const d = doc([
      region("c", { shape: "circle", x: 10, y: 10, r: 3 }),
      region("p1", { shape: "point", x: 12.5, y: 40 }),
      region("p2", { shape: "point", x: 99, y: 7 }, false),
      region("p3", { shape: "point", x: Number.NaN, y: 7 }),
      region("b", { shape: "box", x: 1, y: 1, width: 4, height: 4, angle: 0 }),
      region("p4", { shape: "point", x: 3, y: 200 }),
    ]);
    expect(pointSamplesFromDoc(d)).toEqual([
      [12.5, 40],
      [3, 200],
    ]);
  });

  it("returns an empty list for a document without points", () => {
    expect(pointSamplesFromDoc(doc([]))).toEqual([]);
    expect(pointSamplesFromDoc(doc([region("c", { shape: "circle", x: 1, y: 1, r: 1 })]))).toEqual([]);
  });
});

describe("validateDbeParams", () => {
  it("accepts the default configuration with and without an image size", () => {
    expect(validateDbeParams(DEFAULT_DBE_CONFIG)).toEqual({ ok: true, errors: [] });
    expect(validateDbeParams(DEFAULT_DBE_CONFIG, { width: 1024, height: 768 }).ok).toBe(true);
  });

  it("rejects out-of-range numeric parameters", () => {
    const radius = validateDbeParams({ ...DEFAULT_DBE_CONFIG, sampleRadius: 0 });
    expect(radius.ok).toBe(false);
    expect(radius.errors[0]).toMatch(/Sample radius/);

    const fractional = validateDbeParams({ ...DEFAULT_DBE_CONFIG, sampleRadius: 2.5 });
    expect(fractional.ok).toBe(false);

    const tolerance = validateDbeParams({ ...DEFAULT_DBE_CONFIG, tolerance: Number.NaN });
    expect(tolerance.errors.some((e) => e.includes("Tolerance"))).toBe(true);

    const smoothing = validateDbeParams({ ...DEFAULT_DBE_CONFIG, smoothing: 1.5 });
    expect(smoothing.errors.some((e) => e.includes("Smoothing"))).toBe(true);

    const grid = validateDbeParams({ ...DEFAULT_DBE_CONFIG, autoGrid: 1 });
    expect(grid.errors.some((e) => e.includes("Automatic grid"))).toBe(true);

    const maxSamples = validateDbeParams({ ...DEFAULT_DBE_CONFIG, maxSamples: 2 });
    expect(maxSamples.errors.some((e) => e.includes("Maximum samples"))).toBe(true);
  });

  it("requires enough point regions when the automatic grid is off", () => {
    const tooFew = validateDbeParams({ ...DEFAULT_DBE_CONFIG, autoGrid: null, manualSamples: [[1, 1], [2, 2]] });
    expect(tooFew.ok).toBe(false);
    expect(tooFew.errors[0]).toMatch(/at least 4 point regions/);

    const enough = validateDbeParams({
      ...DEFAULT_DBE_CONFIG,
      autoGrid: null,
      manualSamples: [[1, 1], [2, 2], [3, 3], [4, 4]],
    });
    expect(enough).toEqual({ ok: true, errors: [] });
  });

  it("checks point samples and grid density against the image size", () => {
    const size = { width: 100, height: 50 };
    const outside = validateDbeParams({ ...DEFAULT_DBE_CONFIG, manualSamples: [[10, 10], [120, 10], [10, 49]] }, size);
    expect(outside.ok).toBe(false);
    expect(outside.errors).toHaveLength(1);
    expect(outside.errors[0]).toMatch(/Point sample 2 at \(120, 10\) is outside the image/);

    const nonFinite = validateDbeParams({ ...DEFAULT_DBE_CONFIG, manualSamples: [[Number.POSITIVE_INFINITY, 1]] }, size);
    expect(nonFinite.errors[0]).toMatch(/non-finite/);

    const dense = validateDbeParams({ ...DEFAULT_DBE_CONFIG, autoGrid: 20 }, size);
    expect(dense.ok).toBe(false);
    expect(dense.errors[0]).toMatch(/cells under 4 px/);

    const sparse = validateDbeParams({ ...DEFAULT_DBE_CONFIG, autoGrid: 12 }, size);
    expect(sparse.ok).toBe(true);
  });
});

describe("dbeRunBlock", () => {
  const size = { width: 100, height: 50 };
  const two: [number, number][] = [[10, 10], [20, 20]];
  const four: [number, number][] = [[10, 10], [20, 20], [30, 30], [40, 40]];
  const gridOff = { ...DEFAULT_DBE_CONFIG, autoGrid: null };

  it("is null for the defaults, on a file and on the composite", () => {
    expect(dbeRunBlock(DEFAULT_DBE_CONFIG, true, false, null)).toBeNull();
    expect(dbeRunBlock(DEFAULT_DBE_CONFIG, true, false, size)).toBeNull();
    expect(dbeRunBlock(DEFAULT_DBE_CONFIG, false, false, size)).toBeNull();
    expect(dbeRunBlock(DEFAULT_DBE_CONFIG, true, true, null)).toBeNull();
  });

  it("gives the grid message for an automatic grid of 1", () => {
    expect(dbeRunBlock({ ...DEFAULT_DBE_CONFIG, autoGrid: 1 }, true, false, null)).toBe("Automatic grid must be an integer between 2 and 40");
  });

  it("counts the point regions the run would use when the grid is off", () => {
    expect(dbeRunBlock({ ...gridOff, manualSamples: two }, true, false, size)).toBe("4 point regions needed, 2 used");
    expect(dbeRunBlock({ ...gridOff, manualSamples: [] }, true, false, size)).toBe("4 point regions needed, 0 used");
    expect(dbeRunBlock({ ...gridOff, manualSamples: four }, true, false, size)).toBeNull();
  });

  it("says the point regions are not used when their toggle is off", () => {
    expect(dbeRunBlock({ ...gridOff, manualSamples: four }, false, false, size)).toBe(
      "Point regions are not used: turn on 'Use point regions as samples' or Auto Grid",
    );
  });

  it("says the composite never uses point regions", () => {
    const composite = "Point regions are not used on the composite: turn Auto Grid on";
    expect(dbeRunBlock({ ...gridOff, manualSamples: four }, true, true, null)).toBe(composite);
    expect(dbeRunBlock({ ...gridOff, manualSamples: [] }, false, true, null)).toBe(composite);
  });

  it("checks only the point regions the run would send", () => {
    const outside: [number, number][] = [[10, 10], [120, 10]];
    expect(dbeRunBlock({ ...DEFAULT_DBE_CONFIG, manualSamples: outside }, true, false, size)).toBe("Point sample 2 at (120, 10) is outside the image");
    expect(dbeRunBlock({ ...DEFAULT_DBE_CONFIG, manualSamples: outside }, false, false, size)).toBeNull();
    expect(dbeRunBlock({ ...DEFAULT_DBE_CONFIG, manualSamples: outside }, true, true, size)).toBeNull();
  });

  it("gives the first message when several settings fail", () => {
    expect(dbeRunBlock({ ...gridOff, sampleRadius: 0 }, true, false, null)).toBe("Sample radius must be an integer between 1 and 64");
    expect(dbeRunBlock({ ...DEFAULT_DBE_CONFIG, autoGrid: 20 }, true, false, size)).toBe("Automatic grid of 20 leaves cells under 4 px on a 100x50 image");
  });
});
