import { describe, expect, it } from "vitest";
import { MAX_CLIP_ITERS, clipFieldError, parseClipIters, parseClipSigma } from "../useRegionStats";

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

describe("clipFieldError", () => {
  const COMMA = "Use a dot (.) as the decimal separator";
  const ITERS_LISTS = [["1", " 5 ", "100"], ["200", "101", "1e3", "1e20", "7.5", "2.5", "0", "-3", "abc", "Infinity"], ["", "   "]];
  const SIGMA_LISTS = [["3", " 2.5 "], ["1e39", "1e-46", "0", "-1", "Infinity", "NaN", "abc"], ["  "]];

  it("clipFieldError words every entry the parsers refuse and nothing else", () => {
    for (const text of ["", "  ", "3", " 2.5 "]) expect(clipFieldError("sigma", text)).toBeNull();
    expect(clipFieldError("sigma", "2,5")).toBe(COMMA);
    for (const text of ["abc", "NaN", "Infinity"]) expect(clipFieldError("sigma", text)).toBe("Not a number");
    for (const text of ["0", "-1", "1e39", "1e-46"]) expect(clipFieldError("sigma", text)).toBe("Use a number above 0 and below 3.4e38");

    for (const text of ["", "1", " 5 ", "100"]) expect(clipFieldError("iters", text)).toBeNull();
    for (const text of ["7.5", "2.5"]) expect(clipFieldError("iters", text)).toBe("Use a whole number");
    for (const text of ["200", "101", "0", "-3", "1e3", "1e20"]) expect(clipFieldError("iters", text)).toBe("Use a value from 1 to 100");
    expect(clipFieldError("iters", "2,5")).toBe(COMMA);
    expect(clipFieldError("iters", "abc")).toBe("Not a number");

    for (const text of ITERS_LISTS.flat()) {
      expect(clipFieldError("iters", text) === null, text).toBe(text.trim() === "" || parseClipIters(text) !== null);
    }
    for (const text of SIGMA_LISTS.flat()) {
      expect(clipFieldError("sigma", text) === null, text).toBe(text.trim() === "" || parseClipSigma(text) !== null);
    }
  });
});
