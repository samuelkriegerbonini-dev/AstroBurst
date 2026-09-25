import { describe, expect, it } from "vitest";
import { MAX_CLIP_ITERS, parseClipIters, parseClipSigma } from "../useRegionStats";

describe("parseClipIters", () => {
  it("mirrors the backend iteration limit", () => {
    expect(MAX_CLIP_ITERS).toBe(100);
  });

  it("accepts whole numbers from 1 to the backend limit", () => {
    expect(parseClipIters("1")).toBe(1);
    expect(parseClipIters(" 5 ")).toBe(5);
    expect(parseClipIters("100")).toBe(100);
  });

  it("returns null for every value region_stats_cmd would refuse so the default is sent", () => {
    for (const text of ["200", "101", "1e3", "1e20", "7.5", "2.5", "0", "-3", "abc", "Infinity"]) {
      expect(parseClipIters(text)).toBeNull();
    }
  });

  it("treats a blank field as the default", () => {
    expect(parseClipIters("")).toBeNull();
    expect(parseClipIters("   ")).toBeNull();
  });
});

describe("parseClipSigma", () => {
  it("accepts finite positive thresholds", () => {
    expect(parseClipSigma("3")).toBe(3);
    expect(parseClipSigma(" 2.5 ")).toBe(2.5);
  });

  it("returns null for thresholds that are not finite and positive once converted to f32", () => {
    for (const text of ["1e39", "1e-46", "0", "-1", "Infinity", "NaN", "abc"]) {
      expect(parseClipSigma(text)).toBeNull();
    }
  });

  it("treats a blank field as the default", () => {
    expect(parseClipSigma("  ")).toBeNull();
  });
});
