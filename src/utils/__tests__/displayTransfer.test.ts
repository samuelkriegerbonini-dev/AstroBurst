import { describe, it, expect } from "vitest";
import {
  GRAY_LUT_RGBA,
  LUT_WITH_NODATA_BYTES,
  NODATA_INDEX,
  STRETCH_KIND,
  SYMMETRIC_NONLINEAR_STRETCH_NOTE,
  centreDraftFor,
  isPaddingValue,
  lutIndex,
  lutNodataRgb,
  normalize,
  parseCentreDraft,
  reconcileDisplayPatch,
  renderRgba,
  resolveTransferLimits,
  stretchValue,
  symmetricStretchNote,
  toDisplayTransfer,
  transferByte,
  withNodataEntry,
  type DisplayTransfer,
} from "../displayTransfer";
import { DEFAULT_DISPLAY_SETTINGS } from "../../shared/types/display";

const BASE: DisplayTransfer = {
  vmin: 0,
  vmax: 1,
  shadow: 0,
  midtone: 0.5,
  highlight: 1,
  stretchKind: 1,
  asinhA: 0.1,
  power: 2,
  invert: false,
};

const GOLDEN_X = [0, 0.25, 0.5, 0.75, 1];
const GOLDEN_WINDOW = { vmin: 1, vmax: 2 };
const GOLDEN_RAW = GOLDEN_X.map((x) => x + GOLDEN_WINDOW.vmin);
const GOLDEN: Record<Exclude<keyof typeof STRETCH_KIND, "mtf">, number[]> = {
  linear: [0, 64, 128, 191, 255],
  log: [0, 204, 229, 244, 255],
  sqrt: [0, 128, 180, 221, 255],
  asinh: [0, 140, 197, 231, 255],
  power: [0, 16, 64, 143, 255],
};

function withKind(kind: DisplayTransfer["stretchKind"], invert = false): DisplayTransfer {
  return { ...BASE, stretchKind: kind, invert };
}

function goldenKind(kind: DisplayTransfer["stretchKind"], invert = false): DisplayTransfer {
  return { ...withKind(kind, invert), ...GOLDEN_WINDOW };
}

function oldStfTransfer(
  raw: number,
  invRange: number,
  dataMin: number,
  shadow: number,
  clipRange: number,
  midtone: number,
): number {
  if (raw !== raw) return 0;
  const norm = (raw - dataMin) * invRange;
  const clipped = Math.max(0, Math.min(1, (norm - shadow) / clipRange));
  if (clipped <= 0) return 0;
  if (clipped >= 1) return 1;
  if (Math.abs(midtone - 0.5) < 1e-6) return clipped;
  const b = (2 * midtone - 1) * clipped - midtone;
  if (Math.abs(b) < 1e-8) return clipped;
  return ((midtone - 1) * clipped) / b;
}

const f = Math.fround;

function shaderMtf(m: number, x: number): number {
  if (x <= 0) return 0;
  if (x >= 1) return 1;
  if (f(Math.abs(f(m - 0.5))) < f(1e-6)) return x;
  const a = f(f(m - 1) * x);
  const b = f(f(f(f(2 * m) - 1) * x) - m);
  if (f(Math.abs(b)) < f(1e-8)) return x;
  return f(a / b);
}

function shaderByte(val: number, t: DisplayTransfer): number {
  if (!Number.isFinite(val) || val === 0) return NODATA_INDEX;
  const vmin = f(t.vmin);
  const vmax = f(t.vmax);
  const range = f(vmax - vmin);
  const n = range > 0 ? f(Math.min(1, Math.max(0, f(f(val - vmin) / range)))) : 0;
  let y: number;
  if (t.stretchKind === 0) {
    const shadow = f(t.shadow);
    const highlight = f(t.highlight);
    const clipRange = Math.max(f(highlight - shadow), f(1e-8));
    const x = f(Math.min(1, Math.max(0, f(f(n - shadow) / clipRange))));
    y = shaderMtf(f(t.midtone), x);
  } else if (t.stretchKind === 1) {
    y = n;
  } else if (t.stretchKind === 2) {
    y = f(f(Math.log(f(f(1000 * n) + 1))) / f(Math.log(1001)));
  } else if (t.stretchKind === 3) {
    y = f(Math.sqrt(n));
  } else if (t.stretchKind === 4) {
    const a = t.asinhA > 0 ? f(t.asinhA) : f(0.1);
    y = f(f(Math.asinh(f(n / a))) / f(Math.asinh(f(1 / a))));
  } else {
    const p = t.power > 0 ? f(t.power) : 2;
    y = f(Math.pow(n, p));
  }
  y = f(Math.min(1, Math.max(0, y)));
  let idx = Math.floor(f(f(y * 255) + 0.5));
  if (t.invert) idx = 255 - idx;
  return idx;
}

