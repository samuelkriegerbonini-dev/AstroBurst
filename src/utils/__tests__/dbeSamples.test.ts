import { describe, it, expect } from "vitest";
import { DEFAULT_DBE_CONFIG, pointSamplesFromDoc, validateDbeParams } from "../dbeSamples";
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
