import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  MISSING_VALUE,
  PLUS_MINUS,
  formatFluxDensityJy,
  formatWithUncertainty,
  type UncertaintyFormatOptions,
} from "../formatUncertainty";

const FUZZ_FIXTURE = fileURLToPath(new URL("../../../src-tauri/tests/fixtures/format_uncertainty/oracle_fuzz.jsonl", import.meta.url));
const FUZZ_SHA256 = "de08fe2ddc66202a27028bc3606be70599f93f11358d10723edac78cae89c965";

type UncertaintyFuzzRow = readonly ["u", number, number | null, 1 | 2, string];
type FluxDensityFuzzRow = readonly ["j", number, number | null, string];
type FuzzRow = UncertaintyFuzzRow | FluxDensityFuzzRow;

const WITH_UNCERTAINTY_CASES: readonly (readonly [number | null, number | null, UncertaintyFormatOptions | undefined, string])[] = [
  [1.23456, 0.0123, undefined, "1.235 ± 0.012"],
  [1.23456, 0.0123, { unit: "mag" }, "1.235 ± 0.012 mag"],
  [1.23456, 0.0123, { errDigits: 1 }, "1.23 ± 0.01"],
  [12.3456, 0.5, undefined, "12.35 ± 0.50"],
  [1234.56, 12.3, { unit: "DN" }, "1235 ± 12 DN"],
  [56789, 1234, undefined, "56800 ± 1200"],
  [56789, 1234, { errDigits: 1 }, "57000 ± 1000"],
  [12, 1234, undefined, "0 ± 1200"],
  [0.5, 120, undefined, "0 ± 120"],
  [-56789, 1234, undefined, "-56800 ± 1200"],
  [-12, 1234, undefined, "0 ± 1200"],
  [1.2341e-17, 1.2e-19, undefined, "(1.234 ± 0.012)e-17"],
  [1.2341e-17, 1.2e-19, { unit: "erg s-1 cm-2" }, "(1.234 ± 0.012)e-17 erg s-1 cm-2"],
  [-1.2341e-17, 1.2e-19, undefined, "(-1.234 ± 0.012)e-17"],
  [2e-19, 1.5e-17, undefined, "(0.0 ± 1.5)e-17"],
  [1234567, 2345, undefined, "(1.2346 ± 0.0023)e6"],
  [123456.7, 0.5, undefined, "(1.2345670 ± 0.0000050)e5"],
  [99999, 0.6, undefined, "99999.00 ± 0.60"],
  [0.0996, 0.0996, undefined, "0.10 ± 0.10"],
  [5, 0.0996, undefined, "5.00 ± 0.10"],
  [5, 0.0996, { errDigits: 1 }, "5.0 ± 0.1"],
  [5, 0.96, { errDigits: 1 }, "5 ± 1"],
  [-0.0004, 0.01, undefined, "0.000 ± 0.010"],
  [-3.14159, 0.002, undefined, "-3.1416 ± 0.0020"],
  [0, 0.05, undefined, "0.000 ± 0.050"],
  [0.125, 0.01, { errDigits: 1 }, "0.13 ± 0.01"],
  [0.000123, 4.5e-5, undefined, "(1.23 ± 0.45)e-4"],
  [0.00123, 0.00045, undefined, "0.00123 ± 0.00045"],
  [20.5, 0.25, undefined, "20.50 ± 0.25"],
  [0.001, 0.0001, undefined, "0.00100 ± 0.00010"],
  [99996, 12, undefined, "99996 ± 12"],
  [100000, 12, undefined, "(1.00000 ± 0.00012)e5"],
  [99999.996, 0.6, undefined, "100000.00 ± 0.60"],
  [999999.9999, 1200, undefined, "(10.000 ± 0.012)e5"],
  [30000, 960000, { errDigits: 1 }, "(0 ± 1)e6"],
  [30000, 996000, undefined, "(0.0 ± 1.0)e6"],
  [24.51234, 0.2512, undefined, "24.51 ± 0.25"],
  [1234.5678, 12.3, undefined, "1235 ± 12"],
  [10.25, 0.1414, undefined, "10.25 ± 0.14"],
  [3, 0.3, undefined, "3.00 ± 0.30"],
  [1, 1e-150, undefined, "1.000 ± 1.0e-150"],
  [1, 5e-324, undefined, "1.000 ± 4.9e-324"],
  [123456.7, 1.5e-8, { unit: "Jy" }, "1.235e5 ± 1.5e-8 Jy"],
  [1, 1e-12, undefined, "1.0000000000000 ± 0.0000000000010"],
  [1, 1e-13, undefined, "1.000 ± 1.0e-13"],
  [5e-324, 5e-324, undefined, "(4.9 ± 4.9)e-324"],
  [1.7e308, 1.2e306, undefined, "(1.700 ± 0.012)e308"],
  [3, null, undefined, "3.000"],
  [12.3456, null, undefined, "12.35"],
  [12.3, 0, undefined, "12.30"],
  [0.0012345, Number.NaN, undefined, "0.001234"],
  [1234.5, null, undefined, "1235"],
  [12345.6, -1, undefined, "12346"],
  [123456.7, null, { unit: "Jy" }, "1.235e5 Jy"],
  [1.2e-17, null, undefined, "1.200e-17"],
  [0, null, undefined, "0"],
  [-0.00012345, null, undefined, "-1.234e-4"],
  [999.96, null, undefined, "1000"],
  [-2048.5, Number.POSITIVE_INFINITY, undefined, "-2049"],
  [0.00099996, null, undefined, "1.000e-3"],
  [99999.5, null, undefined, "100000"],
  [-0, null, undefined, "0"],
  [null, 0.1, undefined, "--"],
  [Number.NaN, 0.1, { unit: "Jy" }, "--"],
  [Number.POSITIVE_INFINITY, 0.1, undefined, "--"],
  [Number.NEGATIVE_INFINITY, null, { unit: "Jy" }, "--"],
];

