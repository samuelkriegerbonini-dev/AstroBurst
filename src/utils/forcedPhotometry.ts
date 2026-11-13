import type {
  ForcedAperture,
  ForcedBand,
  ForcedCell,
  ForcedPhotometryResult,
  ForcedSource,
} from "../shared/types/forcedPhotometry";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { PROVENANCE_FILE_SEPARATOR, provenanceLines, unitLines, withProvenance, type ProvenanceEntry } from "./csvProvenance";
import { MISSING_VALUE, formatWithUncertainty } from "./formatUncertainty";

export const UJY_PER_JY = 1e6;
export const FORCED_WEIGHTS_NOTE = "5 x 5 subsampled overlap of the aperture with boundary pixels";
export const FORCED_ERROR_NOTE =
  "sqrt(sum (w sigma)^2) over the aperture, w the overlap weight; with a sky annulus, plus n_ap^2 sigma_sky^2 / n_sky";

export type ForcedBasis = "total" | "aperture";
export type ColourLimit = "none" | "lower" | "upper" | "both";

export interface ForcedMagnitude {
  mag: number | null;
  err: number | null;
  limit: boolean;
}

export interface ForcedColour {
  value: number | null;
  err: number | null;
  limit: ColourLimit;
  basis: ForcedBasis;
}

export interface ForcedSedPoint {
  index: number;
  filter: string;
  pivotUm: number;
  value: number;
  err: number | null;
  limit: boolean;
}

export interface ForcedCsvInput {
  result: ForcedPhotometryResult;
  exportedUtc: string;
  appVersion: string;
}

const UJY_DIGITS = 12;
const PROVENANCE_DIGITS = 6;
const UJY_UNIT = "uJy";
const AB_UNIT = "mag (AB)";
const NO_MAGNITUDE: ForcedMagnitude = { mag: null, err: null, limit: false };

