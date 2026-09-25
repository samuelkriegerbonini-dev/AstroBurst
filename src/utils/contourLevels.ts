import type { Pt } from "./regionGeometry";
import type { ContourLevel, ContourMode, ContourOptions } from "../shared/types/contours";

export type ContourColourMode = "single" | "ramp";

export const MIN_CONTOUR_BIN = 1;
export const MAX_CONTOUR_BIN = 8;
export const DEFAULT_BIN_TARGET_DIM = 2048;
export const DEFAULT_POLYGON_MAX_VERTICES = 512;
export const SINGLE_CONTOUR_COLOUR = "#14b8a6";
const MIN_POLYGON_VERTICES = 3;
const RAMP_HUE_START = 220;
const RAMP_HUE_END = 40;
const RAMP_SATURATION = 90;
const RAMP_LIGHTNESS = 60;
const LEVEL_SEPARATOR = /[\s,;]+/;

function parseNumberList(text: string): number[] | null {
  const tokens = text
    .split(LEVEL_SEPARATOR)
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
  if (tokens.length === 0) return null;
  const values: number[] = [];
  for (const token of tokens) {
    const v = Number(token);
    if (!Number.isFinite(v)) return null;
    values.push(v);
  }
  return values;
}

export function parseLevelList(text: string): number[] | null {
  return parseNumberList(text);
}

export function parseSigmaMultiples(text: string): number[] | null {
  return parseNumberList(text);
}

export function suggestBin(width: number, height: number, targetMaxDim = DEFAULT_BIN_TARGET_DIM): number {
  const maxDim = Math.max(width, height);
  if (!Number.isFinite(maxDim) || maxDim <= 0 || !Number.isFinite(targetMaxDim) || targetMaxDim <= 0) {
    return MIN_CONTOUR_BIN;
  }
  const bin = Math.ceil(maxDim / targetMaxDim);
  return Math.min(MAX_CONTOUR_BIN, Math.max(MIN_CONTOUR_BIN, bin));
}

export function levelColour(index: number, n: number, mode: ContourColourMode): string {
  if (mode === "single") return SINGLE_CONTOUR_COLOUR;
  const t = n <= 1 ? 0 : Math.min(1, Math.max(0, index / (n - 1)));
  const hue = Math.round(RAMP_HUE_START + (RAMP_HUE_END - RAMP_HUE_START) * t);
  return `hsl(${hue} ${RAMP_SATURATION}% ${RAMP_LIGHTNESS}%)`;
}

export function cullPolyline(
  points: ReadonlyArray<readonly [number, number]>,
  toScreen: (p: Pt) => Pt,
  width: number,
  height: number,
): boolean {
  if (points.length === 0) return true;
  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  for (const [x, y] of points) {
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (y < minY) minY = y;
    if (y > maxY) maxY = y;
  }
  const corners = [
    toScreen({ x: minX, y: minY }),
    toScreen({ x: maxX, y: minY }),
    toScreen({ x: minX, y: maxY }),
    toScreen({ x: maxX, y: maxY }),
  ];
  let sMinX = Number.POSITIVE_INFINITY;
  let sMinY = Number.POSITIVE_INFINITY;
  let sMaxX = Number.NEGATIVE_INFINITY;
  let sMaxY = Number.NEGATIVE_INFINITY;
  for (const c of corners) {
    if (c.x < sMinX) sMinX = c.x;
    if (c.x > sMaxX) sMaxX = c.x;
    if (c.y < sMinY) sMinY = c.y;
    if (c.y > sMaxY) sMaxY = c.y;
  }
  return sMaxX < 0 || sMaxY < 0 || sMinX > width || sMinY > height;
}

export function contourToPolygon(
  points: [number, number][],
  maxVertices = DEFAULT_POLYGON_MAX_VERTICES,
): [number, number][] | null {
  if (points.length < MIN_POLYGON_VERTICES) return null;
  const limit = Math.max(MIN_POLYGON_VERTICES, Math.floor(maxVertices));
  if (points.length <= limit) return points.map(([x, y]) => [x, y]);
  const stride = Math.ceil(points.length / limit);
  const out: [number, number][] = [];
  for (let i = 0; i < points.length; i += stride) {
    out.push([points[i][0], points[i][1]]);
  }
  return out.length >= MIN_POLYGON_VERTICES ? out : null;
}

export function levelsText(values: readonly number[]): string {
  return values.map((v) => String(v)).join(", ");
}