describe("normalize", () => {
  it("maps the range to [0,1] and clamps", () => {
    expect(normalize(5, 0, 10)).toBe(0.5);
    expect(normalize(-1, 0, 10)).toBe(0);
    expect(normalize(11, 0, 10)).toBe(1);
  });
  it("returns 0 when vmax <= vmin", () => {
    expect(normalize(5, 10, 10)).toBe(0);
    expect(normalize(5, 10, 0)).toBe(0);
  });
  it("passes NaN through", () => {
    expect(normalize(NaN, 0, 1)).toBeNaN();
  });
  it("returns an f32-representable value", () => {
    const n = normalize(1, 0, 3);
    expect(Math.fround(n)).toBe(n);
  });
});

describe("golden stretch bytes", () => {
  for (const name of Object.keys(GOLDEN) as (keyof typeof GOLDEN)[]) {
    it(`${name} matches the shared golden vector`, () => {
      const t = goldenKind(STRETCH_KIND[name]);
      const bytes = GOLDEN_RAW.map((x) => transferByte(x, t));
      expect(bytes).toEqual(GOLDEN[name]);
    });
    it(`${name} inverted equals 255 - byte`, () => {
      const t = goldenKind(STRETCH_KIND[name], true);
      const bytes = GOLDEN_RAW.map((x) => transferByte(x, t));
      expect(bytes).toEqual(GOLDEN[name].map((b) => 255 - b));
    });
  }
});

describe("non-finite pixels", () => {
  const values = [NaN, Infinity, -Infinity];
  for (const kind of [0, 1, 2, 3, 4, 5] as const) {
    it(`kind ${kind} maps NaN/Inf to the no-data index regardless of invert`, () => {
      for (const v of values) {
        expect(transferByte(v, withKind(kind))).toBe(NODATA_INDEX);
        expect(transferByte(v, withKind(kind, true))).toBe(NODATA_INDEX);
      }
    });
  }
  it("paints padding below vmin and NaN with the same byte, inverted or not", () => {
    for (const kind of [0, 1, 2, 3, 4, 5] as const) {
      for (const invert of [false, true]) {
        const t: DisplayTransfer = { ...BASE, vmin: 0.5, vmax: 900, stretchKind: kind, invert, shadow: 0.01, midtone: 0.2 };
        expect(transferByte(NaN, t)).toBe(transferByte(0, t));
        expect(shaderByte(NaN, t)).toBe(shaderByte(0, t));
      }
    }
  });
  it("treats exact zero as padding when the valid minimum is negative", () => {
    for (const kind of [0, 1, 2, 3, 4, 5] as const) {
      for (const invert of [false, true]) {
        const t: DisplayTransfer = { ...BASE, vmin: -3.5, vmax: 1200.25, stretchKind: kind, invert, shadow: 0, midtone: 0.5 };
        const noData = NODATA_INDEX;
        for (const v of [0, -0]) {
          expect(transferByte(v, t)).toBe(noData);
          expect(shaderByte(v, t)).toBe(noData);
        }
      }
    }
  });
  it("keeps tiny non-zero and negative values as data", () => {
    const t: DisplayTransfer = { ...BASE, vmin: -3.5, vmax: 1200.25, stretchKind: 1 };
    expect(transferByte(1e-30, t)).toBe(1);
    expect(transferByte(-1e-30, t)).toBe(1);
    expect(transferByte(-3.5, t)).toBe(0);
    expect(transferByte(-3.5, { ...t, invert: true })).toBe(255);
    expect(isPaddingValue(-2)).toBe(false);
    expect(isPaddingValue(1e-30)).toBe(false);
    for (const v of [0, -0, NaN, Infinity, -Infinity]) expect(isPaddingValue(v)).toBe(true);
  });
});