function finite(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function toUjy(jy: number | null | undefined): number | null {
  return finite(jy) ? Number((jy * UJY_PER_JY).toPrecision(UJY_DIGITS)) : null;
}

export function isLimit(cell: ForcedCell | null): boolean {
  return cell !== null && cell.upper_limit_jy !== null;
}

function pivotOf(band: ForcedBand): number | null {
  const pivot = band.filter?.pivot_um;
  return finite(pivot) ? pivot : null;
}

export function bandOrder(bands: readonly ForcedBand[]): number[] {
  const pivots = bands.map(pivotOf);
  const indices = bands.map((_, index) => index);
  const withPivot = indices
    .filter((index) => pivots[index] !== null)
    .sort((a, b) => (pivots[a] as number) - (pivots[b] as number) || a - b);
  return [...withPivot, ...indices.filter((index) => pivots[index] === null)];
}

export function colourBasis(a: ForcedBand, b: ForcedBand): ForcedBasis {
  return a.aperture_correction && b.aperture_correction ? "total" : "aperture";
}

function basisFactor(band: ForcedBand, basis: ForcedBasis): number {
  return basis === "total" && band.aperture_correction ? band.aperture_correction.factor : 1;
}

function usableCell(cell: ForcedCell | null): cell is ForcedCell {
  return cell !== null && cell.on_image && cell.error === null;
}

export function cellMagnitude(cell: ForcedCell | null, band: ForcedBand, basis: ForcedBasis): ForcedMagnitude {
  if (!usableCell(cell)) return NO_MAGNITUDE;
  const limit = isLimit(cell);
  const mag = limit ? cell.upper_limit_ab : cell.mag_ab;
  if (!finite(mag)) return NO_MAGNITUDE;
  const shifted = mag - 2.5 * Math.log10(basisFactor(band, basis));
  if (limit) return { mag: shifted, err: null, limit: true };
  return { mag: shifted, err: finite(cell.mag_ab_err) ? cell.mag_ab_err : null, limit: false };
}

export function forcedColour(result: ForcedPhotometryResult, source: number, a: number, b: number): ForcedColour {
  const basis = colourBasis(result.bands[a], result.bands[b]);
  const cells = result.sources[source].cells;
  const ma = cellMagnitude(cells[a] ?? null, result.bands[a], basis);
  const mb = cellMagnitude(cells[b] ?? null, result.bands[b], basis);
  if (ma.mag === null || mb.mag === null) return { value: null, err: null, limit: "none", basis };
  if (ma.limit && mb.limit) return { value: null, err: null, limit: "both", basis };
  const value = ma.mag - mb.mag;
  if (ma.limit) return { value, err: null, limit: "lower", basis };
  if (mb.limit) return { value, err: null, limit: "upper", basis };
  const err = ma.err !== null && mb.err !== null ? Math.hypot(ma.err, mb.err) : null;
  return { value, err, limit: "none", basis };
}

function colourBand(band: ForcedBand): boolean {
  return band.error === null && band.jy_per_native_unit !== null && band.filter !== null;
}

export function adjacentPairs(bands: readonly ForcedBand[]): [number, number][] {
  const order = bandOrder(bands).filter((index) => colourBand(bands[index]));
  const pairs: [number, number][] = [];
  for (let i = 1; i < order.length; i++) pairs.push([order[i - 1], order[i]]);
  return pairs;
}

function effectiveErrJy(cell: ForcedCell): number | null {
  return cell.flux_err_corr_jy ?? cell.flux_err_jy;
}

export function fluxCellText(cell: ForcedCell | null, band: ForcedBand): string {
  if (cell === null || band.error !== null) return MISSING_VALUE;
  if (!cell.on_image) return "off image";
  if (cell.error !== null) return "failed";
  if (isLimit(cell)) return "< " + formatWithUncertainty(toUjy(cell.upper_limit_jy), null, { unit: UJY_UNIT });
  if (cell.flux_jy !== null) {
    return formatWithUncertainty(toUjy(cell.flux_jy), toUjy(effectiveErrJy(cell)), { unit: UJY_UNIT });
  }
  return formatWithUncertainty(cell.flux_native, cell.flux_err_native, { unit: band.bunit ?? undefined });
}

export function magCellText(cell: ForcedCell | null): string {
  if (cell === null) return MISSING_VALUE;
  if (isLimit(cell)) return finite(cell.upper_limit_ab) ? "> " + cell.upper_limit_ab.toFixed(2) : MISSING_VALUE;
  return finite(cell.mag_ab) ? formatWithUncertainty(cell.mag_ab, cell.mag_ab_err) : MISSING_VALUE;
}

export function colourCellText(colour: ForcedColour): string {
  if (colour.value === null) return MISSING_VALUE;
  if (colour.limit === "lower") return "> " + colour.value.toFixed(2);
  if (colour.limit === "upper") return "< " + colour.value.toFixed(2);
  return formatWithUncertainty(colour.value, colour.err);
}

export function cellFlags(cell: ForcedCell | null, band: ForcedBand): string[] {
  if (cell === null) return [];
  const flags: string[] = [];
  if (cell.saturated) flags.push("saturated");
  const masked = cell.masked_fraction;
  if (finite(masked) && masked > 0) flags.push(`${(masked * 100).toFixed(masked < 0.1 ? 1 : 0)}% no data`);
  const bits = cell.flags_or ?? 0;
  const names = band.flag_names ?? [];
  if (names.length > 0) {
    for (const flag of names) if ((bits & flag.value) !== 0) flags.push(flag.name);
  } else if (bits > 0) {
    flags.push(`flags 0x${bits.toString(16)}`);
  }
  return flags;
}

function scaledUjy(jy: number | null, factor: number): number | null {
  return finite(jy) ? toUjy(jy * factor) : null;
}

export function forcedSedPoints(
  result: ForcedPhotometryResult,
  source: number,
): { points: ForcedSedPoint[]; basis: ForcedBasis } {
  const cells = result.sources[source].cells;
  const included = bandOrder(result.bands).filter((index) => {
    const band = result.bands[index];
    const pivot = pivotOf(band);
    const cell = cells[index] ?? null;
    return pivot !== null && pivot > 0 && band.error === null && cell !== null && (finite(cell.flux_jy) || isLimit(cell));
  });
  const basis: ForcedBasis =
    included.length > 0 && included.every((index) => result.bands[index].aperture_correction) ? "total" : "aperture";
  const points = included.map((index): ForcedSedPoint => {
    const band = result.bands[index];
    const cell = cells[index] as ForcedCell;
    const factor = basisFactor(band, basis);
    const base = { index, filter: band.label, pivotUm: pivotOf(band) as number };
    if (isLimit(cell)) return { ...base, value: scaledUjy(cell.upper_limit_jy, factor) as number, err: null, limit: true };
    return { ...base, value: scaledUjy(cell.flux_jy, factor) as number, err: scaledUjy(effectiveErrJy(cell), factor), limit: false };
  });
  return { points, basis };
}

export function bandSlugs(bands: readonly ForcedBand[]): string[] {
  const used = new Set<string>();
  return bands.map((band) => {
    const slug = band.label.replace(/[^A-Za-z0-9]/g, "_") || "band";
    let candidate = slug;
    for (let n = 2; used.has(candidate); n++) candidate = `${slug}_${n}`;
    used.add(candidate);
    return candidate;
  });
}

function provenanceNumber(value: number): string {
  return String(Number(value.toPrecision(PROVENANCE_DIGITS)));
}

function optionalNumber(value: number | null): string {
  return value === null ? MISSING_VALUE : provenanceNumber(value);
}

function bandLineText(band: ForcedBand): string {
  const head = `${band.label}; file ${band.file_name}`;
  if (band.error !== null) return `${head}; not measured: ${band.error}`;
  const filter = band.filter
    ? `filter ${band.filter.dataset} ${band.filter.filter}, pivot ${provenanceNumber(band.filter.pivot_um)} um`
    : "filter not identified";
  const err = band.err_origin
    ? `error layer ${band.err_origin.role} from ${band.err_origin.source}${band.err_origin.assumed ? " (assumed)" : ""}`
    : "error layer none";
  const correction = band.aperture_correction;
  const parts = [
    head,
    filter,
    `pixel scale ${optionalNumber(band.pixel_scale_arcsec)} arcsec/px`,
    `radius ${optionalNumber(band.aperture_radius_px)} px`,
    `calibration ${band.calibration_origin ?? "none"}`,
    err,
    band.f_corr === null ? "f_corr not given" : `f_corr ${provenanceNumber(band.f_corr)}`,
    correction
      ? `aperture correction ${provenanceNumber(correction.factor)} (scatter ${provenanceNumber(correction.scatter)}, ${provenanceNumber(correction.n_stars)} stars, reference ${provenanceNumber(correction.reference_radius_px)} px)`
      : "aperture correction none",
  ];
  return parts.join("; ");
}

function skyAnnulusText(aperture: ForcedAperture): string {
  if (aperture.annulus_arcsec) {
    return `${provenanceNumber(aperture.annulus_arcsec[0])} to ${provenanceNumber(aperture.annulus_arcsec[1])} arcsec`;
  }
  if (aperture.annulus_px) return `${provenanceNumber(aperture.annulus_px[0])} to ${provenanceNumber(aperture.annulus_px[1])} px`;
  return "none";
}

function provenanceEntries(input: ForcedCsvInput, extra: ProvenanceEntry[]): ProvenanceEntry[] {
  const { result } = input;
  const aperture = result.aperture;
  const growthCurve = aperture.correction === "growth_curve";
  const entries: ProvenanceEntry[] = [
    ["software", `AstroBurst ${input.appVersion}`],
    ["exported_utc", input.exportedUtc],
    ["file", result.bands.map((band) => band.file_name).join(PROVENANCE_FILE_SEPARATOR)],
    ["positions", result.sources.length],
    ["bands", result.bands.length],
    finite(aperture.radius_arcsec)
      ? ["aperture_radius_arcsec", aperture.radius_arcsec]
      : ["aperture_radius_px", aperture.radius_px ?? null],
    ["sky_annulus", skyAnnulusText(aperture)],
    ["aperture_correction", growthCurve ? "growth curve of the stars of each band" : "none"],
  ];
  if (growthCurve) entries.push(["reference_radius_arcsec", aperture.reference_radius_arcsec ?? null]);
  entries.push(
    ["limit_sigma", result.limit_sigma],
    ["aperture_weights", FORCED_WEIGHTS_NOTE],
    ["flux_error", FORCED_ERROR_NOTE],
    ...extra,
  );
  result.bands.forEach((band, k) => entries.push([`band_${k + 1}`, bandLineText(band)]));
  return entries;
}

interface ForcedLongRow {
  source: ForcedSource;
  band: ForcedBand;
  cell: ForcedCell | null;
}

const FORCED_LONG_COLUMNS: CsvColumn<ForcedLongRow>[] = [
  { header: "id", value: (r) => r.source.id },
  { header: "ra_deg", value: (r) => r.source.ra_deg },
  { header: "dec_deg", value: (r) => r.source.dec_deg },
  { header: "band", value: (r) => r.band.label },
  { header: "file", value: (r) => r.band.file_name },
  { header: "filter", value: (r) => (r.band.filter ? `${r.band.filter.dataset} ${r.band.filter.filter}` : null) },
  { header: "pivot_um", value: (r) => r.band.filter?.pivot_um },
  { header: "x", value: (r) => r.cell?.x },
  { header: "y", value: (r) => r.cell?.y },
  { header: "on_image", value: (r) => r.cell?.on_image },
  { header: "flux_native", value: (r) => r.cell?.flux_native },
  { header: "flux_err_native", value: (r) => r.cell?.flux_err_native },
  { header: "flux_ujy", value: (r) => toUjy(r.cell?.flux_jy) },
  { header: "flux_err_ujy", value: (r) => toUjy(r.cell?.flux_err_jy) },
  { header: "flux_err_corr_ujy", value: (r) => toUjy(r.cell?.flux_err_corr_jy) },
  { header: "flux_total_ujy", value: (r) => toUjy(r.cell?.flux_total_jy) },
  { header: "mag_ab", value: (r) => r.cell?.mag_ab },
  { header: "mag_ab_err", value: (r) => r.cell?.mag_ab_err },
  { header: "upper_limit_ujy", value: (r) => toUjy(r.cell?.upper_limit_jy) },
  { header: "upper_limit_ab", value: (r) => r.cell?.upper_limit_ab },
  { header: "snr", value: (r) => r.cell?.snr },
  { header: "masked_fraction", value: (r) => r.cell?.masked_fraction },
  { header: "n_masked", value: (r) => r.cell?.n_masked },
  { header: "n_nonfinite", value: (r) => r.cell?.n_nonfinite },
  { header: "flags_or", value: (r) => r.cell?.flags_or },
  { header: "saturated", value: (r) => r.cell?.saturated },
  { header: "fwhm_px", value: (r) => r.cell?.fwhm_px },
  { header: "error", value: (r) => (r.cell ? r.cell.error : r.band.error) },
];

const FORCED_LONG_UNITS: Record<string, string> = {
  ra_deg: "deg",
  dec_deg: "deg",
  pivot_um: "um",
  x: "px (0-based)",
  y: "px (0-based)",
  flux_native: "image unit (see the band lines)",
  flux_err_native: "image unit (see the band lines)",
  flux_ujy: UJY_UNIT,
  flux_err_ujy: UJY_UNIT,
  flux_err_corr_ujy: UJY_UNIT,
  flux_total_ujy: UJY_UNIT,
  mag_ab: AB_UNIT,
  mag_ab_err: AB_UNIT,
  upper_limit_ujy: UJY_UNIT,
  upper_limit_ab: AB_UNIT,
  fwhm_px: "px",
};

export function forcedLongCsv(input: ForcedCsvInput): string {
  const { result } = input;
  const rows: ForcedLongRow[] = result.sources.flatMap((source) =>
    result.bands.map((band, k) => ({ source, band, cell: source.cells[k] ?? null })),
  );
  const lines = [...provenanceLines(provenanceEntries(input, [])), ...unitLines(FORCED_LONG_UNITS)];
  return withProvenance(lines, buildCsv(FORCED_LONG_COLUMNS, rows), CSV_LINE_END);
}

interface ForcedWideRow {
  source: ForcedSource;
  colours: ForcedColour[];
}

export function forcedWideCsv(input: ForcedCsvInput): string {
  const { result } = input;
  const slugs = bandSlugs(result.bands);
  const pairs = adjacentPairs(result.bands);
  const columns: CsvColumn<ForcedWideRow>[] = [
    { header: "id", value: (r) => r.source.id },
    { header: "ra_deg", value: (r) => r.source.ra_deg },
    { header: "dec_deg", value: (r) => r.source.dec_deg },
  ];
  const units: Record<string, string> = { ra_deg: "deg", dec_deg: "deg" };
  for (const k of bandOrder(result.bands)) {
    if (result.bands[k].error !== null) continue;
    const slug = slugs[k];
    const cellOf = (r: ForcedWideRow) => r.source.cells[k] ?? null;
    columns.push(
      { header: `${slug}_flux_ujy`, value: (r) => toUjy(cellOf(r)?.flux_jy) },
      { header: `${slug}_flux_err_ujy`, value: (r) => toUjy(cellOf(r)?.flux_err_jy) },
      { header: `${slug}_mag_ab`, value: (r) => cellOf(r)?.mag_ab },
      { header: `${slug}_mag_ab_err`, value: (r) => cellOf(r)?.mag_ab_err },
      { header: `${slug}_upper_limit_ab`, value: (r) => cellOf(r)?.upper_limit_ab },
      { header: `${slug}_masked_fraction`, value: (r) => cellOf(r)?.masked_fraction },
    );
    units[`${slug}_flux_ujy`] = UJY_UNIT;
    units[`${slug}_flux_err_ujy`] = UJY_UNIT;
    units[`${slug}_mag_ab`] = AB_UNIT;
    units[`${slug}_mag_ab_err`] = AB_UNIT;
    units[`${slug}_upper_limit_ab`] = AB_UNIT;
  }
  const colourNames: string[] = [];
  pairs.forEach(([a, b], p) => {
    const name = `${slugs[a]}-${slugs[b]}`;
    columns.push(
      { header: name, value: (r) => r.colours[p].value },
      { header: `${name}_err`, value: (r) => r.colours[p].err },
      { header: `${name}_limit`, value: (r) => r.colours[p].limit },
    );
    units[name] = AB_UNIT;
    units[`${name}_err`] = AB_UNIT;
    colourNames.push(`${name} (${colourBasis(result.bands[a], result.bands[b])})`);
  });
  const rows = result.sources.map((source, s) => ({ source, colours: pairs.map(([a, b]) => forcedColour(result, s, a, b)) }));
  const colours: ProvenanceEntry = ["colours", colourNames.length > 0 ? colourNames.join(PROVENANCE_FILE_SEPARATOR) : "none"];
  const lines = [...provenanceLines(provenanceEntries(input, [colours])), ...unitLines(units)];
  return withProvenance(lines, buildCsv(columns, rows), CSV_LINE_END);
}

export function forcedCsvFileName(kind: "long" | "wide"): string {
  return `forced_photometry_${kind}.csv`;
}
