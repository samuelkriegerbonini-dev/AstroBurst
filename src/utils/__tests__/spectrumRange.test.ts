import { describe, it, expect } from "vitest";
import {
  axisTextForChannel,
  axisValueToPixel,
  channelAxisValue,
  channelFromAxisText,
  channelRangeFromDrag,
  channelWidthAt,
  clampRange,
  commitRangeEdge,
  commitWindowEdge,
  continuumOverlapNote,
  defaultContinuumWindows,
  formatRangeLabel,
  fullCubeCollapse,
  nearestChannel,
  noSpectralAxisText,
  parseChannelInput,
  pixelToAxisValue,
  rangeBounds,
  rangePixelSpan,
  spectralSectionsVisible,
  spectrumXTicks,
  windowsAreValid,
  windowsOverlapRange,
  type PlotMapping,
} from "../spectrumRange";
import { formatAxisValue } from "../spectralAxis";
import type { ContinuumWindows } from "../../shared/types/cube";
import type { RampInfo } from "../../shared/types/ramp";

const WAVE = Array.from({ length: 40 }, (_, i) => 1.0 + i * 0.001);
const FREQ = WAVE.map((w) => 299792.458 / w);

function mapping(xValues: number[] | null, n = 40): PlotMapping {
  const values = xValues ?? Array.from({ length: n }, (_, i) => i);
  return {
    width: 400,
    padLeft: 50,
    padRight: 12,
    xMin: Math.min(...values),
    xMax: Math.max(...values),
    xValues,
    n,
  };
}

describe("pixel and axis value mapping", () => {
  it("round-trips pixels through axis values", () => {
    const m = mapping(WAVE);
    for (const px of [50, 120, 300, 388]) {
      expect(axisValueToPixel(pixelToAxisValue(px, m), m)).toBeCloseTo(px, 9);
    }
    expect(pixelToAxisValue(50, m)).toBeCloseTo(1.0, 12);
    expect(pixelToAxisValue(388, m)).toBeCloseTo(1.039, 12);
  });

  it("reads channel values from the axis or the channel index", () => {
    expect(channelAxisValue(20, mapping(WAVE))).toBeCloseTo(1.02, 12);
    expect(channelAxisValue(20, mapping(null))).toBe(20);
  });
});

describe("nearestChannel", () => {
  it("finds the closest channel on increasing and decreasing axes", () => {
    expect(nearestChannel(1.0204, mapping(WAVE))).toBe(20);
    expect(nearestChannel(1.0206, mapping(WAVE))).toBe(21);
    expect(nearestChannel(FREQ[7] + 1e-6, mapping(FREQ))).toBe(7);
    expect(nearestChannel(FREQ[0] + 100, mapping(FREQ))).toBe(0);
  });

  it("clamps channel-index axes and rejects empty or non-finite input", () => {
    expect(nearestChannel(12.4, mapping(null))).toBe(12);
    expect(nearestChannel(-5, mapping(null))).toBe(0);
    expect(nearestChannel(99, mapping(null))).toBe(39);
    expect(nearestChannel(NaN, mapping(null))).toBeNull();
    expect(nearestChannel(3, mapping(null, 0))).toBeNull();
    expect(nearestChannel(1.0, mapping([NaN, NaN], 2))).toBeNull();
  });
});

describe("channelRangeFromDrag", () => {
  it("orders a reversed drag and clamps drags outside the plot", () => {
    const m = mapping(WAVE);
    const start = axisValueToPixel(WAVE[25], m);
    const end = axisValueToPixel(WAVE[15], m);
    expect(channelRangeFromDrag(start, end, m)).toEqual({ z0: 15, z1: 25 });
    expect(channelRangeFromDrag(-100, 10_000, m)).toEqual({ z0: 0, z1: 39 });
  });

  it("maps decreasing frequency axes back to channel order", () => {
    const m = mapping(FREQ);
    const start = axisValueToPixel(FREQ[30], m);
    const end = axisValueToPixel(FREQ[10], m);
    expect(channelRangeFromDrag(start, end, m)).toEqual({ z0: 10, z1: 30 });
  });

  it("returns null without channels", () => {
    expect(channelRangeFromDrag(60, 100, mapping(null, 0))).toBeNull();
  });
});

describe("clampRange", () => {
  it("sorts, rounds and clamps", () => {
    expect(clampRange({ z0: 30.4, z1: 2.6 }, 40)).toEqual({ z0: 3, z1: 30 });
    expect(clampRange({ z0: -4, z1: 400 }, 40)).toEqual({ z0: 0, z1: 39 });
    expect(clampRange({ z0: 1, z1: 2 }, 0)).toBeNull();
    expect(clampRange({ z0: NaN, z1: 2 }, 40)).toBeNull();
  });
});