describe("MTF kind", () => {
  it("reproduces the previous worker stfTransfer on a fixed vector", () => {
    const dataMin = 120;
    const dataMax = 4200;
    const shadow = 0.02;
    const midtone = 0.18;
    const highlight = 0.95;
    const t: DisplayTransfer = { ...BASE, stretchKind: 0, vmin: dataMin, vmax: dataMax, shadow, midtone, highlight };
    const invRange = 1 / Math.max(dataMax - dataMin, 1e-8);
    const clipRange = Math.max(highlight - shadow, 1e-8);
    const raw = [120, 150, 300, 512, 777, 1024, 1500, 2048, 3000, 3999, 4200, NaN];
    const expected = raw.map((v) =>
      Number.isFinite(v) ? (oldStfTransfer(v, invRange, dataMin, shadow, clipRange, midtone) * 255 + 0.5) | 0 : NODATA_INDEX,
    );
    expect(raw.map((v) => transferByte(v, t))).toEqual(expected);
  });
  it("is identity for midtone 0.5 with full range", () => {
    const t = goldenKind(0);
    expect(GOLDEN_RAW.map((x) => transferByte(x, t))).toEqual(GOLDEN.linear);
  });
});

describe("lutIndex", () => {
  it("rounds half up", () => {
    expect(lutIndex(0.5, false)).toBe(128);
  });
  it("inverts", () => {
    expect(lutIndex(1, true)).toBe(0);
    expect(lutIndex(0, true)).toBe(255);
  });
  it("maps NaN to index 0 and then applies the inversion", () => {
    expect(lutIndex(NaN, false)).toBe(0);
    expect(lutIndex(NaN, true)).toBe(255);
  });
});

describe("stretchValue", () => {
  it("returns f32 values inside [0,1]", () => {
    for (const kind of [0, 1, 2, 3, 4, 5] as const) {
      for (const x of [-0.5, 0, 0.3, 1, 1.5]) {
        const y = stretchValue(x, withKind(kind));
        expect(y).toBeGreaterThanOrEqual(0);
        expect(y).toBeLessThanOrEqual(1);
        expect(Math.fround(y)).toBe(y);
      }
    }
  });
  it("falls back to the defaults for non-positive asinh_a and power", () => {
    expect(stretchValue(0.25, { ...BASE, stretchKind: 4, asinhA: 0 })).toBe(stretchValue(0.25, { ...BASE, stretchKind: 4, asinhA: 0.1 }));
    expect(stretchValue(0.25, { ...BASE, stretchKind: 5, power: -1 })).toBe(stretchValue(0.25, { ...BASE, stretchKind: 5, power: 2 }));
  });
});

