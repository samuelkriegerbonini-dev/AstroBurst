import { describe, it, expect } from "vitest";
import {
  decimalDraftFor,
  decimalDraftText,
  decimalEditValue,
  decimalFieldState,
  firstFieldError,
  parseDecimalText,
} from "../decimalText";

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

describe("decimalFieldState", () => {
  const COMMA = "Use a dot (.) as the decimal separator";

  it("refuses a decimal comma with an actionable error and no value, so 2,5 never becomes 25", () => {
    expect(decimalFieldState("2,5")).toEqual({ value: null, error: COMMA });
    expect(decimalFieldState(" 0,75 ", { min: 0, max: 1 })).toEqual({ value: null, error: COMMA });
    expect(decimalFieldState("2,5", { integer: true })).toEqual({ value: null, error: COMMA });
  });

  it("reads a dot decimal, trimming surrounding spaces", () => {
    expect(decimalFieldState("2.5")).toEqual({ value: 2.5, error: null });
    expect(decimalFieldState(" -11.63 ")).toEqual({ value: -11.63, error: null });
    expect(decimalFieldState("1e3")).toEqual({ value: 1000, error: null });
  });

  it("treats blank text as unset when the field is optional and as an error otherwise", () => {
    expect(decimalFieldState("", { optional: true })).toEqual({ value: null, error: null });
    expect(decimalFieldState("   ", { optional: true })).toEqual({ value: null, error: null });
    expect(decimalFieldState("")).toEqual({ value: null, error: "Enter a number" });
  });

  it("rejects text that is not a finite number", () => {
    expect(decimalFieldState("abc")).toEqual({ value: null, error: "Not a number" });
    expect(decimalFieldState("1.5px", { optional: true })).toEqual({ value: null, error: "Not a number" });
    expect(decimalFieldState("Infinity")).toEqual({ value: null, error: "Not a number" });
  });

  it("applies min and max inclusively", () => {
    expect(decimalFieldState("1", { min: 1 })).toEqual({ value: 1, error: null });
    expect(decimalFieldState("0.5", { min: 1 })).toEqual({ value: null, error: "Use a value of at least 1" });
    expect(decimalFieldState("100", { max: 100 })).toEqual({ value: 100, error: null });
    expect(decimalFieldState("100.5", { max: 100 })).toEqual({ value: null, error: "Use a value of at most 100" });
    expect(decimalFieldState("-0.1", { min: 0, max: 100 })).toEqual({ value: null, error: "Use a value from 0 to 100" });
    expect(decimalFieldState("0.0001", { min: 0.0001 })).toEqual({ value: 0.0001, error: null });
  });

  it("accepts only whole numbers in integer mode", () => {
    expect(decimalFieldState("3", { integer: true })).toEqual({ value: 3, error: null });
    expect(decimalFieldState("3.0", { integer: true })).toEqual({ value: 3, error: null });
    expect(decimalFieldState("3.5", { integer: true })).toEqual({ value: null, error: "Use a whole number" });
    expect(decimalFieldState("12", { integer: true, min: 0, max: 9 })).toEqual({ value: null, error: "Use a value from 0 to 9" });
  });
});

describe("decimal drafts bound to a number", () => {
  it("formats a number for the field and leaves an unset value blank", () => {
    expect(decimalDraftText(0.1)).toBe("0.1");
    expect(decimalDraftText(-300)).toBe("-300");
    expect(decimalDraftText(null)).toBe("");
  });

  it("keeps a draft that already reads as the value, so typing 1e3 or 2. is not rewritten", () => {
    expect(decimalDraftFor("1e3", 1000)).toBe("1e3");
    expect(decimalDraftFor("2.", 2)).toBe("2.");
    expect(decimalDraftFor("", null, { optional: true })).toBe("");
  });

  it("replaces a draft with the value when the value changed elsewhere", () => {
    expect(decimalDraftFor("5", 12)).toBe("12");
    expect(decimalDraftFor("2,5", 0.1)).toBe("0.1");
    expect(decimalDraftFor("7", null, { optional: true })).toBe("");
    expect(decimalDraftFor("150", 100, { max: 100 })).toBe("100");
  });

  it("commits only a valid value that differs from the current one", () => {
    expect(decimalEditValue("2.5", 1)).toBe(2.5);
    expect(decimalEditValue("2.5", 2.5)).toBeUndefined();
    expect(decimalEditValue("", 4, { optional: true })).toBeNull();
    expect(decimalEditValue("", null, { optional: true })).toBeUndefined();
  });

  it("never commits an invalid draft, so 2,5 leaves the value alone instead of becoming 25", () => {
    expect(decimalEditValue("2,5", 1)).toBeUndefined();
    expect(decimalEditValue("", 1)).toBeUndefined();
    expect(decimalEditValue("-", 1)).toBeUndefined();
    expect(decimalEditValue("0", 1, { min: 0.01 })).toBeUndefined();
    expect(decimalEditValue("2.5", 1, { integer: true })).toBeUndefined();
  });
});

describe("firstFieldError", () => {
  it("names the first invalid field with its error", () => {
    expect(
      firstFieldError([
        ["r_ap px", decimalFieldState("3", { optional: true })],
        ["sky in px", decimalFieldState("2,5", { optional: true })],
        ["gain e-/ADU", decimalFieldState("abc", { optional: true })],
      ]),
    ).toBe("sky in px: Use a dot (.) as the decimal separator");
  });

  it("gives null when every field is valid or blank-optional", () => {
    expect(firstFieldError([])).toBeNull();
    expect(
      firstFieldError([
        ["Radius", decimalFieldState("", { optional: true })],
        ["G limit", decimalFieldState("18")],
      ]),
    ).toBeNull();
  });
});