describe("range bounds, spans and labels", () => {
  it("reports sorted axis bounds and a shaded span that covers half a channel each side", () => {
    const m = mapping(FREQ);
    const bounds = rangeBounds({ z0: 10, z1: 30 }, m);
    expect(bounds.lo).toBeCloseTo(FREQ[30], 6);
    expect(bounds.hi).toBeCloseTo(FREQ[10], 6);
    const span = rangePixelSpan({ z0: 10, z1: 30 }, m);
    expect(span.x0).toBeLessThan(axisValueToPixel(FREQ[30], m));
    expect(span.x1).toBeGreaterThan(axisValueToPixel(FREQ[10], m));
    expect(span.x0).toBeGreaterThanOrEqual(m.padLeft);
    expect(span.x1).toBeLessThanOrEqual(m.width - m.padRight);
  });

  it("uses half-channel padding on channel-index axes", () => {
    const m = mapping(null);
    const span = rangePixelSpan({ z0: 5, z1: 5 }, m);
    expect(span.x0).toBeCloseTo(axisValueToPixel(4.5, m), 9);
    expect(span.x1).toBeCloseTo(axisValueToPixel(5.5, m), 9);
  });

  it("formats labels with and without axis values", () => {
    expect(formatRangeLabel({ z0: 15, z1: 25 }, mapping(WAVE), "μm")).toBe("ch 15–25 (1.0150–1.0250 μm)");
    expect(formatRangeLabel({ z0: 4, z1: 4 }, mapping(null), "μm")).toBe("ch 4");
    const kms = mapping([-300, -200, -100, 0], 4);
    expect(formatRangeLabel({ z0: 1, z1: 2 }, kms, "km/s")).toBe("ch 1–2 (-200.0–-100.0 km/s)");
  });

  it("keeps range bounds apart when channels are finer than 1e-4 um", () => {
    const fine = Array.from({ length: 40 }, (_, i) => 0.658 + i * 2e-6);
    const label = formatRangeLabel({ z0: 10, z1: 20 }, mapping(fine), "μm");
    const [, lo, hi] = /\(([^–]+)–([^ ]+) μm\)/.exec(label)!;
    expect(Math.abs(Number(lo) - fine[10])).toBeLessThan(1e-6);
    expect(Math.abs(Number(hi) - fine[20])).toBeLessThan(1e-6);
    expect(label).toBe("ch 10–20 (0.658020–0.658040 μm)");
  });

  it("keeps range bounds apart on a velocity grid finer than 0.1 km/s", () => {
    const fine = mapping([-1.0, -0.96, -0.92, -0.88], 4);
    expect(formatRangeLabel({ z0: 1, z1: 2 }, fine, "km/s")).toBe("ch 1–2 (-0.96–-0.92 km/s)");
  });
});

describe("channelWidthAt", () => {
  it("uses centred differences with one-sided edges", () => {
    const values = [0, 10, 30, 60];
    expect([0, 1, 2, 3].map((i) => channelWidthAt(values, i))).toEqual([10, 15, 25, 30]);
    expect(channelWidthAt([5], 0)).toBe(0);
    expect(channelWidthAt(values, 9)).toBe(0);
  });
});

describe("continuum windows", () => {
  it("proposes windows on both sides of a range", () => {
    expect(defaultContinuumWindows({ z0: 8, z1: 32 }, 40)).toEqual([
      [3, 7],
      [33, 37],
    ]);
    expect(defaultContinuumWindows({ z0: 2, z1: 32 }, 40, 5)).toEqual([
      [0, 1],
      [33, 37],
    ]);
  });

  it("repeats the only free side and gives up on a full range", () => {
    expect(defaultContinuumWindows({ z0: 0, z1: 30 }, 40)).toEqual([
      [31, 35],
      [31, 35],
    ]);
    expect(defaultContinuumWindows({ z0: 10, z1: 39 }, 40)).toEqual([
      [5, 9],
      [5, 9],
    ]);
    expect(defaultContinuumWindows({ z0: 0, z1: 39 }, 40)).toBeNull();
    expect(defaultContinuumWindows({ z0: 0, z1: 3 }, 0)).toBeNull();
  });

  it("validates windows against the channel count", () => {
    expect(windowsAreValid([[0, 5], [34, 39]], 40)).toBe(true);
    expect(windowsAreValid([[0, 5], [34, 40]], 40)).toBe(false);
    expect(windowsAreValid([[6, 5], [34, 39]], 40)).toBe(false);
    expect(windowsAreValid([[0.5, 5], [34, 39]], 40)).toBe(false);
    expect(windowsAreValid(null, 40)).toBe(false);
  });
});

