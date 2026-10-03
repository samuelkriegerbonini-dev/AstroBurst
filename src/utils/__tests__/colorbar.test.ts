import { describe, it, expect } from "vitest";
import {
  COLORBAR_MIN_LABEL_GAP_PX,
  colorbarCentre,
  colorbarPixels,
  colorbarTicks,
  colorbarUnit,
  formatTickValue,
  invertStretch,
  niceStep,
  niceTicks,
  tickFraction,
  valueAtFraction,
} from "../colorbar";
import { LUT_BYTES, STRETCH_KIND, stretchValue, transferByte, type DisplayTransfer } from "../displayTransfer";

const LINEAR_0_100: DisplayTransfer = {
  vmin: 0,
  vmax: 100,
  shadow: 0,
  midtone: 0.25,
  highlight: 1,
  stretchKind: STRETCH_KIND.linear,
  asinhA: 0.1,
  power: 2,
  invert: false,
};

function transfer(patch: Partial<DisplayTransfer>): DisplayTransfer {
  return { ...LINEAR_0_100, ...patch };
}

function distinctLut(): Uint8Array {
  const lut = new Uint8Array(LUT_BYTES);
  for (let i = 0; i < 256; i++) {
    lut[i * 4] = i;
    lut[i * 4 + 1] = 255 - i;
    lut[i * 4 + 2] = (i * 7) & 255;
    lut[i * 4 + 3] = 255;
  }
  return lut;
}

function pixelAt(row: Uint8ClampedArray, x: number): number[] {
  return Array.from(row.subarray(x * 4, x * 4 + 4));
}

describe("niceStep", () => {
  it("picks 1, 2, 2.5 or 5 times a power of ten so the span splits into about target - 1 intervals", () => {
    expect(niceStep(100, 5)).toBe(25);
    expect(niceStep(20, 5)).toBe(5);
    expect(niceStep(7, 5)).toBe(2);
    expect(niceStep(1, 5)).toBe(0.25);
    expect(niceStep(0.003, 5)).toBeCloseTo(0.001, 12);
    expect(niceStep(4000, 5)).toBe(1000);
  });

  it("returns 0 for a span that cannot be split", () => {
    expect(niceStep(0, 5)).toBe(0);
    expect(niceStep(Number.NaN, 5)).toBe(0);
  });
});

describe("niceTicks", () => {
  it("splits 0..100 into quarters", () => {
    expect(niceTicks(0, 100, 5)).toEqual([0, 25, 50, 75, 100]);
  });

  it("keeps the limits even when they are not nice values", () => {
    expect(niceTicks(0.37, 99.2, 5)).toEqual([0.37, 25, 50, 75, 99.2]);
  });

  it("returns clean decimal values without float noise", () => {
    expect(niceTicks(0, 0.3, 4)).toEqual([0, 0.1, 0.2, 0.3]);
    expect(niceTicks(-10, 10, 5)).toEqual([-10, -5, 0, 5, 10]);
  });

  it("degrades to the single limit or nothing on a degenerate range", () => {
    expect(niceTicks(3, 3, 5)).toEqual([3]);
    expect(niceTicks(Number.NaN, 1, 5)).toEqual([]);
  });
});

describe("tickFraction", () => {
  it("places linear ticks proportionally", () => {
    expect([0, 25, 50, 75, 100].map((v) => tickFraction(v, LINEAR_0_100))).toEqual([0, 0.25, 0.5, 0.75, 1]);
  });

  it("follows the log stretch: mid values compress toward the top", () => {
    const f = tickFraction(50, transfer({ stretchKind: STRETCH_KIND.log }));
    expect(f).toBeCloseTo(Math.log(501) / Math.log(1001), 6);
    expect(f).toBeGreaterThan(0.5);
  });

  it("keeps the value axis under invert: the strip colours flip, the tick positions do not", () => {
    const inverted = transfer({ invert: true });
    expect([0, 25, 50, 75, 100].map((v) => tickFraction(v, inverted))).toEqual([0, 0.25, 0.5, 0.75, 1]);
    const lut = distinctLut();
    const plain = colorbarPixels(lut, 256, false);
    const flipped = colorbarPixels(lut, 256, true);
    expect(pixelAt(flipped, 0)).toEqual(pixelAt(plain, 255));
    expect(pixelAt(flipped, 255)).toEqual(pixelAt(plain, 0));
  });

  it("puts every value over the colour the viewer paints it with, for every stretch, with and without invert", () => {
    const lut = distinctLut();
    const values = [-5, 3, 12.5, 37, 50, 61.8, 88, 99, 120];
    for (const kind of Object.values(STRETCH_KIND)) {
      for (const invert of [false, true]) {
        const t = transfer({ stretchKind: kind, invert, vmin: -5, vmax: 120, shadow: 0.05, highlight: 0.95 });
        const row = colorbarPixels(lut, 256, invert);
        for (const v of values) {
          const x = Math.round(tickFraction(v, t) * 255);
          const idx = transferByte(v, t);
          expect(pixelAt(row, x), `kind ${kind} invert ${invert} v ${v}`).toEqual(Array.from(lut.subarray(idx * 4, idx * 4 + 4)));
        }
      }
    }
  });
});