const FLUX_DENSITY_CASES: readonly (readonly [number | null, number | null, string])[] = [
  [1.5, 0.02, "1.500 ± 0.020 Jy"],
  [0.0123, 0.00099, "12.30 ± 0.99 mJy"],
  [1.23e-5, 9.9e-7, "12.30 ± 0.99 uJy"],
  [1.23e-5, 9.9e-10, "12.30000 ± 0.00099 uJy"],
  [4.2e-9, 1.1e-9, "4.2 ± 1.1 nJy"],
  [3.3e-13, 2e-14, "(3.30 ± 0.20)e-4 nJy"],
  [1500, 20, "1500 ± 20 Jy"],
  [2500000, 12000, "(2.500 ± 0.012)e6 Jy"],
  [0, 2e-6, "0.0 ± 2.0 uJy"],
  [-3e-6, 2e-6, "-3.0 ± 2.0 uJy"],
  [0.00123, null, "1.230 mJy"],
  [null, 0.001, "--"],
  [0.001, 1e-5, "1.000 ± 0.010 mJy"],
  [0.000999, 1e-5, "999 ± 10 uJy"],
  [1, 0.01, "1.000 ± 0.010 Jy"],
  [0, null, "0 Jy"],
  [1e-6, 1e-8, "1.000 ± 0.010 uJy"],
  [9.99e-7, 1e-8, "999 ± 10 nJy"],
  [-0.0123, 0.00099, "-12.30 ± 0.99 mJy"],
  [Number.NaN, null, "--"],
  [2e-5, Number.NaN, "20.00 uJy"],
  [0, 0, "0 Jy"],
];