describe("parseChannelInput", () => {
  it("accepts integer channels inside the cube", () => {
    expect(parseChannelInput(" 12 ", 40)).toBe(12);
    expect(parseChannelInput("39", 40)).toBe(39);
    expect(parseChannelInput("40", 40)).toBeNull();
    expect(parseChannelInput("-1", 40)).toBeNull();
    expect(parseChannelInput("1.5", 40)).toBeNull();
    expect(parseChannelInput("abc", 40)).toBeNull();
  });

  it("reads a blank box as no channel instead of channel 0", () => {
    expect(parseChannelInput("", 40)).toBeNull();
    expect(parseChannelInput("   ", 40)).toBeNull();
    expect(parseChannelInput("0", 40)).toBe(0);
  });
});

describe("commitWindowEdge", () => {
  const WINDOWS: ContinuumWindows = [
    [50, 55],
    [72, 80],
  ];

  it("moves only the edited edge of the edited window", () => {
    expect(commitWindowEdge(WINDOWS, 1, 1, 75)).toEqual([
      [50, 55],
      [72, 75],
    ]);
    expect(commitWindowEdge(WINDOWS, 0, 0, 45)).toEqual([
      [45, 55],
      [72, 80],
    ]);
  });

  it("clamps a 'to' below 'from' onto 'from' instead of rewriting 'from'", () => {
    expect(commitWindowEdge(WINDOWS, 1, 1, 7)).toEqual([
      [50, 55],
      [72, 72],
    ]);
  });

  it("clamps a 'from' above 'to' onto 'to' instead of rewriting 'to'", () => {
    expect(commitWindowEdge(WINDOWS, 1, 0, 90)).toEqual([
      [50, 55],
      [80, 80],
    ]);
  });

  it("starts both windows on the value when none are set and never mutates its input", () => {
    expect(commitWindowEdge(null, 1, 0, 12)).toEqual([
      [12, 12],
      [12, 12],
    ]);
    commitWindowEdge(WINDOWS, 1, 1, 75);
    expect(WINDOWS).toEqual([
      [50, 55],
      [72, 80],
    ]);
  });
});

describe("commitRangeEdge", () => {
  it("moves only the edited edge and clamps it at the other edge", () => {
    expect(commitRangeEdge({ z0: 60, z1: 70 }, "z1", 65)).toEqual({ z0: 60, z1: 65 });
    expect(commitRangeEdge({ z0: 60, z1: 70 }, "z1", 7)).toEqual({ z0: 60, z1: 60 });
    expect(commitRangeEdge({ z0: 60, z1: 70 }, "z0", 75)).toEqual({ z0: 70, z1: 70 });
    expect(commitRangeEdge({ z0: 60, z1: 70 }, "z0", 55)).toEqual({ z0: 55, z1: 70 });
  });
});

describe("continuum windows over the line range", () => {
  const RANGE = { z0: 60, z1: 70 };

  it("names each window that shares a channel with the line range", () => {
    expect(windowsOverlapRange([[50, 55], [65, 75]], RANGE)).toEqual([1]);
    expect(windowsOverlapRange([[50, 60], [72, 80]], RANGE)).toEqual([0]);
    expect(windowsOverlapRange([[62, 64], [55, 75]], RANGE)).toEqual([0, 1]);
    expect(windowsOverlapRange([[50, 59], [71, 80]], RANGE)).toEqual([]);
    expect(windowsOverlapRange(null, RANGE)).toEqual([]);
    expect(windowsOverlapRange([[50, 59], [65, 75]], null)).toEqual([]);
  });

  it("words the overlap note for what can run", () => {
    expect(continuumOverlapNote([[50, 55], [65, 75]], RANGE)).toBe(
      "Continuum window B overlaps the line range; Measure line, Moments and Fit lines would fit their continuum over line flux.",
    );
    expect(continuumOverlapNote([[58, 61], [72, 80]], RANGE, false)).toBe(
      "Continuum window A overlaps the line range; Measure line, Moments and Fit lines would fit their continuum over line flux.",
    );
    expect(continuumOverlapNote([[62, 64], [55, 75]], RANGE)).toBe(
      "Continuum windows A and B overlap the line range; Measure line, Moments and Fit lines would fit their continuum over line flux.",
    );
    expect(continuumOverlapNote([[58, 61], [72, 80]], RANGE, true)).toBe(
      "Continuum window A overlaps the line range; Measure line would fit its continuum over line flux.",
    );
    expect(continuumOverlapNote([[62, 64], [55, 75]], RANGE, true)).toBe(
      "Continuum windows A and B overlap the line range; Measure line would fit its continuum over line flux.",
    );
    expect(continuumOverlapNote([[50, 59], [71, 80]], RANGE)).toBeNull();
    expect(continuumOverlapNote([[50, 59], [71, 80]], RANGE, true)).toBeNull();
    expect(continuumOverlapNote(null, RANGE)).toBeNull();
    expect(continuumOverlapNote(null, RANGE, true)).toBeNull();
  });

  it("says spectrum for a file and cube for a cube", () => {
    expect(noSpectralAxisText(true)).toBe("no spectral axis for this spectrum");
    expect(noSpectralAxisText(false)).toBe("no spectral axis for this cube");
  });
});

