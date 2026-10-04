import { describe, it, expect } from "vitest";
import { snapToStep, resolveTypedValue, parseTypedText, typedTextRejected } from "../sliderValue";

describe("snapToStep", () => {
  it("snaps a fractional entry to an integer grid", () => {
    expect(snapToStep(3.5, 1, 10, 1)).toBe(4);
    expect(snapToStep(3.4, 1, 10, 1)).toBe(3);
    expect(Number.isInteger(snapToStep(7.9, 1, 10, 1))).toBe(true);
  });

  it("anchors the grid at min, matching the range input", () => {
    expect(snapToStep(4, 1, 9, 2)).toBe(5);
    expect(snapToStep(3.9, 1, 9, 2)).toBe(3);
  });

  it("clamps after snapping", () => {
    expect(snapToStep(42, 1, 10, 1)).toBe(10);
    expect(snapToStep(-7, 1, 10, 1)).toBe(1);
    expect(snapToStep(10.4, 1, 10, 1)).toBe(10);
  });

  it("keeps fractional steps free of binary-float noise", () => {
    expect(snapToStep(0.30000000000000004, 0, 1, 0.1)).toBe(0.3);
    expect(snapToStep(1.17, 0, 5, 0.05)).toBe(1.15);
    expect(snapToStep(2.34, 0.5, 10, 0.01)).toBe(2.34);
  });

  it("passes the value through when there is no usable step", () => {
    expect(snapToStep(3.5, 1, 10, 0)).toBe(3.5);
    expect(snapToStep(3.5, 1, 10, NaN)).toBe(3.5);
  });

  it("does not invent a value for non-numeric input", () => {
    expect(snapToStep(NaN, 1, 10, 1)).toBeNaN();
  });
});

describe("resolveTypedValue", () => {
  const midtone = { min: 0.0001, max: 1, step: 0.0001 };

  it("keeps an off-grid entry on a log slider, whose drag path has no step grid", () => {
    expect(resolveTypedValue(0.000123, midtone.min, midtone.max, midtone.step, true)).toBe(0.000123);
    expect(resolveTypedValue(0.000456, midtone.min, midtone.max, midtone.step, true)).toBe(0.000456);
    expect(resolveTypedValue(0.00035, midtone.min, midtone.max, midtone.step, true)).toBe(0.00035);
  });

  it("still clamps a log entry to the range", () => {
    expect(resolveTypedValue(0, midtone.min, midtone.max, midtone.step, true)).toBe(0.0001);
    expect(resolveTypedValue(5, midtone.min, midtone.max, midtone.step, true)).toBe(1);
  });

  it("snaps on a linear slider, where the range input enforces the same grid", () => {
    expect(resolveTypedValue(3.5, 1, 10, 1, false)).toBe(4);
    expect(resolveTypedValue(0.000123, midtone.min, midtone.max, midtone.step, false)).toBe(0.0001);
  });

  it("does not invent a value for non-numeric input on either scale", () => {
    expect(resolveTypedValue(NaN, 1, 10, 1, true)).toBeNaN();
    expect(resolveTypedValue(NaN, 1, 10, 1, false)).toBeNaN();
  });
});

describe("parseTypedText", () => {
  it("rejects a decimal comma instead of reading the digits before it", () => {
    expect(parseTypedText("0,75")).toBeNull();
    expect(parseTypedText("1,5")).toBeNull();
    expect(parseTypedText("2,5px")).toBeNull();
  });

  it("keeps reading the unit-suffixed text the value button shows", () => {
    expect(parseTypedText("4px")).toBe(4);
    expect(parseTypedText("1.5x")).toBe(1.5);
    expect(parseTypedText("2.5σ")).toBe(2.5);
    expect(parseTypedText("50%")).toBe(50);
    expect(parseTypedText(" 0.75 ")).toBe(0.75);
    expect(parseTypedText("1e-3")).toBe(0.001);
  });

  it("gives no value for text without a number", () => {
    expect(parseTypedText("")).toBeNull();
    expect(parseTypedText("abc")).toBeNull();
  });
});

describe("typedTextRejected", () => {
  it("marks a decimal comma, so Enter keeps the box open instead of applying the digits before it", () => {
    expect(typedTextRejected("0,75")).toBe(true);
    expect(typedTextRejected("1,5px")).toBe(true);
    expect(typedTextRejected(",")).toBe(true);
  });

  it("does not mark the valid prefix of a number while it is being typed", () => {
    for (const prefix of ["-", "+", ".", "-.", "-0.", "1e"]) {
      expect(typedTextRejected(prefix), `prefix "${prefix}"`).toBe(false);
    }
  });

  it("does not mark text without a comma, which Enter still closes without applying as before", () => {
    expect(typedTextRejected("abc")).toBe(false);
    expect(typedTextRejected("")).toBe(false);
    expect(typedTextRejected("4px")).toBe(false);
    expect(parseTypedText("abc")).toBeNull();
  });
});
