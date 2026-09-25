import type { DisplaySettings, StretchMode } from "../shared/types/display";
import type { StfParams } from "../shared/types/fits.types";

export const STRETCH_KIND = { mtf: 0, linear: 1, log: 2, sqrt: 3, asinh: 4, power: 5 } as const;

export type StretchKindCode = (typeof STRETCH_KIND)[StretchMode];

export interface DisplayTransfer {
  vmin: number;
  vmax: number;
  shadow: number;
  midtone: number;
  highlight: number;
  stretchKind: StretchKindCode;
  asinhA: number;
  power: number;
  invert: boolean;
}

export const LUT_BYTES = 1024;
export const LUT_NODATA_OFFSET = 1024;
export const LUT_WITH_NODATA_BYTES = 1028;
export const NODATA_INDEX = -1;

export function withNodataEntry(rgba: ArrayLike<number>, nodata: ArrayLike<number>): Uint8Array {
  const out = new Uint8Array(LUT_WITH_NODATA_BYTES);
  const n = Math.min(rgba.length, LUT_BYTES);
  for (let i = 0; i < n; i++) out[i] = rgba[i];
  for (let c = 0; c < 3; c++) out[LUT_NODATA_OFFSET + c] = nodata[c];
  out[LUT_NODATA_OFFSET + 3] = 255;
  return out;
}

export function lutNodataRgb(lut: Uint8Array): [number, number, number] {
  if (lut.length >= LUT_WITH_NODATA_BYTES) {
    return [lut[LUT_NODATA_OFFSET], lut[LUT_NODATA_OFFSET + 1], lut[LUT_NODATA_OFFSET + 2]];
  }
  return [lut[0], lut[1], lut[2]];
}

function buildGrayLut(): Uint8Array {
  const lut = new Uint8Array(LUT_BYTES);
  for (let i = 0; i < 256; i++) {
    const off = i * 4;
    lut[off] = i;
    lut[off + 1] = i;
    lut[off + 2] = i;
    lut[off + 3] = 255;
  }
  return lut;
}

export const GRAY_LUT_RGBA: Uint8Array = buildGrayLut();

const LN_1001 = Math.log(1001);

function clamp01(x: number): number {
  return x < 0 ? 0 : x > 1 ? 1 : x;
}

export function normalize(v: number, vmin: number, vmax: number): number {
  if (v !== v) return NaN;
  const range = vmax - vmin;
  if (!(range > 0)) return 0;
  return Math.fround(clamp01((v - vmin) / range));
}

function mtf(m: number, x: number): number {
  if (x <= 0) return 0;
  if (x >= 1) return 1;
  if (Math.abs(m - 0.5) < 1e-6) return x;
  const b = (2 * m - 1) * x - m;
  if (Math.abs(b) < 1e-8) return x;
  return ((m - 1) * x) / b;
}

export function stretchValue(n: number, t: DisplayTransfer): number {
  if (n !== n) return NaN;
  const x = clamp01(n);
  let y: number;
  switch (t.stretchKind) {
    case 0: {
      const clipRange = Math.max(t.highlight - t.shadow, 1e-8);
      y = mtf(t.midtone, clamp01((x - t.shadow) / clipRange));
      break;
    }
    case 1:
      y = x;
      break;
    case 2:
      y = Math.log(1000 * x + 1) / LN_1001;
      break;
    case 3:
      y = Math.sqrt(x);
      break;
    case 4: {
      const a = t.asinhA > 0 ? t.asinhA : 0.1;
      y = Math.asinh(x / a) / Math.asinh(1 / a);
      break;
    }
    case 5: {
      const p = t.power > 0 ? t.power : 2.0;
      y = Math.pow(x, p);
      break;
    }
    default:
      y = x;
  }
  return Math.fround(clamp01(y));
}

export function lutIndex(y: number, invert: boolean): number {
  const idx = y !== y ? 0 : Math.floor(clamp01(Math.fround(y)) * 255 + 0.5);
  return invert ? 255 - idx : idx;
}

export function isPaddingValue(raw: number): boolean {
  return !Number.isFinite(raw) || raw === 0;
}

export function transferByte(raw: number, t: DisplayTransfer): number {
  if (isPaddingValue(raw)) return NODATA_INDEX;
  return lutIndex(stretchValue(normalize(raw, t.vmin, t.vmax), t), t.invert);
}

export interface TransferLimits {
  vmin: number;
  vmax: number;
}

export function resolveTransferLimits(
  stretch: StretchMode,
  scaleLimits: TransferLimits | null,
  rawRange: { min: number; max: number } | null,
  histRange: { data_min: number; data_max: number } | null,
): TransferLimits {
  const histUsable =
    histRange !== null &&
    Number.isFinite(histRange.data_min) &&
    Number.isFinite(histRange.data_max) &&
    histRange.data_max > histRange.data_min;
  const data = histUsable
    ? { vmin: histRange.data_min, vmax: histRange.data_max }
    : { vmin: rawRange?.min ?? 0, vmax: rawRange?.max ?? 1 };
  if (stretch === "mtf") return data;
  return scaleLimits ?? data;
}

export function renderRgba(pixels: Float32Array, t: DisplayTransfer, lut: Uint8Array, out: Uint8ClampedArray): void {
  const [nr, ng, nb] = lutNodataRgb(lut);
  const len = pixels.length;
  for (let i = 0; i < len; i++) {
    const idx = transferByte(pixels[i], t);
    const off = i * 4;
    if (idx < 0) {
      out[off] = nr;
      out[off + 1] = ng;
      out[off + 2] = nb;
    } else {
      const src = idx * 4;
      out[off] = lut[src];
      out[off + 1] = lut[src + 1];
      out[off + 2] = lut[src + 2];
    }
    out[off + 3] = 255;
  }
}

export function toDisplayTransfer(
  settings: DisplaySettings,
  stf: StfParams,
  limits: { vmin: number; vmax: number },
): DisplayTransfer {
  return {
    vmin: limits.vmin,
    vmax: limits.vmax,
    shadow: stf.shadow,
    midtone: stf.midtone,
    highlight: stf.highlight,
    stretchKind: STRETCH_KIND[settings.stretch] ?? 0,
    asinhA: settings.asinhA,
    power: settings.power,
    invert: settings.invert,
  };
}

export function reconcileDisplayPatch(prev: DisplaySettings, patch: Partial<DisplaySettings>): DisplaySettings {
  const next = { ...prev, ...patch };
  if (next.symmetric && next.stretch === "mtf") {
    if (patch.symmetric === true && patch.stretch === undefined) {
      next.stretch = "linear";
    } else {
      next.symmetric = false;
    }
  }
  return next;
}

export const SYMMETRIC_NONLINEAR_STRETCH_NOTE = "the centre maps to the colormap centre only under a linear stretch";

export function symmetricStretchNote(settings: Pick<DisplaySettings, "symmetric" | "stretch">): string | null {
  return settings.symmetric && settings.stretch !== "linear" ? SYMMETRIC_NONLINEAR_STRETCH_NOTE : null;
}

export function parseCentreDraft(text: string): number | null {
  if (text.trim() === "") return null;
  const n = Number(text);
  return Number.isFinite(n) ? n : null;
}

export function centreDraftFor(draft: string, centre: number): string {
  return parseCentreDraft(draft) === centre ? draft : String(centre);
}