describe("spectralSectionsVisible", () => {
  const RAMP = { kind: "jwst_groups", nints: 1, ngroups: 10 } as RampInfo;

  it("hides the spectral-only sections for a ramp and shows them for any other cube", () => {
    expect(spectralSectionsVisible(RAMP)).toBe(false);
    expect(spectralSectionsVisible(null)).toBe(true);
  });
});

describe("fullCubeCollapse", () => {
  it("asks for one collapse over every channel instead of the per-frame preview pipeline", () => {
    expect(fullCubeCollapse(3681, "mean")).toEqual({ z0: 0, z1: 3680, mode: "mean" });
    expect(fullCubeCollapse(2, "median")).toEqual({ z0: 0, z1: 1, mode: "median" });
  });

  it("refuses a cube with no known channel count", () => {
    expect(fullCubeCollapse(0, "mean")).toBeNull();
    expect(fullCubeCollapse(Number.NaN, "mean")).toBeNull();
  });
});

function linearAxis(crval: number, cdelt: number, n: number): number[] {
  return Array.from({ length: n }, (_, i) => crval + i * cdelt);
}

function expectInsidePlotAscending(px: number[]): void {
  for (const p of px) {
    expect(p).toBeGreaterThanOrEqual(50);
    expect(p).toBeLessThanOrEqual(388);
  }
  for (let i = 1; i < px.length; i++) expect(px[i]).toBeGreaterThan(px[i - 1]);
}

describe("spectrumXTicks", () => {
  it("labels the MIRI ch3-short wavelength axis with ticks inside the plot", () => {
    const m = mapping(linearAxis(11.551250190706924, 0.002499999944120645, 769), 769);
    const ticks = spectrumXTicks(m);
    expect(ticks.map((t) => t.label)).toEqual(["12.0", "12.5", "13.0"]);
    expectInsidePlotAscending(ticks.map((t) => t.px));
    expect(ticks[1].px).toBeCloseTo(axisValueToPixel(12.5, m), 9);
  });

  it("labels the NIRSpec G395H wavelength axis", () => {
    const m = mapping(linearAxis(2.870332385558868, 0.000664999999571591, 3610), 3610);
    const ticks = spectrumXTicks(m);
    expect(ticks.map((t) => t.label)).toEqual(["3.0", "3.5", "4.0", "4.5", "5.0"]);
    expectInsidePlotAscending(ticks.map((t) => t.px));
  });

  it("labels channel indices when the cube has no spectral axis", () => {
    const ticks = spectrumXTicks(mapping(null, 769));
    expect(ticks.map((t) => t.label)).toEqual(["0", "200", "400", "600"]);
    expect(ticks[0].px).toBe(50);
    expectInsidePlotAscending(ticks.map((t) => t.px));
  });

  it("labels only whole channels on a 3-channel axis without spectral values", () => {
    const ticks = spectrumXTicks(mapping(null, 3));
    expect(ticks.map((t) => t.label)).toEqual(["0", "1", "2"]);
    expect(ticks.map((t) => t.px)).toEqual([50, 219, 388]);
  });

  it("labels only whole channels on 2- and 4-channel axes without spectral values", () => {
    expect(spectrumXTicks(mapping(null, 2)).map((t) => t.label)).toEqual(["0", "1"]);
    expect(spectrumXTicks(mapping(null, 4)).map((t) => t.label)).toEqual(["0", "1", "2", "3"]);
  });

  it("keeps fractional ticks on a short spectral axis", () => {
    expect(spectrumXTicks(mapping([1.0, 1.5, 2.0], 3)).map((t) => t.label)).toEqual(["1.0", "1.2", "1.4", "1.6", "1.8", "2.0"]);
  });

  it("keeps ticks that fall exactly on the ends of a velocity axis", () => {
    const ticks = spectrumXTicks(mapping(linearAxis(-3000, 6000 / 768, 769), 769));
    expect(ticks.map((t) => t.label)).toEqual(["-3000", "-2000", "-1000", "0", "1000", "2000", "3000"]);
    expect(ticks[0].px).toBeCloseTo(50, 9);
    expect(ticks[ticks.length - 1].px).toBeCloseTo(388, 9);
  });
});