describe("invertStretch and valueAtFraction", () => {
  it("inverts stretchValue for every stretch kind", () => {
    for (const kind of Object.values(STRETCH_KIND)) {
      const t = transfer({ stretchKind: kind });
      for (const n of [0.1, 0.5, 0.9]) {
        expect(Math.abs(invertStretch(stretchValue(n, t), t) - n), `kind ${kind} n ${n}`).toBeLessThan(1e-6);
      }
    }
  });

  it("reads back the data value at a position on the bar", () => {
    expect(valueAtFraction(0.25, LINEAR_0_100)).toBeCloseTo(25, 4);
    const log = transfer({ stretchKind: STRETCH_KIND.log });
    expect(valueAtFraction(tickFraction(50, log), log)).toBeCloseTo(50, 3);
    const inverted = transfer({ invert: true });
    expect(valueAtFraction(0.75, inverted)).toBeCloseTo(75, 4);
  });
});

describe("formatTickValue", () => {
  it("chooses decimals from the span", () => {
    expect(formatTickValue(25, 100)).toBe("25");
    expect(formatTickValue(0.25, 0.5)).toBe("0.250");
    expect(formatTickValue(2.5, 20)).toBe("2.5");
    expect(formatTickValue(0.0025, 0.01)).toBe("0.0025");
  });

  it("switches to an exponent for tiny or huge values", () => {
    expect(formatTickValue(2e-4, 1e-3)).toMatch(/e-4$/);
    expect(formatTickValue(2.5e6, 1e7)).toMatch(/e\+6$/);
  });
});

describe("colorbarTicks", () => {
  it("labels the quarters of a linear 0..100 bar", () => {
    const ticks = colorbarTicks(LINEAR_0_100, 400, null);
    expect(ticks.map((t) => t.value)).toEqual([0, 25, 50, 75, 100]);
    expect(ticks.map((t) => t.frac)).toEqual([0, 0.25, 0.5, 0.75, 1]);
    expect(ticks.map((t) => t.label)).toEqual(["0", "25", "50", "75", "100"]);
    expect(ticks.every((t) => t.kind === "value")).toBe(true);
  });

  it("drops crowded labels but keeps their marks and always labels the ends", () => {
    const ticks = colorbarTicks(LINEAR_0_100, 150, null);
    expect(ticks.map((t) => t.value)).toEqual([0, 25, 50, 75, 100]);
    expect(ticks.map((t) => t.label)).toEqual(["0", "", "50", "", "100"]);
    const kept = ticks.filter((t) => t.label !== "").map((t) => t.frac * 150);
    for (let i = 1; i < kept.length; i++) {
      expect(kept[i] - kept[i - 1]).toBeGreaterThanOrEqual(COLORBAR_MIN_LABEL_GAP_PX);
    }
  });

  it("drops a label whose text would run into the left-aligned first label even when the anchors are far enough apart", () => {
    const t = transfer({ vmin: 123450, vmax: 123550 });
    const ticks = colorbarTicks(t, 200, null);
    expect(ticks.map((k) => k.value)).toEqual([123450, 123475, 123500, 123525, 123550]);
    expect(ticks[1].frac * 200 - ticks[0].frac * 200).toBeGreaterThanOrEqual(COLORBAR_MIN_LABEL_GAP_PX);
    expect(ticks.map((k) => k.label)).toEqual(["123450", "", "123500", "", "123550"]);
    expect(colorbarTicks(t, 400, null).map((k) => k.label)).toEqual(["123450", "123475", "123500", "123525", "123550"]);
  });

  it("marks the centre of symmetric limits at the middle of a linear bar", () => {
    const ticks = colorbarTicks(transfer({ vmin: -10, vmax: 10 }), 400, 0);
    const centre = ticks.filter((t) => t.kind === "centre");
    expect(centre).toHaveLength(1);
    expect(centre[0].value).toBe(0);
    expect(centre[0].frac).toBe(0.5);
    expect(centre[0].label).toBe("0.0");
    expect(ticks.filter((t) => t.value === 0)).toHaveLength(1);
  });

  it("adds a centre that is not a nice value as its own tick and ignores one outside the limits", () => {
    const t = transfer({ vmin: -10, vmax: 10 });
    const ticks = colorbarTicks(t, 400, 1.3);
    expect(ticks.filter((k) => k.kind === "centre").map((k) => k.value)).toEqual([1.3]);
    expect(ticks.map((k) => k.value)).toEqual([...ticks.map((k) => k.value)].sort((a, b) => a - b));
    expect(colorbarTicks(t, 400, 50).some((k) => k.kind === "centre")).toBe(false);
  });

  it("returns no ticks for non-finite limits", () => {
    expect(colorbarTicks(transfer({ vmin: Number.NaN }), 400, null)).toEqual([]);
    expect(colorbarTicks(transfer({ vmin: Number.NaN, stretchKind: STRETCH_KIND.mtf }), 400, null)).toEqual([]);
  });
});

