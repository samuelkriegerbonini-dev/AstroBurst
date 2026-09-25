import { describe, it, expect } from "vitest";
import { formatPositionAngle, formatSeparation, normalizePositionAngle, pixelLength } from "../skyMeasure";

describe("formatSeparation", () => {
  it("switches from arcseconds to arcminutes at 60 arcsec and to degrees at 60 arcmin", () => {
    expect(formatSeparation(12.345)).toBe('12.35"');
    expect(formatSeparation(59.999)).toBe('60.00"');
    expect(formatSeparation(60)).toBe("1.00'");
    expect(formatSeparation(73.8)).toBe("1.23'");
    expect(formatSeparation(3599)).toBe("59.98'");
    expect(formatSeparation(3600)).toBe("1.000 deg");
    expect(formatSeparation(4442.4)).toBe("1.234 deg");
  });

  it("shows a placeholder for non-finite input", () => {
    expect(formatSeparation(Number.NaN)).toBe("--");
    expect(formatSeparation(Number.POSITIVE_INFINITY)).toBe("--");
  });
});

describe("formatPositionAngle", () => {
  it("prints east of north with one decimal and wraps into 0..360", () => {
    expect(formatPositionAngle(47.24)).toBe("47.2 deg E of N");
    expect(formatPositionAngle(-90)).toBe("270.0 deg E of N");
    expect(formatPositionAngle(359.99)).toBe("0.0 deg E of N");
    expect(normalizePositionAngle(720)).toBe(0);
    expect(formatPositionAngle(Number.NaN)).toBe("--");
  });
});

describe("pixelLength", () => {
  it("is the euclidean length of the line", () => {
    expect(pixelLength(0, 0, 3, 4)).toBe(5);
    expect(pixelLength(2, 2, 2, 2)).toBe(0);
  });
});
