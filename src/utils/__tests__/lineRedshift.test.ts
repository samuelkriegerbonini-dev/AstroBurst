import { describe, it, expect } from "vitest";
import { combineRedshifts, redshiftFromLines } from "../lineRedshift";
import type { LineRedshiftInput } from "../lineRedshift";

const THREE_LINES: LineRedshiftInput[] = [
  { restUm: 0.656461, centroidUm: 0.98475, centroidErrUm: 3e-5 },
  { restUm: 0.500824, centroidUm: 0.75127, centroidErrUm: 2e-5 },
  { restUm: 0.486268, centroidUm: 0.72943, centroidErrUm: 5e-5 },
];

function relativeDifference(actual: number, expected: number): number {
  return Math.abs(actual - expected) / Math.abs(expected);
}

describe("redshift from several lines", () => {
  it("combines three lines into the error-weighted mean with its chi2", () => {
    const combined = redshiftFromLines(THREE_LINES);
    expect(combined).not.toBeNull();
    if (!combined) return;
    expect(Math.abs(combined.z - 0.5000755423957206)).toBeLessThan(1e-12);
    expect(relativeDifference(combined.zErrFormal, 2.8861889458078432e-5)).toBeLessThan(1e-9);
    expect(relativeDifference(combined.chi2, 0.155446694624214)).toBeLessThan(1e-6);
    expect(combined.dof).toBe(2);
    expect(combined.n).toBe(3);
    expect(combined.zErr).toBe(combined.zErrFormal);
  });

  it("scales the error by the scatter when the lines disagree", () => {
    const combined = combineRedshifts([
      { z: 0.5, zErr: 2e-4 },
      { z: 0.501, zErr: 2e-4 },
      { z: 0.5004, zErr: 2e-4 },
    ]);
    expect(combined).not.toBeNull();
    if (!combined) return;
    expect(Math.abs(combined.z - 0.5004666666666666)).toBeLessThan(1e-12);
    expect(relativeDifference(combined.zErrFormal, 1.1547005383792516e-4)).toBeLessThan(1e-9);
    expect(relativeDifference(combined.chi2, 12.666666666666838)).toBeLessThan(1e-9);
    expect(combined.dof).toBe(2);
    expect(relativeDifference(combined.zErr, 2.9059326290271356e-4)).toBeLessThan(1e-9);
    expect(
      relativeDifference(combined.zErr, combined.zErrFormal * Math.sqrt(combined.chi2 / 2)),
    ).toBeLessThan(1e-12);
  });

  it("returns the single line unchanged", () => {
    const combined = combineRedshifts([{ z: 0.5000891142048043, zErr: 4.5699592207305544e-5 }]);
    expect(combined).toEqual({
      z: 0.5000891142048043,
      zErr: 4.5699592207305544e-5,
      zErrFormal: 4.5699592207305544e-5,
      chi2: 0,
      dof: 0,
      n: 1,
    });
  });

  it("two lines", () => {
    const combined = redshiftFromLines(THREE_LINES.slice(0, 2));
    expect(combined).not.toBeNull();
    if (!combined) return;
    expect(Math.abs(combined.z - 0.5000770785378363)).toBeLessThan(1e-12);
    expect(relativeDifference(combined.zErrFormal, 3.007080127136866e-5)).toBeLessThan(1e-9);
    expect(relativeDifference(combined.chi2, 0.12232504944013413)).toBeLessThan(1e-6);
    expect(combined.dof).toBe(1);
    expect(combined.n).toBe(2);
    expect(combined.zErr).toBe(combined.zErrFormal);
  });

  it("drops values without a usable error and returns null when none is left", () => {
    const unusable = [
      { z: null, zErr: 1e-4 },
      { z: Number.NaN, zErr: 1e-4 },
      { z: Number.POSITIVE_INFINITY, zErr: 1e-4 },
      { z: 0.5, zErr: null },
      { z: 0.5, zErr: 0 },
      { z: 0.5, zErr: -1e-4 },
      { z: 0.5, zErr: Number.NaN },
      { z: 0.5, zErr: Number.POSITIVE_INFINITY },
    ];
    expect(combineRedshifts([])).toBeNull();
    expect(combineRedshifts(unusable)).toBeNull();
    const combined = combineRedshifts([
      ...unusable,
      { z: 0.5, zErr: 2e-4 },
      { z: 0.501, zErr: 2e-4 },
    ]);
    expect(combined?.n).toBe(2);
    expect(combined?.dof).toBe(1);
    expect(Math.abs((combined?.z ?? 0) - 0.5005)).toBeLessThan(1e-12);

    const fromLines = redshiftFromLines([
      { restUm: 0, centroidUm: 0.98475, centroidErrUm: 3e-5 },
      { restUm: Number.NaN, centroidUm: 0.98475, centroidErrUm: 3e-5 },
      { restUm: -0.656461, centroidUm: 0.98475, centroidErrUm: 3e-5 },
      ...THREE_LINES,
    ]);
    expect(fromLines?.n).toBe(3);
    expect(Math.abs((fromLines?.z ?? 0) - 0.5000755423957206)).toBeLessThan(1e-12);
    expect(redshiftFromLines([{ restUm: 0, centroidUm: 0.98475, centroidErrUm: 3e-5 }])).toBeNull();
    expect(redshiftFromLines([])).toBeNull();
  });

  it("the weighted mean does not depend on the order of the lines", () => {
    const forward = redshiftFromLines(THREE_LINES);
    const backward = redshiftFromLines([...THREE_LINES].reverse());
    expect(forward).not.toBeNull();
    expect(backward).not.toBeNull();
    if (!forward || !backward) return;
    expect(Math.abs(forward.z - backward.z)).toBeLessThan(1e-15);
    expect(relativeDifference(backward.chi2, forward.chi2)).toBeLessThan(1e-9);
  });
});