const MTF_PROBE = transfer({ stretchKind: STRETCH_KIND.mtf, vmin: -50, vmax: 5000, shadow: 0.0095, midtone: 0.004, highlight: 1 });

function significantDigits(value: number): number {
  if (value === 0) return 0;
  const digits = Math.abs(value).toExponential().split("e")[0].replace(".", "").replace(/0+$/, "");
  return digits.length;
}

function interiorWithinQuarterSpacing(ticks: ReturnType<typeof colorbarTicks>): void {
  const n = ticks.length - 1;
  ticks.slice(1, -1).forEach((tick, k) => {
    expect(Math.abs(tick.frac - (k + 1) / n), `tick ${tick.value}`).toBeLessThanOrEqual(0.25 / n + 1e-6);
  });
}

describe("colorbarTicks under non-linear stretches", () => {
  it("spreads the default MTF ticks over the bar instead of piling them into its last percent", () => {
    const ticks = colorbarTicks(MTF_PROBE, 800, null);
    expect(ticks.length).toBeGreaterThanOrEqual(5);
    expect(ticks.length).toBeLessThanOrEqual(7);
    expect(ticks[0]).toMatchObject({ value: -50, frac: 0, label: "-50" });
    expect(ticks[ticks.length - 1]).toMatchObject({ value: 5000, frac: 1, label: "5000" });
    for (let i = 1; i < ticks.length; i++) {
      expect(ticks[i].frac - ticks[i - 1].frac, `ticks ${i - 1}-${i}`).toBeGreaterThanOrEqual(0.08);
    }
    const inner = ticks.slice(1, -1);
    expect(inner.filter((k) => k.frac > 0.1 && k.frac < 0.9).length).toBeGreaterThanOrEqual(3);
    expect(inner.every((k) => k.label !== "")).toBe(true);
    expect(new Set(ticks.map((k) => k.label)).size).toBe(ticks.length);
    for (const k of ticks) expect(k.frac).toBe(tickFraction(k.value, MTF_PROBE));
  });

  it("rounds each interior value to two or three significant digits that stay within a quarter spacing of its even position", () => {
    for (const width of [150, 300, 500, 800]) {
      const ticks = colorbarTicks(MTF_PROBE, width, null);
      expect(ticks.length).toBeGreaterThanOrEqual(5);
      interiorWithinQuarterSpacing(ticks);
      for (const k of ticks.slice(1, -1)) {
        expect(significantDigits(k.value), `width ${width} value ${k.value}`).toBeLessThanOrEqual(3);
        expect(k.label === "" || Number(k.label) === k.value, `label ${k.label}`).toBe(true);
      }
    }
  });

  it("adds digits on a narrow range far from zero so the ticks stay distinct and inside the limits", () => {
    const t = transfer({ stretchKind: STRETCH_KIND.sqrt, vmin: 1000, vmax: 1010 });
    const ticks = colorbarTicks(t, 400, null);
    expect(ticks.length).toBe(5);
    interiorWithinQuarterSpacing(ticks);
    for (let i = 1; i < ticks.length; i++) expect(ticks[i].value).toBeGreaterThan(ticks[i - 1].value);
    expect(new Set(ticks.map((k) => k.label)).size).toBe(ticks.length);
    for (const k of ticks.slice(1, -1)) expect(Number(k.label)).toBe(k.value);
  });

  it("gives the two ends distinct labels when the span-based format would print the same text for both", () => {
    const t = transfer({ stretchKind: STRETCH_KIND.mtf, vmin: 1, vmax: 1.00004, midtone: 0.1 });
    expect(formatTickValue(t.vmin, t.vmax - t.vmin)).toBe(formatTickValue(t.vmax, t.vmax - t.vmin));
    const ticks = colorbarTicks(t, 600, null);
    expect(ticks[0].label).toBe("1");
    expect(ticks[ticks.length - 1].label).toBe("1.00004");
    expect(new Set(ticks.map((k) => k.label)).size).toBe(ticks.length);
  });

  it("keeps the log stretch compressing: equal steps along the bar are ever larger steps in value", () => {
    const t = transfer({ stretchKind: STRETCH_KIND.log });
    const ticks = colorbarTicks(t, 400, null);
    expect(ticks.length).toBe(5);
    interiorWithinQuarterSpacing(ticks);
    const inner = ticks.slice(1, -1).map((k) => k.value);
    expect(inner.every((v) => v > 0 && v < 25)).toBe(true);
    for (let i = 2; i < ticks.length; i++) {
      expect(ticks[i].value - ticks[i - 1].value).toBeGreaterThan(ticks[i - 1].value - ticks[i - 2].value);
    }
    for (const k of ticks) expect(k.frac).toBe(tickFraction(k.value, t));
  });

  it("places the same ticks under invert: the strip colours flip, the value axis does not", () => {
    expect(colorbarTicks({ ...MTF_PROBE, invert: true }, 800, null)).toEqual(colorbarTicks(MTF_PROBE, 800, null));
    const asinh = transfer({ stretchKind: STRETCH_KIND.asinh, vmin: -10, vmax: 10 });
    expect(colorbarTicks({ ...asinh, invert: true }, 400, 0)).toEqual(colorbarTicks(asinh, 400, 0));
  });

  it("marks the centre of symmetric limits on a non-linear bar once, where the stretch puts it", () => {
    const t = transfer({ stretchKind: STRETCH_KIND.asinh, vmin: -10, vmax: 10 });
    const ticks = colorbarTicks(t, 400, 0);
    const centre = ticks.filter((k) => k.kind === "centre");
    expect(centre).toHaveLength(1);
    expect(centre[0].value).toBe(0);
    expect(centre[0].frac).toBe(tickFraction(0, t));
    expect(centre[0].label).toBe("0");
    expect(ticks.filter((k) => k.value === 0)).toHaveLength(1);
    const offCentre = colorbarTicks(t, 400, 1.3);
    expect(offCentre.filter((k) => k.kind === "centre").map((k) => k.value)).toEqual([1.3]);
    expect(offCentre.map((k) => k.frac)).toEqual([...offCentre.map((k) => k.frac)].sort((a, b) => a - b));
  });
});

