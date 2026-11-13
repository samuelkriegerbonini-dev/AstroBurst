import { describe, it, expect } from "vitest";
import {
  AUTO_WINDOWS_RULE,
  GUARD_GAP_LINE_FRACTION,
  GUARD_GAP_MIN_CHANNELS,
  guardGapChannels,
  guardedContinuumWindows,
} from "../lineWindows";

const ROOMY_CASES: [number, number, number][] = [
  [90, 109, 200],
  [1769, 1786, 3610],
  [10, 10, 30],
];

describe("guard-gap rule for automatic continuum windows", () => {
  it("the guard gap is a quarter of the line width and at least two channels", () => {
    expect(GUARD_GAP_MIN_CHANNELS).toBe(2);
    expect(GUARD_GAP_LINE_FRACTION).toBe(0.25);
    expect([1, 3, 8, 9, 18, 20, 25, 40].map(guardGapChannels)).toEqual([2, 2, 2, 3, 5, 5, 7, 10]);
  });

  it("windows are as wide as the line and stand off by the guard gap when the axis has room", () => {
    expect(guardedContinuumWindows({ z0: 90, z1: 109 }, 200)).toEqual([
      [65, 84],
      [115, 134],
    ]);
    expect(guardedContinuumWindows({ z0: 1769, z1: 1786 }, 3610)).toEqual([
      [1746, 1763],
      [1792, 1809],
    ]);
    expect(guardedContinuumWindows({ z0: 10, z1: 10 }, 30)).toEqual([
      [7, 7],
      [13, 13],
    ]);
  });

  it("near an edge the gap shrinks first and then the window", () => {
    expect(guardedContinuumWindows({ z0: 22, z1: 41 }, 200)).toEqual([
      [0, 19],
      [47, 66],
    ]);
    expect(guardedContinuumWindows({ z0: 3, z1: 5 }, 50)).toEqual([
      [0, 2],
      [8, 10],
    ]);
    expect(guardedContinuumWindows({ z0: 150, z1: 189 }, 200)).toEqual([
      [100, 139],
      [190, 199],
    ]);
    expect(guardedContinuumWindows({ z0: 8, z1: 32 }, 40)).toEqual([
      [0, 7],
      [33, 39],
    ]);
  });

  it("a line at an edge keeps one window twice and a line that fills the axis has none", () => {
    expect(guardedContinuumWindows({ z0: 0, z1: 9 }, 100)).toEqual([
      [13, 22],
      [13, 22],
    ]);
    expect(guardedContinuumWindows({ z0: 90, z1: 99 }, 100)).toEqual([
      [77, 86],
      [77, 86],
    ]);
    expect(guardedContinuumWindows({ z0: 0, z1: 39 }, 40)).toBeNull();
  });

  it("invalid ranges give null", () => {
    expect(guardedContinuumWindows({ z0: 12, z1: 10 }, 40)).toBeNull();
    expect(guardedContinuumWindows({ z0: 10, z1: 40 }, 40)).toBeNull();
    expect(guardedContinuumWindows({ z0: -1, z1: 10 }, 40)).toBeNull();
    expect(guardedContinuumWindows({ z0: 10.5, z1: 12 }, 40)).toBeNull();
    expect(guardedContinuumWindows({ z0: 10, z1: 12.5 }, 40)).toBeNull();
    expect(guardedContinuumWindows({ z0: 10, z1: 12 }, 40.5)).toBeNull();
    expect(guardedContinuumWindows({ z0: Number.NaN, z1: 12 }, 40)).toBeNull();
    expect(guardedContinuumWindows({ z0: 0, z1: 0 }, 0)).toBeNull();
  });

  it("the windows never touch or overlap the line range when a gap fits", () => {
    for (const [z0, z1, n] of ROOMY_CASES) {
      const windows = guardedContinuumWindows({ z0, z1 }, n);
      expect(windows).not.toBeNull();
      if (!windows) continue;
      expect(windows[0][1]).toBeLessThan(z0 - 1);
      expect(windows[1][0]).toBeGreaterThan(z1 + 1);
    }
  });

  it("the rule text is the one the tooltip shows", () => {
    expect(AUTO_WINDOWS_RULE).toBe(
      "two windows as wide as the line range, each separated from it by a guard gap of a quarter of the line width (at least 2 channels); near an edge of the axis the gap shrinks first, then the window",
    );
  });
});
