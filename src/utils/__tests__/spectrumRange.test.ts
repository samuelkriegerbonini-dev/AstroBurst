import { describe, it, expect } from "vitest";
import {
  axisValueToPixel,
  channelAxisValue,
  channelRangeFromDrag,
  channelWidthAt,
  clampRange,
  defaultContinuumWindows,
  formatRangeLabel,
  fullCubeCollapse,
  nearestChannel,
  parseChannelInput,
  pixelToAxisValue,
  rangeBounds,
  rangePixelSpan,
  spectrumXTicks,
  windowsAreValid,
  type PlotMapping,
} from "../spectrumRange";

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
