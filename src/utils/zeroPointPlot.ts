import type { CrossMatchResult, GaiaBand, MatchedCatalogRow, ZeroPointStatus } from "../shared/types/catalog";

export interface ZeroPointSplit {
  fitted: { x: number[]; y: number[] };
  outliers: { x: number[]; y: number[] };
  saturated: { x: number[]; y: number[] };
  noColour: { x: number[]; y: number[] };
  line: { x: number[]; y: number[] } | null;
}

export function splitZeroPointPoints(
  catMag: (number | null)[],
  magInst: (number | null)[],
  zp: number | null,
  status: (ZeroPointStatus | null)[],
  colour: (number | null)[] = [],
  colourCoeff: number | null = null,
): ZeroPointSplit {
  const fitted = { x: [] as number[], y: [] as number[] };
  const outliers = { x: [] as number[], y: [] as number[] };
  const saturated = { x: [] as number[], y: [] as number[] };
  const noColour = { x: [] as number[], y: [] as number[] };
  const groups = { used: fitted, clipped: outliers, saturated, no_colour: noColour };
  const fit = zp !== null && Number.isFinite(zp) ? zp : null;
  const coeff = colourCoeff !== null && Number.isFinite(colourCoeff) ? colourCoeff : null;
  const n = Math.min(catMag.length, magInst.length);
  for (let i = 0; i < n; i++) {
    const x = catMag[i];
    const raw = magInst[i];
    const entered = status[i] ?? null;
    if (entered === null || x === null || raw === null || !Number.isFinite(x) || !Number.isFinite(raw)) continue;
    const c = colour[i] ?? null;
    const corrected = entered !== "no_colour" && coeff !== null && c !== null && Number.isFinite(c);
    groups[entered].x.push(x);
    groups[entered].y.push(corrected ? raw + coeff * c : raw);
  }
  const xs = fitted.x.concat(outliers.x);
  const ext = finiteExtentOf(xs);
  const line = fit !== null && ext ? { x: [ext[0], ext[1]], y: [ext[0] - fit, ext[1] - fit] } : null;
  return { fitted, outliers, saturated, noColour, line };
}

function finiteExtentOf(values: number[]): [number, number] | null {
  let lo = Infinity;
  let hi = -Infinity;
  for (const v of values) {
    if (!Number.isFinite(v)) continue;
    if (v < lo) lo = v;
    if (v > hi) hi = v;
  }
  return lo <= hi ? [lo, hi] : null;
}

export function catalogueMagnitude(row: MatchedCatalogRow, band: GaiaBand): number | null {
  if (band === "BP") return row.bp;
  if (band === "RP") return row.rp;
  return row.g;
}

export function zeroPointSplitOf(cross: CrossMatchResult): ZeroPointSplit {
  const zp = cross.zero_point;
  return splitZeroPointPoints(
    cross.matches.map((m) => catalogueMagnitude(m.row, cross.band)),
    cross.matches.map((m) => m.star.mag_inst),
    zp ? zp.zp : null,
    cross.matches.map((m) => m.zp_status),
    cross.matches.map((m) => m.row.bp_rp),
    zp ? zp.colour_coeff : null,
  );
}