describe("renderRgba", () => {
  it("writes r,g,b from the LUT row and alpha 255", () => {
    const lut = new Uint8Array(1024);
    for (let i = 0; i < 256; i++) {
      lut[i * 4] = i;
      lut[i * 4 + 1] = 255 - i;
      lut[i * 4 + 2] = (i * 7) & 255;
      lut[i * 4 + 3] = 17;
    }
    const pixels = new Float32Array([0, 0.5, NaN]);
    const out = new Uint8ClampedArray(12);
    renderRgba(pixels, withKind(1), lut, out);
    expect(Array.from(out)).toEqual([
      0, 255, 0, 255,
      128, 127, (128 * 7) & 255, 255,
      0, 255, 0, 255,
    ]);
  });
  it("paints padding with the trailing no-data entry and never inverts it", () => {
    const lut = new Uint8Array(LUT_WITH_NODATA_BYTES);
    for (let i = 0; i < 256; i++) {
      lut[i * 4] = i;
      lut[i * 4 + 1] = 255 - i;
      lut[i * 4 + 2] = 9;
      lut[i * 4 + 3] = 255;
    }
    lut.set([64, 64, 64, 255], 1024);
    const pixels = new Float32Array([0, 0.5, NaN]);
    const out = new Uint8ClampedArray(12);
    renderRgba(pixels, withKind(1), lut, out);
    expect(Array.from(out)).toEqual([
      64, 64, 64, 255,
      128, 127, 9, 255,
      64, 64, 64, 255,
    ]);
    renderRgba(pixels, withKind(1, true), lut, out);
    expect(Array.from(out)).toEqual([
      64, 64, 64, 255,
      127, 128, 9, 255,
      64, 64, 64, 255,
    ]);
  });
  it("lutNodataRgb falls back to the first entry for a bare LUT", () => {
    expect(lutNodataRgb(GRAY_LUT_RGBA)).toEqual([0, 0, 0]);
    const bare = new Uint8Array(1024);
    bare.set([7, 8, 9, 255], 0);
    expect(lutNodataRgb(bare)).toEqual([7, 8, 9]);
    const withTail = new Uint8Array(LUT_WITH_NODATA_BYTES);
    withTail.set([7, 8, 9, 255], 0);
    withTail.set([64, 65, 66, 255], 1024);
    expect(lutNodataRgb(withTail)).toEqual([64, 65, 66]);
  });
  it("withNodataEntry builds 1028 bytes with alpha 255", () => {
    const rgba = Array.from({ length: 1024 }, (_, i) => i & 255);
    const lut = withNodataEntry(rgba, [64, 64, 64]);
    expect(lut.length).toBe(LUT_WITH_NODATA_BYTES);
    expect(Array.from(lut.subarray(0, 8))).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
    expect(Array.from(lut.subarray(1020, 1024))).toEqual([252, 253, 254, 255]);
    expect(Array.from(lut.subarray(1024))).toEqual([64, 64, 64, 255]);
  });
});

describe("reconcileDisplayPatch", () => {
  const linear = { ...DEFAULT_DISPLAY_SETTINGS, stretch: "linear" as const };
  it("enabling symmetric under mtf switches the stretch to linear", () => {
    expect(DEFAULT_DISPLAY_SETTINGS.stretch).toBe("mtf");
    const next = reconcileDisplayPatch(DEFAULT_DISPLAY_SETTINGS, { symmetric: true });
    expect(next).toMatchObject({ symmetric: true, stretch: "linear" });
  });
  it("selecting mtf turns symmetric off", () => {
    const next = reconcileDisplayPatch({ ...linear, symmetric: true }, { stretch: "mtf" });
    expect(next).toMatchObject({ symmetric: false, stretch: "mtf" });
  });
  it("a patch carrying both symmetric and mtf keeps mtf and drops symmetric", () => {
    const next = reconcileDisplayPatch(linear, { symmetric: true, stretch: "mtf" });
    expect(next).toMatchObject({ symmetric: false, stretch: "mtf" });
  });
  it("passes a plain patch through under a non-mtf stretch", () => {
    const next = reconcileDisplayPatch(linear, { symmetric: true, centre: 0.25, colormap: "bwr" });
    expect(next).toEqual({ ...linear, symmetric: true, centre: 0.25, colormap: "bwr" });
  });
  it("the Reset patch restores the defaults from a symmetric state", () => {
    const next = reconcileDisplayPatch({ ...linear, symmetric: true, centre: 3 }, DEFAULT_DISPLAY_SETTINGS);
    expect(next).toEqual(DEFAULT_DISPLAY_SETTINGS);
    expect(next.symmetric).toBe(false);
    expect(next.centre).toBe(0);
  });
  it("sanitises a stored symmetric flag under the default mtf stretch to symmetric off", () => {
    const next = reconcileDisplayPatch({ ...DEFAULT_DISPLAY_SETTINGS, symmetric: true }, {});
    expect(next).toMatchObject({ symmetric: false, stretch: "mtf" });
  });
});