describe("colorbarCentre", () => {
  it("uses the centre only when symmetric limits were requested and the backend applied them", () => {
    expect(colorbarCentre(true, { symmetric: true, centre: 0 })).toBe(0);
    expect(colorbarCentre(true, { symmetric: true, centre: 2.5 })).toBe(2.5);
  });

  it("draws no centre on display-referred data, where the backend skipped symmetric limits", () => {
    expect(colorbarCentre(true, { symmetric: false, centre: null })).toBeNull();
    expect(colorbarCentre(true, { symmetric: true, centre: null })).toBeNull();
    expect(colorbarCentre(true, null)).toBeNull();
    expect(colorbarCentre(false, { symmetric: true, centre: 0 })).toBeNull();
  });
});

describe("colorbarPixels", () => {
  it("paints the LUT from its first to its last entry, reversed under invert", () => {
    const lut = distinctLut();
    const row = colorbarPixels(lut, 300, false);
    expect(row).toHaveLength(300 * 4);
    expect(pixelAt(row, 0)).toEqual(Array.from(lut.subarray(0, 4)));
    expect(pixelAt(row, 299)).toEqual(Array.from(lut.subarray(1020, 1024)));
    const inv = colorbarPixels(lut, 300, true);
    expect(pixelAt(inv, 0)).toEqual(Array.from(lut.subarray(1020, 1024)));
    expect(pixelAt(inv, 299)).toEqual(Array.from(lut.subarray(0, 4)));
  });

  it("handles one-pixel and empty widths", () => {
    const lut = distinctLut();
    expect(pixelAt(colorbarPixels(lut, 1, false), 0)).toEqual(Array.from(lut.subarray(0, 4)));
    expect(colorbarPixels(lut, 0, false)).toHaveLength(0);
  });
});

describe("colorbarUnit", () => {
  it("shows the header BUNIT without quotes on the original file", () => {
    expect(colorbarUnit("'MJy/sr'", false)).toBe("MJy/sr");
    expect(colorbarUnit(" MJy/sr  ", false)).toBe("MJy/sr");
  });

  it("shows no unit for processed results or a missing BUNIT", () => {
    expect(colorbarUnit("'MJy/sr'", true)).toBeNull();
    expect(colorbarUnit(undefined, false)).toBeNull();
    expect(colorbarUnit(null, false)).toBeNull();
    expect(colorbarUnit("''", false)).toBeNull();
  });
});