export function formatLevelValue(value: number): string {
  if (!Number.isFinite(value)) return "--";
  const magnitude = Math.abs(value);
  if (magnitude === 0) return "0";
  if (magnitude >= 1e5 || magnitude < 1e-3) return value.toExponential(3);
  return value.toPrecision(5).replace(/\.?0+$/, "");
}

export type ContourBinChoice = "auto" | "1" | "2" | "4" | "8";

export const CONTOUR_BIN_CHOICES: readonly ContourBinChoice[] = ["auto", "1", "2", "4", "8"];
export const MAX_REGION_POLYGONS = 200;
export const MAX_SMOOTH_SIGMA_PX = 20;

export interface ContourForm {
  mode: ContourMode;
  levelsText: string;
  nLevelsText: string;
  loText: string;
  hiText: string;
  sigmaText: string;
  smoothText: string;
  binChoice: ContourBinChoice;
  imageWidth: number;
  imageHeight: number;
  excludeDq: boolean;
}

export type ContourRequest = { ok: true; options: ContourOptions } | { ok: false; error: string };

function parseFiniteNumber(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed.length === 0) return null;
  const v = Number(trimmed);
  return Number.isFinite(v) ? v : null;
}

export function resolveBin(choice: ContourBinChoice, imageWidth: number, imageHeight: number): number {
  if (choice === "auto") return suggestBin(imageWidth, imageHeight);
  const parsed = Number(choice);
  return Number.isFinite(parsed) ? Math.min(MAX_CONTOUR_BIN, Math.max(MIN_CONTOUR_BIN, Math.round(parsed))) : MIN_CONTOUR_BIN;
}

export function buildContourRequest(form: ContourForm): ContourRequest {
  const smooth = parseFiniteNumber(form.smoothText);
  if (smooth === null || smooth < 0 || smooth > MAX_SMOOTH_SIGMA_PX) {
    return { ok: false, error: `Smoothing sigma must be a number between 0 and ${MAX_SMOOTH_SIGMA_PX} pixels.` };
  }
  const bin = resolveBin(form.binChoice, form.imageWidth, form.imageHeight);
  const base: ContourOptions = { mode: form.mode, smoothSigma: smooth, bin, excludeDq: form.excludeDq };
  if (form.mode === "list") {
    const levels = parseLevelList(form.levelsText);
    if (!levels) return { ok: false, error: "Levels must be a comma or space separated list of numbers." };
    return { ok: true, options: { ...base, levels } };
  }
  if (form.mode === "sigma") {
    const sigmaMultiples = parseSigmaMultiples(form.sigmaText);
    if (!sigmaMultiples) return { ok: false, error: "Sigma multiples must be a comma or space separated list of numbers." };
    return { ok: true, options: { ...base, sigmaMultiples } };
  }
  const nLevels = parseFiniteNumber(form.nLevelsText);
  if (nLevels === null || !Number.isInteger(nLevels) || nLevels < 1) {
    return { ok: false, error: "The number of levels must be a whole number of at least 1." };
  }
  const lo = parseFiniteNumber(form.loText);
  const hi = parseFiniteNumber(form.hiText);
  if (lo === null || hi === null) return { ok: false, error: "Low and high must both be numbers." };
  if (lo >= hi) return { ok: false, error: "Low must be less than high." };
  if (form.mode === "log" && lo <= 0) return { ok: false, error: "Low must be greater than 0 for log levels." };
  if (form.mode === "sqrt" && lo < 0) return { ok: false, error: "Low must be at least 0 for sqrt levels." };
  return { ok: true, options: { ...base, nLevels, lo, hi } };
}

export interface ContourPolygon {
  value: number;
  points: [number, number][];
}

export interface ContourPolygonPick {
  polygons: ContourPolygon[];
  skipped: number;
}

export function closedContourPolygons(
  levels: readonly ContourLevel[],
  hidden: ReadonlySet<number>,
  maxPolygons = MAX_REGION_POLYGONS,
): ContourPolygonPick {
  const polygons: ContourPolygon[] = [];
  let skipped = 0;
  levels.forEach((level, li) => {
    if (hidden.has(li)) return;
    level.polylines.forEach((polyline, pi) => {
      if (!level.closed[pi]) return;
      const points = contourToPolygon(polyline);
      if (!points) return;
      if (polygons.length >= maxPolygons) {
        skipped++;
        return;
      }
      polygons.push({ value: level.value, points });
    });
  });
  return { polygons, skipped };
}
