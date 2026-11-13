import { describe, it, expect } from "vitest";
import { AB_ZERO_POINT_JY, DEFAULT_LIMIT_SIGMA, abFromJy, effectiveError, upperLimit } from "../upperLimits";

const NO_ERROR = { isLimit: false, limitJy: null, limitAb: null, effectiveErrJy: null };

describe("upperLimit", () => {
  it("a flux below k times the effective error is a limit", () => {
    const limit = upperLimit(1.0, 0.5, 3);
    expect(limit.isLimit).toBe(true);
    expect(limit.limitJy).toBe(1.5);
    expect(limit.limitAb).toBeCloseTo(8.459771852360797, 12);
    expect(limit.effectiveErrJy).toBe(0.5);
    expect(upperLimit(2.0, 0.5, 3)).toEqual({ isLimit: false, limitJy: null, limitAb: null, effectiveErrJy: 0.5 });
    expect(upperLimit(1.5, 0.5, 3)).toEqual({ isLimit: false, limitJy: null, limitAb: null, effectiveErrJy: 0.5 });
  });

  it("a negative flux is a limit and the correlated-noise factor scales the error", () => {
    const limit = upperLimit(-0.3, 0.2, 3, 1.5);
    expect(limit.isLimit).toBe(true);
    expect(Math.abs((limit.limitJy as number) - 0.9)).toBeLessThanOrEqual(1e-15);
    expect(Math.abs((limit.effectiveErrJy as number) - 0.3)).toBeLessThanOrEqual(1e-15);
  });

  it("a micro-jansky example in AB", () => {
    const limit = upperLimit(1e-7, 2e-7, 3, 1.647);
    expect(limit.isLimit).toBe(true);
    expect(Math.abs((limit.limitJy as number) - 9.882e-7) / 9.882e-7).toBeLessThanOrEqual(1e-12);
    expect(Math.abs((limit.limitAb as number) - 23.912887876116507)).toBeLessThanOrEqual(1e-9);
  });

  it("no usable error means no limit", () => {
    for (const errJy of [null, undefined, Number.NaN, 0, -1]) {
      expect(upperLimit(1, errJy, 3)).toEqual(NO_ERROR);
    }
  });

  it("a flux that is not a number and a k that is not positive give no limit", () => {
    expect(upperLimit(Number.NaN, 0.5, 3)).toEqual({ isLimit: false, limitJy: null, limitAb: null, effectiveErrJy: 0.5 });
    expect(upperLimit(null, 0.5, 3)).toEqual({ isLimit: false, limitJy: null, limitAb: null, effectiveErrJy: 0.5 });
    expect(upperLimit(1, 0.5, 0).isLimit).toBe(false);
    expect(upperLimit(1, 0.5, Number.NaN).isLimit).toBe(false);
  });

  it("the factor is applied as given and is optional", () => {
    expect(effectiveError(0.5)).toBe(0.5);
    expect(effectiveError(0.5, null)).toBe(0.5);
    expect(effectiveError(0.5, undefined)).toBe(0.5);
    expect(effectiveError(0.5, 0.9)).toBe(0.45);
    expect(effectiveError(0.5, 0)).toBeNull();
    expect(effectiveError(0.5, Number.NaN)).toBeNull();
  });

  it("abFromJy uses the 8.90 zero point", () => {
    expect(Math.abs((abFromJy(3.630780547701014e-6) as number) - 22.5)).toBeLessThanOrEqual(1e-9);
    expect(abFromJy(0)).toBeNull();
    expect(abFromJy(-1)).toBeNull();
    expect(abFromJy(Number.NaN)).toBeNull();
    expect(abFromJy(null)).toBeNull();
    expect(AB_ZERO_POINT_JY === 8.9).toBe(true);
    expect(DEFAULT_LIMIT_SIGMA === 3).toBe(true);
  });
});
