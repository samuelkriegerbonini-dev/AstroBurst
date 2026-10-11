import { describe, expect, it } from "vitest";
import { formatSignificant, formatSignificantFixed } from "../formatSignificant";

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

describe("formatSignificantFixed", () => {
  it("rounds values from 10^4 up to four significant digits, as the backend format_sig4 does", () => {
    expect(formatSignificantFixed(12345.6)).toBe("12350");
    expect(formatSignificantFixed(12345)).toBe("12350");
    expect(formatSignificantFixed(430123.4)).toBe("430100");
    expect(formatSignificantFixed(1234567.8)).toBe("1235000");
    expect(formatSignificantFixed(99999.6)).toBe("100000");
  });

  it("matches formatSignificant below 10^4", () => {
    for (const v of [0.25, 4, 14, 1100, 3221.04, 9999.6, (2 * 16 * 0.25) / Math.PI, 0.012323]) {
      expect(formatSignificantFixed(v)).toBe(formatSignificant(v));
    }
    expect(formatSignificantFixed(2.5464790894703255)).toBe("2.546");
    expect(formatSignificantFixed(9999.6)).toBe("10000");
  });

  it("rounds negatives symmetrically and keeps zero and non-finite values", () => {
    expect(formatSignificantFixed(-12345.6)).toBe("-12350");
    expect(formatSignificantFixed(0)).toBe("0");
    expect(formatSignificantFixed(Number.NaN)).toBe("—");
  });

  it("honours another digit count", () => {
    expect(formatSignificantFixed(12345.6, 3)).toBe("12300");
  });
});
