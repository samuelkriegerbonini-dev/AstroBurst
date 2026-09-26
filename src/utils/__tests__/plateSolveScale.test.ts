import { describe, it, expect } from "vitest";
import {
  DEFAULT_SCALE_HIGH_TEXT,
  DEFAULT_SCALE_LOW_TEXT,
  parseScaleRange,
  scaleHintFromPixelScale,
} from "../plateSolveScale";

describe("scaleHintFromPixelScale", () => {
  it("brackets a NIRCam short-wave scale by 0.8x and 1.25x, rounded outwards", () => {
    expect(scaleHintFromPixelScale(0.031)).toEqual({ low: "0.0248", high: "0.0388" });
  });

  it("keeps exact products without drifting past them", () => {
    expect(scaleHintFromPixelScale(0.04)).toEqual({ low: "0.032", high: "0.05" });
    expect(scaleHintFromPixelScale(0.05)).toEqual({ low: "0.04", high: "0.0625" });
  });

  it("brackets an amateur scale above one arcsecond", () => {
    expect(scaleHintFromPixelScale(1.5)).toEqual({ low: "1.2", high: "1.88" });
  });

  it("gives no hint for a missing, zero, negative or non-finite scale", () => {
    expect(scaleHintFromPixelScale(null)).toBeNull();
    expect(scaleHintFromPixelScale(undefined)).toBeNull();
    expect(scaleHintFromPixelScale(0)).toBeNull();
    expect(scaleHintFromPixelScale(-0.1)).toBeNull();
    expect(scaleHintFromPixelScale(Number.NaN)).toBeNull();
    expect(scaleHintFromPixelScale(Number.POSITIVE_INFINITY)).toBeNull();
  });

  it("always returns a range that parses and contains the header scale", () => {
    for (const scale of [0.0123, 0.031, 0.063, 0.1, 0.396, 2.06, 13.7, 206]) {
      const hint = scaleHintFromPixelScale(scale);
      expect(hint).not.toBeNull();
      const parsed = parseScaleRange(hint!.low, hint!.high);
      expect(parsed.error).toBeNull();
      if (parsed.error === null) {
        expect(parsed.low).toBeLessThanOrEqual(scale * 0.8 + 1e-12);
        expect(parsed.high).toBeGreaterThanOrEqual(scale * 1.25 - 1e-12);
      }
    }
  });
});

describe("parseScaleRange", () => {
  it("accepts the defaults", () => {
    expect(parseScaleRange(DEFAULT_SCALE_LOW_TEXT, DEFAULT_SCALE_HIGH_TEXT)).toEqual({ low: 0.1, high: 10, error: null });
  });

  it("accepts a small scale typed digit by digit once complete", () => {
    expect(parseScaleRange("0.05", " 0.08 ")).toEqual({ low: 0.05, high: 0.08, error: null });
    expect(parseScaleRange("1e-2", "2e-2")).toEqual({ low: 0.01, high: 0.02, error: null });
  });

  it("rejects partial or empty text instead of snapping it to a default", () => {
    for (const partial of ["", " ", ".", "0", "0.", "-", "abc", "0.1x"]) {
      expect(parseScaleRange(partial, "10").error).toBe("Scale low must be a number above 0.");
      expect(parseScaleRange("0.1", partial).error).toBe("Scale high must be a number above 0.");
    }
  });

  it("rejects negative and non-finite values", () => {
    expect(parseScaleRange("-1", "10").error).toBe("Scale low must be a number above 0.");
    expect(parseScaleRange("0.1", "Infinity").error).toBe("Scale high must be a number above 0.");
  });

  it("rejects a low bound that is not below the high bound", () => {
    expect(parseScaleRange("5", "5").error).toBe("Scale low must be below scale high.");
    expect(parseScaleRange("10", "0.1").error).toBe("Scale low must be below scale high.");
  });
});
