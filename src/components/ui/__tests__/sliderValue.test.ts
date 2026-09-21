import { describe, it, expect } from "vitest";
import { snapToStep, resolveTypedValue } from "../sliderValue";

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
