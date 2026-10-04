import { describe, it, expect } from "vitest";
import { parseDecimalText } from "../decimalText";

describe("parseDecimalText", () => {
  it("rejects a decimal comma instead of reading the digits before it", () => {
    expect(parseDecimalText("0,75")).toBeNull();
    expect(parseDecimalText("190,029137")).toBeNull();
    expect(parseDecimalText("-11,6")).toBeNull();
  });

  it("reads dot decimals and exponents, ignoring surrounding spaces", () => {
    expect(parseDecimalText(" 1.5 ")).toBe(1.5);
    expect(parseDecimalText("1e3")).toBe(1000);
    expect(parseDecimalText("-11.633336")).toBe(-11.633336);
    expect(parseDecimalText(".5")).toBe(0.5);
  });

  it("gives no value for blank text", () => {
    expect(parseDecimalText("")).toBeNull();
    expect(parseDecimalText("   ")).toBeNull();
  });

  it("rejects unit suffixes and non-finite values", () => {
    expect(parseDecimalText("1.5px")).toBeNull();
    expect(parseDecimalText("abc")).toBeNull();
    expect(parseDecimalText("Infinity")).toBeNull();
    expect(parseDecimalText("1e999")).toBeNull();
  });
});