describe("formatWithUncertainty and formatFluxDensityJy", () => {
  it("formats every pinned value and error as the oracle does", () => {
    const differing = WITH_UNCERTAINTY_CASES.flatMap(([value, err, options, expected]) => {
      const got = formatWithUncertainty(value, err, options);
      return got === expected ? [] : [[value, err, options, expected, got]];
    });
    expect(differing).toEqual([]);
  });

  it("formats every pinned flux density with one prefix for the value and its error", () => {
    const differing = FLUX_DENSITY_CASES.flatMap(([valueJy, errJy, expected]) => {
      const got = formatFluxDensityJy(valueJy, errJy);
      return got === expected ? [] : [[valueJy, errJy, expected, got]];
    });
    expect(differing).toEqual([]);
  });

  it("agrees with the oracle on the 600 random pairs of the committed fixture", () => {
    const text = readFileSync(FUZZ_FIXTURE, "utf8");
    expect(createHash("sha256").update(text.replace(/\r\n/g, "\n"), "utf8").digest("hex")).toBe(FUZZ_SHA256);
    const rows = text
      .split(/\r?\n/)
      .filter((line) => line.length > 0)
      .map((line) => JSON.parse(line) as FuzzRow);
    expect(rows).toHaveLength(600);
    expect(rows.filter((row) => row[0] === "u")).toHaveLength(300);
    expect(rows.filter((row) => row[0] === "j")).toHaveLength(300);
    const differing = rows.flatMap((row) => {
      const got =
        row[0] === "u" ? formatWithUncertainty(row[1], row[2], { errDigits: row[3] }) : formatFluxDensityJy(row[1], row[2]);
      return got === row[row.length - 1] ? [] : [[...row, got]];
    });
    expect(differing).toEqual([]);
  });

  it("prints -- without a value, whatever the error and the unit", () => {
    expect(MISSING_VALUE).toBe("--");
    expect(formatWithUncertainty(null, 0.1)).toBe(MISSING_VALUE);
    expect(formatWithUncertainty(undefined, 0.1)).toBe(MISSING_VALUE);
    expect(formatWithUncertainty(Number.NaN, 0.1, { unit: "Jy" })).toBe(MISSING_VALUE);
    expect(formatWithUncertainty(Number.POSITIVE_INFINITY, 0.1)).toBe(MISSING_VALUE);
    expect(formatWithUncertainty(Number.NEGATIVE_INFINITY, null, { unit: "Jy" })).toBe(MISSING_VALUE);
    expect(formatFluxDensityJy(null, 0.001)).toBe(MISSING_VALUE);
    expect(formatFluxDensityJy(undefined, undefined)).toBe(MISSING_VALUE);
    expect(formatFluxDensityJy(Number.NaN, null)).toBe(MISSING_VALUE);
  });

  it("prints four significant digits when the error is missing, zero, negative or not finite", () => {
    expect(formatWithUncertainty(12.3456, null)).toBe("12.35");
    expect(formatWithUncertainty(12.3456, undefined)).toBe("12.35");
    expect(formatWithUncertainty(12.3, 0)).toBe("12.30");
    expect(formatWithUncertainty(12345.6, -1)).toBe("12346");
    expect(formatWithUncertainty(0.0012345, Number.NaN)).toBe("0.001234");
    expect(formatWithUncertainty(-2048.5, Number.POSITIVE_INFINITY)).toBe("-2049");
    expect(formatWithUncertainty(1.2e-17, null)).toBe("1.200e-17");
    expect(formatWithUncertainty(123456.7, null, { unit: "Jy" })).toBe("1.235e5 Jy");
    expect(formatWithUncertainty(0, null)).toBe("0");
    expect(formatWithUncertainty(-0, null)).toBe("0");
  });

  it("rounds the value to the decimal place of the error", () => {
    expect(formatWithUncertainty(1.23456, 0.0123)).toBe("1.235 ± 0.012");
    expect(formatWithUncertainty(24.51234, 0.2512)).toBe("24.51 ± 0.25");
    expect(formatWithUncertainty(1234.5678, 12.3)).toBe("1235 ± 12");
    expect(formatWithUncertainty(3, 0.3)).toBe("3.00 ± 0.30");
    expect(formatWithUncertainty(56789, 1234)).toBe("56800 ± 1200");
    expect(formatWithUncertainty(-56789, 1234)).toBe("-56800 ± 1200");
  });

  it("keeps one or two digits of the error", () => {
    expect(formatWithUncertainty(1.23456, 0.0123, { errDigits: 1 })).toBe("1.23 ± 0.01");
    expect(formatWithUncertainty(1.23456, 0.0123, { errDigits: 2 })).toBe(formatWithUncertainty(1.23456, 0.0123));
    expect(formatWithUncertainty(5, 0.96, { errDigits: 1 })).toBe("5 ± 1");
    expect(formatWithUncertainty(56789, 1234, { errDigits: 1 })).toBe("57000 ± 1000");
  });

  it("shares one exponent outside 1e-3 to 1e5", () => {
    expect(formatWithUncertainty(1.2341e-17, 1.2e-19)).toBe("(1.234 ± 0.012)e-17");
    expect(formatWithUncertainty(2e-19, 1.5e-17)).toBe("(0.0 ± 1.5)e-17");
    expect(formatWithUncertainty(1234567, 2345)).toBe("(1.2346 ± 0.0023)e6");
    expect(formatWithUncertainty(0.000123, 4.5e-5)).toBe("(1.23 ± 0.45)e-4");
    expect(formatWithUncertainty(0.00123, 0.00045)).toBe("0.00123 ± 0.00045");
    expect(formatWithUncertainty(99996, 12)).toBe("99996 ± 12");
    expect(formatWithUncertainty(100000, 12)).toBe("(1.00000 ± 0.00012)e5");
  });

  it("writes value and error apart when they cannot share a decimal place", () => {
    expect(formatWithUncertainty(1, 1e-13)).toBe("1.000 ± 1.0e-13");
    expect(formatWithUncertainty(1, 1e-12)).toBe("1.0000000000000 ± 0.0000000000010");
    expect(formatWithUncertainty(123456.7, 1.5e-8, { unit: "Jy" })).toBe("1.235e5 ± 1.5e-8 Jy");
  });

  it("drops the sign of a value that rounds to zero", () => {
    expect(formatWithUncertainty(-0.0004, 0.01)).toBe("0.000 ± 0.010");
    expect(formatWithUncertainty(-12, 1234)).toBe("0 ± 1200");
    expect(formatWithUncertainty(-3.14159, 0.002)).toBe("-3.1416 ± 0.0020");
  });

  it("puts the unit after one space and adds nothing for an empty unit", () => {
    expect(formatWithUncertainty(1.23456, 0.0123, { unit: "mag" })).toBe("1.235 ± 0.012 mag");
    expect(formatWithUncertainty(1.2341e-17, 1.2e-19, { unit: "erg s-1 cm-2" })).toBe("(1.234 ± 0.012)e-17 erg s-1 cm-2");
    expect(formatWithUncertainty(1.23456, 0.0123, { unit: "" })).toBe("1.235 ± 0.012");
  });

  it("picks the prefix of a flux density from the value, and from the error when the value is zero", () => {
    expect(formatFluxDensityJy(1.5, 0.02)).toBe("1.500 ± 0.020 Jy");
    expect(formatFluxDensityJy(0.0123, 0.00099)).toBe("12.30 ± 0.99 mJy");
    expect(formatFluxDensityJy(1.23e-5, 9.9e-7)).toBe("12.30 ± 0.99 uJy");
    expect(formatFluxDensityJy(4.2e-9, 1.1e-9)).toBe("4.2 ± 1.1 nJy");
    expect(formatFluxDensityJy(0.001, 1e-5)).toBe("1.000 ± 0.010 mJy");
    expect(formatFluxDensityJy(0.000999, 1e-5)).toBe("999 ± 10 uJy");
    expect(formatFluxDensityJy(0, 2e-6)).toBe("0.0 ± 2.0 uJy");
    expect(formatFluxDensityJy(0, null)).toBe("0 Jy");
    expect(formatFluxDensityJy(0.00123, null)).toBe("1.230 mJy");
    expect(formatFluxDensityJy(2500000, 12000)).toBe("(2.500 ± 0.012)e6 Jy");
  });

  it("exports the plus-minus sign as one character", () => {
    expect(PLUS_MINUS.length).toBe(1);
    expect(PLUS_MINUS.charCodeAt(0)).toBe(0xb1);
    expect(formatWithUncertainty(1, 0.1)).toBe(`1.00 ${PLUS_MINUS} 0.10`);
  });
});