describe("range boxes in the unit of the axis", () => {
  const xs = Array.from({ length: 40 }, (_, i) => 0.6555 + i * 1e-4);
  const m: PlotMapping = { width: 400, padLeft: 62, padRight: 12, xMin: xs[0], xMax: xs[39], xValues: xs, n: 40 };
  const NO_AXIS = { channel: null, error: "No axis values for these channels" };

  it("prints the axis value of a channel at the resolution of the axis", () => {
    expect(axisTextForChannel(20, m, "μm")).toBe("0.6575");
    const fine = Array.from({ length: 40 }, (_, i) => 0.658 + i * 2e-6);
    expect(axisTextForChannel(1, { ...m, xMin: fine[0], xMax: fine[39], xValues: fine }, "μm")).toBe("0.658002");
    expect(axisTextForChannel(null, m, "μm")).toBe("");
    expect(axisTextForChannel(20, { ...m, xValues: null }, "μm")).toBe("20");
  });

  it("finds the channel of a typed axis value", () => {
    expect(channelFromAxisText("0.6575", m, "μm")).toEqual({ channel: 20, error: null });
    expect(channelFromAxisText("0.65753", m, "μm").channel).toBe(20);
    expect(channelFromAxisText("0.65756", m, "μm").channel).toBe(21);
    expect(channelFromAxisText(" 0.6555 ", m, "μm").channel).toBe(0);
    expect(channelFromAxisText("0.6594", m, "μm").channel).toBe(39);
    expect(channelFromAxisText("0.65546", m, "μm").channel).toBe(0);
    expect(channelFromAxisText("0.65944", m, "μm").channel).toBe(39);
    const single: PlotMapping = { ...m, xMin: 0.6575, xMax: 0.6575, xValues: [0.6575], n: 1 };
    expect(channelFromAxisText("0.6575", single, "μm")).toEqual({ channel: 0, error: null });
    expect(channelFromAxisText("0.66", single, "μm")).toEqual({ channel: null, error: "Use a value from 0.6575 to 0.6575 μm" });
  });

  it("refuses what it cannot place, with a reason", () => {
    const outside = { channel: null, error: "Use a value from 0.6555 to 0.6594 μm" };
    expect(channelFromAxisText("0.65", m, "μm")).toEqual(outside);
    expect(channelFromAxisText("0.7", m, "μm")).toEqual(outside);
    expect(channelFromAxisText("", m, "μm")).toEqual({ channel: null, error: "Enter a number" });
    expect(channelFromAxisText("0,6575", m, "μm")).toEqual({ channel: null, error: "Use a dot (.) as the decimal separator" });
    expect(channelFromAxisText("abc", m, "μm")).toEqual({ channel: null, error: "Not a number" });
    expect(channelFromAxisText("0.6575", { ...m, xValues: null }, "μm")).toEqual(NO_AXIS);
    expect(channelFromAxisText("0.6575", { ...m, xValues: [], n: 0 }, "μm")).toEqual(NO_AXIS);
  });

  it("works on a descending axis and on a scaled one", () => {
    const reversed: PlotMapping = { ...m, xValues: [...xs].reverse() };
    expect(channelFromAxisText("0.6575", reversed, "μm")).toEqual({ channel: 19, error: null });
    expect(axisTextForChannel(0, reversed, "μm")).toBe("0.6594");
    const nm = xs.map((x) => x * 1000);
    const scaled: PlotMapping = { ...m, xMin: nm[0], xMax: nm[39], xValues: nm };
    expect(channelFromAxisText("657.5", scaled, "nm")).toEqual({ channel: 20, error: null });
    expect(axisTextForChannel(20, scaled, "nm")).toBe(formatAxisValue(657.5, "nm", 0.1));
  });
});
