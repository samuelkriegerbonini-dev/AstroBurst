import { describe, expect, it } from "vitest";
import { formatSignificant } from "../formatSignificant";

describe("formatSignificant", () => {
  it("keeps four significant digits for small-unit data", () => {
    expect(formatSignificant(0.012323)).toBe("0.01232");
    expect(formatSignificant(8.3884e-4)).toBe("0.0008388");
  });

  it("never prints a non-zero value as zero", () => {
    for (const v of [1e-3, 4e-5, 2.5e-8, -6e-4]) {
      expect(formatSignificant(v)).not.toMatch(/^-?0(\.0*)?$/);
    }
  });

  it("keeps four significant digits between 1 and 1000 and trims trailing zeros", () => {
    expect(formatSignificant(93.2345)).toBe("93.23");
    expect(formatSignificant(10.2873)).toBe("10.29");
    expect(formatSignificant(12.5)).toBe("12.5");
  });

  it("prints values from 1000 up as whole numbers without an exponent", () => {
    expect(formatSignificant(1024.53)).toBe("1025");
    expect(formatSignificant(9999.6)).toBe("10000");
    expect(formatSignificant(430123.4)).toBe("430123");
    expect(formatSignificant(1234567.8)).toBe("1234568");
  });

  it("prints zero, negatives and non-finite values", () => {
    expect(formatSignificant(0)).toBe("0");
    expect(formatSignificant(-0.0123)).toBe("-0.0123");
    expect(formatSignificant(-2048.4)).toBe("-2048");
    expect(formatSignificant(Number.NaN)).toBe("—");
  });
});