describe("symmetricStretchNote", () => {
  it("is silent under a linear stretch and whenever symmetric is off", () => {
    expect(symmetricStretchNote({ symmetric: true, stretch: "linear" })).toBeNull();
    for (const stretch of ["mtf", "linear", "log", "sqrt", "asinh", "power"] as const) {
      expect(symmetricStretchNote({ symmetric: false, stretch })).toBeNull();
    }
  });
  it("says the centre leaves the colormap centre under every non-linear stretch", () => {
    for (const stretch of ["log", "sqrt", "asinh", "power"] as const) {
      expect(symmetricStretchNote({ symmetric: true, stretch })).toBe(SYMMETRIC_NONLINEAR_STRETCH_NOTE);
    }
    expect(SYMMETRIC_NONLINEAR_STRETCH_NOTE).toContain("linear stretch");
  });
  it("names the byte the centre gets: a normalised 0.5 stretched by log rounds to 229, not 128", () => {
    const centre = { ...BASE, vmin: -1, vmax: 1 };
    expect(transferByte(1e-9, { ...centre, stretchKind: STRETCH_KIND.linear })).toBe(128);
    expect(transferByte(1e-9, { ...centre, stretchKind: STRETCH_KIND.log })).toBe(229);
    expect(transferByte(1e-9, { ...centre, stretchKind: STRETCH_KIND.sqrt })).toBe(180);
    expect(transferByte(1e-9, { ...centre, stretchKind: STRETCH_KIND.asinh })).toBe(197);
  });
});

describe("centre draft", () => {
  it("keeps a partial, empty or non-finite entry out of the settings", () => {
    for (const text of ["", " ", "-", "1e", "1e-", "-.", "abc", "Infinity", "NaN"]) {
      expect(parseCentreDraft(text), JSON.stringify(text)).toBeNull();
    }
  });
  it("parses signed, fractional and exponent entries", () => {
    expect(parseCentreDraft("-300")).toBe(-300);
    expect(parseCentreDraft("1e-5")).toBe(1e-5);
    expect(parseCentreDraft("-3.05")).toBe(-3.05);
    expect(parseCentreDraft("0")).toBe(0);
    expect(parseCentreDraft(" 2.5 ")).toBe(2.5);
  });
  it("follows an external centre only when the draft does not already spell it", () => {
    expect(centreDraftFor("-3.0", -3)).toBe("-3.0");
    expect(centreDraftFor("1e-5", 0.00001)).toBe("1e-5");
    expect(centreDraftFor("-300", 0)).toBe("0");
    expect(centreDraftFor("", 0)).toBe("0");
    expect(centreDraftFor("-", -300)).toBe("-300");
    expect(centreDraftFor("5", 5)).toBe("5");
  });
});

describe("GRAY_LUT_RGBA", () => {
  it("is the identity ramp with opaque alpha", () => {
    expect(GRAY_LUT_RGBA.length).toBe(1024);
    for (let i = 0; i < 256; i++) {
      expect(GRAY_LUT_RGBA[4 * i]).toBe(i);
      expect(GRAY_LUT_RGBA[4 * i + 1]).toBe(i);
      expect(GRAY_LUT_RGBA[4 * i + 2]).toBe(i);
      expect(GRAY_LUT_RGBA[4 * i + 3]).toBe(255);
    }
  });
});

describe("toDisplayTransfer", () => {
  it("maps stretch names to GPU codes and copies the defaults", () => {
    const stf = { shadow: 0.1, midtone: 0.3, highlight: 0.9 };
    const t = toDisplayTransfer(DEFAULT_DISPLAY_SETTINGS, stf, { vmin: 2, vmax: 9 });
    expect(t).toEqual({
      vmin: 2,
      vmax: 9,
      shadow: 0.1,
      midtone: 0.3,
      highlight: 0.9,
      stretchKind: 0,
      asinhA: 0.1,
      power: 2,
      invert: false,
    });
    expect(toDisplayTransfer({ ...DEFAULT_DISPLAY_SETTINGS, stretch: "linear" }, stf, { vmin: 0, vmax: 1 }).stretchKind).toBe(1);
    expect(toDisplayTransfer({ ...DEFAULT_DISPLAY_SETTINGS, stretch: "log" }, stf, { vmin: 0, vmax: 1 }).stretchKind).toBe(2);
    expect(toDisplayTransfer({ ...DEFAULT_DISPLAY_SETTINGS, stretch: "sqrt" }, stf, { vmin: 0, vmax: 1 }).stretchKind).toBe(3);
    expect(toDisplayTransfer({ ...DEFAULT_DISPLAY_SETTINGS, stretch: "asinh" }, stf, { vmin: 0, vmax: 1 }).stretchKind).toBe(4);
    expect(toDisplayTransfer({ ...DEFAULT_DISPLAY_SETTINGS, stretch: "power", invert: true }, stf, { vmin: 0, vmax: 1 })).toMatchObject({ stretchKind: 5, invert: true });
  });
});

