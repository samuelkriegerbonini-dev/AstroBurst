import { parseSkyCsv } from "./skyList";
import {
  FORCED_MAX_BANDS,
  FORCED_MAX_POSITIONS,
  type ForcedAperture,
  type ForcedBand,
  type ForcedBandInput,
  type ForcedCell,
  type ForcedPhotometryRequest,
  type ForcedPhotometryResult,
  type ForcedRunOptions,
  type ForcedSkyPosition,
} from "../shared/types/forcedPhotometry";

export const DEFAULT_LIMIT_SIGMA = 3;
export const NOISE_FACTOR_RADIUS_TOLERANCE = 0.01;

const RA_DECIMALS = 12;
const DEG_PER_TURN = 360;
const CSV_UNSAFE_ID_CHARACTERS = /[,#"\t\r\n]/g;

export type ForcedRadiusUnit = "arcsec" | "px";

export interface ForcedZeroPointChoice {
  zp: number;
  system: "ab" | "vega";
  perSecond: boolean;
}

export interface ForcedBandChoice {
  path: string;
  fileName: string;
  label: string;
  dataset: string;
  fCorr: number | null;
  zeroPoint?: ForcedZeroPointChoice | null;
}

export interface ForcedFields {
  positionsText: string;
  bands: readonly ForcedBandChoice[];
  radius: number | null;
  radiusUnit: ForcedRadiusUnit;
  annulusInner: number | null;
  annulusOuter: number | null;
  growthCorrection: boolean;
  referenceRadiusArcsec: number | null;
  limitSigma: number | null;
  excludeDq: boolean;
  gain: number | null;
}

export const DEFAULT_FORCED_FIELDS: ForcedFields = Object.freeze({
  positionsText: "",
  bands: Object.freeze([]),
  radius: 1,
  radiusUnit: "arcsec",
  annulusInner: null,
  annulusOuter: null,
  growthCorrection: false,
  referenceRadiusArcsec: null,
  limitSigma: null,
  excludeDq: false,
  gain: null,
});

export interface ForcedPositionsParse {
  positions: ForcedSkyPosition[];
  errors: string[];
  total: number;
}

export interface ForcedRequestPlan {
  request: ForcedPhotometryRequest | null;
  blockers: string[];
  positions: ForcedPositionsParse;
}

export interface DepthNoise {
  radius_arcsec: number | null;
  radius_px: number | null;
  f_corr: number | null;
}

export type NoiseFactorOffer =
  | { kind: "none" }
  | { kind: "usable"; fCorr: number; radius: number; unit: ForcedRadiusUnit }
  | { kind: "other_radius"; radius: number; unit: ForcedRadiusUnit };

export interface ForcedSizeSource {
  fwhmArcsec: number;
  snr: number | null;
}

export interface ForcedApertureMark {
  source: number;
  id: string;
  x: number;
  y: number;
  limit: boolean;
  failed: boolean;
}

function positiveFinite(value: number | null): value is number {
  return value !== null && Number.isFinite(value) && value > 0;
}

export function bandDisplayName(file: { name: string; path: string }): string {
  const at = file.path.lastIndexOf("#");
  return at < 0 ? file.name : `${file.name} ${file.path.slice(at)}`;
}

export function bandChoice(file: { name: string; path: string }): ForcedBandChoice {
  return { path: file.path, fileName: bandDisplayName(file), label: "", dataset: "", fCorr: null };
}

function roundedRightAscension(lon: number): number {
  const ra = Number(lon.toFixed(RA_DECIMALS));
  return ra >= DEG_PER_TURN ? ra - DEG_PER_TURN : ra;
}

function uniqueIds(typedIds: string[]): { ids: string[]; errors: string[] } {
  const taken = new Set(typedIds);
  const seen = new Set<string>();
  const renamed = new Map<string, string[]>();
  const ids = typedIds.map((id) => {
    if (!seen.has(id)) {
      seen.add(id);
      return id;
    }
    let suffix = 2;
    while (taken.has(`${id}_${suffix}`)) suffix++;
    const name = `${id}_${suffix}`;
    taken.add(name);
    renamed.set(id, [...(renamed.get(id) ?? []), name]);
    return name;
  });
  const errors = [...renamed].map(
    ([id, names]) => `id '${id}' is used ${names.length + 1} times: the later rows are named ${names.join(", ")}`,
  );
  return { ids, errors };
}

export function parseForcedPositions(text: string): ForcedPositionsParse {
  if (!text.trim()) return { positions: [], errors: [], total: 0 };
  const parsed = parseSkyCsv(text, true);
  const total = parsed.targets.length;
  const kept = parsed.targets.slice(0, FORCED_MAX_POSITIONS);
  const { ids, errors: renameErrors } = uniqueIds(kept.map((t, i) => t.label.trim() || String(i + 1)));
  const positions = kept.map((t, i) => ({ id: ids[i], ra_deg: roundedRightAscension(t.lon), dec_deg: t.lat }));
  const errors = [...parsed.errors, ...renameErrors];
  if (total > FORCED_MAX_POSITIONS) {
    errors.push(
      `only the first ${FORCED_MAX_POSITIONS} positions are measured: ${total - FORCED_MAX_POSITIONS} more were left out`,
    );
  }
  return { positions, errors, total };
}

export function positionsCsvText(positions: readonly ForcedSkyPosition[]): string {
  if (positions.length === 0) return "";
  const rows = positions.map(
    (p) => `${p.id.replace(CSV_UNSAFE_ID_CHARACTERS, "_").trim()},${String(p.ra_deg)},${String(p.dec_deg)}`,
  );
  return ["id,ra,dec", ...rows].join("\n");
}

function requestBlockers(fields: ForcedFields, positions: ForcedPositionsParse): string[] {
  const blockers: string[] = [];
  const { annulusInner: inner, annulusOuter: outer } = fields;
  if (fields.bands.length === 0) blockers.push("Choose at least one band image.");
  if (fields.bands.length > FORCED_MAX_BANDS) {
    blockers.push(`At most ${FORCED_MAX_BANDS} bands can be measured in one run (${fields.bands.length} chosen).`);
  }
  if (positions.positions.length === 0) {
    blockers.push("Enter at least one position: RA and Dec in degrees, or sexagesimal with RA in hours.");
  }
  if (!positiveFinite(fields.radius)) blockers.push("Enter the aperture radius.");
  if ((inner === null) !== (outer === null)) blockers.push("The sky annulus needs both an inner and an outer radius.");
  if (inner !== null && outer !== null) {
    if (outer <= inner) blockers.push("The outer sky radius must be larger than the inner one.");
    if (positiveFinite(fields.radius) && inner <= fields.radius) {
      blockers.push("The sky annulus must start outside the aperture.");
    }
  }
  return blockers;
}

function bandInput(choice: ForcedBandChoice): ForcedBandInput {
  const input: ForcedBandInput = { path: choice.path };
  const label = choice.label.trim();
  if (label) input.label = label;
  const dataset = choice.dataset.trim();
  if (dataset) input.dataset = dataset;
  return input;
}

function apertureOf(fields: ForcedFields, radius: number): ForcedAperture {
  const arcsec = fields.radiusUnit === "arcsec";
  const { annulusInner: inner, annulusOuter: outer } = fields;
  const annulus: [number, number] | null = inner !== null && outer !== null ? [inner, outer] : null;
  return {
    ...(arcsec ? { radius_arcsec: radius } : { radius_px: radius }),
    ...(annulus === null ? {} : arcsec ? { annulus_arcsec: annulus } : { annulus_px: annulus }),
    correction: fields.growthCorrection ? "growth_curve" : "none",
    ...(fields.growthCorrection && fields.referenceRadiusArcsec !== null
      ? { reference_radius_arcsec: fields.referenceRadiusArcsec }
      : {}),
  };
}

function runOptions(fields: ForcedFields): ForcedRunOptions {
  return {
    exclude_dq: fields.excludeDq,
    limit_sigma: fields.limitSigma ?? DEFAULT_LIMIT_SIGMA,
    ...(fields.gain === null ? {} : { gain: fields.gain }),
    ...(fields.bands.some((b) => b.fCorr !== null) ? { f_corr: fields.bands.map((b) => b.fCorr) } : {}),
    ...(fields.bands.some((b) => b.zeroPoint != null)
      ? {
          zero_points: fields.bands.map((b) =>
            b.zeroPoint != null
              ? { zp: b.zeroPoint.zp, system: b.zeroPoint.system, per_second: b.zeroPoint.perSecond }
              : null,
          ),
        }
      : {}),
  };
}

export function forcedRequest(fields: ForcedFields): ForcedRequestPlan {
  const positions = parseForcedPositions(fields.positionsText);
  const blockers = requestBlockers(fields, positions);
  if (blockers.length > 0 || !positiveFinite(fields.radius)) return { request: null, blockers, positions };
  const request: ForcedPhotometryRequest = {
    bands: fields.bands.map(bandInput),
    positions: positions.positions,
    aperture: apertureOf(fields, fields.radius),
    options: runOptions(fields),
  };
  return { request, blockers, positions };
}

export function requestKey(request: ForcedPhotometryRequest): string {
  return JSON.stringify(request);
}

export function isStale(resultKey: string | null, plan: ForcedRequestPlan): boolean {
  if (resultKey === null) return false;
  return plan.request === null || requestKey(plan.request) !== resultKey;
}

export function noiseFactorOffer(
  depth: DepthNoise | null | undefined,
  radius: number | null,
  unit: ForcedRadiusUnit,
): NoiseFactorOffer {
  if (depth === null || depth === undefined) return { kind: "none" };
  const measuredRadius = unit === "arcsec" ? depth.radius_arcsec : depth.radius_px;
  if (!positiveFinite(depth.f_corr) || !positiveFinite(radius) || !positiveFinite(measuredRadius)) {
    return { kind: "none" };
  }
  if (Math.abs(measuredRadius - radius) <= NOISE_FACTOR_RADIUS_TOLERANCE * radius) {
    return { kind: "usable", fCorr: depth.f_corr, radius: measuredRadius, unit };
  }
  return { kind: "other_radius", radius: measuredRadius, unit };
}

export function sizeSource(cell: ForcedCell | null, band: ForcedBand): ForcedSizeSource | null {
  if (cell === null || !cell.on_image || cell.error !== null || band.error !== null) return null;
  if (!positiveFinite(cell.fwhm_px) || !positiveFinite(band.pixel_scale_arcsec)) return null;
  const snr = cell.snr !== null && Number.isFinite(cell.snr) ? cell.snr : null;
  return { fwhmArcsec: cell.fwhm_px * band.pixel_scale_arcsec, snr };
}

function bandHasSize(result: ForcedPhotometryResult, index: number): boolean {
  const band = result.bands[index];
  return result.sources.some((s) => sizeSource(s.cells[index] ?? null, band) !== null);
}

export function sizeBand(result: ForcedPhotometryResult, preferredPath: string | null): number | null {
  const preferred = result.bands.findIndex((b, k) => b.path === preferredPath && bandHasSize(result, k));
  if (preferred >= 0) return preferred;
  const first = result.bands.findIndex((_, k) => bandHasSize(result, k));
  return first >= 0 ? first : null;
}

export function bandOfPath(result: ForcedPhotometryResult, path: string | null): number | null {
  if (path === null) return null;
  const index = result.bands.findIndex((b) => b.path === path && b.error === null);
  return index >= 0 ? index : null;
}

export function apertureMarks(result: ForcedPhotometryResult, band: number): ForcedApertureMark[] {
  if (result.bands[band] === undefined) return [];
  const marks: ForcedApertureMark[] = [];
  result.sources.forEach((source, index) => {
    const cell = source.cells[band];
    if (!cell || !cell.on_image || cell.x === null || cell.y === null) return;
    if (!Number.isFinite(cell.x) || !Number.isFinite(cell.y)) return;
    marks.push({
      source: index,
      id: source.id,
      x: cell.x,
      y: cell.y,
      limit: cell.upper_limit_jy !== null,
      failed: cell.error !== null,
    });
  });
  return marks;
}