describe("resolveTransferLimits", () => {
  const raw = { min: 0.02, max: 60000 };
  const masked = { data_min: 0.02, data_max: 900 };
  it("normalises mtf against the histogram range of the displayed image, not the unmasked raw extrema", () => {
    expect(resolveTransferLimits("mtf", null, raw, masked)).toEqual({ vmin: 0.02, vmax: 900 });
  });
  it("falls back to the raw extrema when no histogram matches the displayed image", () => {
    expect(resolveTransferLimits("mtf", null, raw, null)).toEqual({ vmin: 0.02, vmax: 60000 });
    expect(resolveTransferLimits("mtf", null, null, null)).toEqual({ vmin: 0, vmax: 1 });
  });
  it("ignores a degenerate or non-finite histogram range", () => {
    expect(resolveTransferLimits("mtf", null, raw, { data_min: 5, data_max: 5 })).toEqual({ vmin: 0.02, vmax: 60000 });
    expect(resolveTransferLimits("mtf", null, raw, { data_min: NaN, data_max: 5 })).toEqual({ vmin: 0.02, vmax: 60000 });
  });
  it("uses the scale limits for every other stretch, with the data range as fallback", () => {
    expect(resolveTransferLimits("linear", { vmin: 1, vmax: 2 }, raw, masked)).toEqual({ vmin: 1, vmax: 2 });
    expect(resolveTransferLimits("asinh", null, raw, masked)).toEqual({ vmin: 0.02, vmax: 900 });
  });
});

describe("WGSL fragment port parity", () => {
  it("matches transferByte on padding pixels with and without invert", () => {
    for (const kind of [0, 1, 2, 3, 4, 5] as const) {
      for (const invert of [false, true]) {
        for (const v of [NaN, Infinity, -Infinity, 0, -0]) {
          expect(shaderByte(v, withKind(kind, invert))).toBe(transferByte(v, withKind(kind, invert)));
        }
      }
    }
  });
  it("matches the CPU chain byte-exactly on the golden vectors", () => {
    for (const name of Object.keys(GOLDEN) as (keyof typeof GOLDEN)[]) {
      const t = goldenKind(STRETCH_KIND[name]);
      expect(GOLDEN_RAW.map((x) => shaderByte(x, t))).toEqual(GOLDEN[name]);
      const inv = goldenKind(STRETCH_KIND[name], true);
      expect(GOLDEN_RAW.map((x) => shaderByte(x, inv))).toEqual(GOLDEN[name].map((b) => 255 - b));
    }
  });
  it("agrees with transferByte within f32 rounding across a sweep for every kind", () => {
    const n = 2001;
    const settings = [
      { vmin: 0, vmax: 1 },
      { vmin: -3.5, vmax: 1200.25 },
      { vmin: 10, vmax: 10 },
    ];
    for (const kind of [0, 1, 2, 3, 4, 5] as const) {
      for (const invert of [false, true]) {
        for (const lim of settings) {
          const t: DisplayTransfer = { ...BASE, ...lim, stretchKind: kind, invert, shadow: 0.05, midtone: 0.25, highlight: 0.98 };
          let offByOne = 0;
          for (let i = 0; i < n; i++) {
            const raw = lim.vmin - 0.1 * (lim.vmax - lim.vmin) + ((lim.vmax - lim.vmin) * 1.2 * i) / (n - 1);
            const cpu = transferByte(raw, t);
            const gpu = shaderByte(raw, t);
            const diff = Math.abs(cpu - gpu);
            expect(diff).toBeLessThanOrEqual(1);
            if (diff === 1) offByOne++;
          }
          expect(offByOne).toBeLessThanOrEqual(3);
        }
      }
    }
  });
});
