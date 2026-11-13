import type { MagSystem, PhotCal, StarPhotometry, UserZeroPoint } from "../services/analysis";
import type { PlotReferenceLine } from "../components/regions/ProfilePlot";
import { CSV_LINE_END, buildCsv, type CsvCell, type CsvColumn } from "./catalogCsv";
import { exportEntries, provenanceLines, summedPixelUnit, unitLines, withProvenance, type ProvenanceEntry } from "./csvProvenance";
import { formatSignificantFixed } from "./formatSignificant";
import { formatWithUncertainty } from "./formatUncertainty";
import { parseImageRef } from "./imageRef";
import type { CalibratedMagnitude, PhotometryLimit } from "./photometryPanel";
import { radiusInArcsec, radiusInPixels, type RadiiInput, type RadiusUnit } from "./photometryUnits";

export interface PhotometryTableRow {
  index: number;
  label: string;
  photometry: StarPhotometry | null;
  sky: { ra: number; dec: number } | null;
  error: string | null;
  magnitude?: CalibratedMagnitude;
  limit?: PhotometryLimit;
}

export const PHOTOMETRY_COLUMN_KEYS = [
  "index",
  "label",
  "x",
  "y",
  "ra",
  "dec",
  "net_flux",
  "flux_err",
  "snr",
  "mag_inst",
  "mag_ab",
  "mag_ab_err",
  "flux_jy",
  "flux_err_jy",
  "fwhm",
  "aperture_radius",
  "sky_inner",
  "sky_outer",
  "bg_mean",
  "bg_sigma",
  "aperture_correction",
  "flux_total",
  "mag_ab_total",
  "ee50_radius",
  "ee80_radius",
  "saturated",
  "n_masked",
  "err_used",
  "error",
  "n_nonfinite",
  "ee_in_aperture",
  "mag",
  "mag_err",
  "mag_system",
  "upper_limit_jy",
  "upper_limit_ab",
] as const;

export type PhotometryColumnKey = (typeof PHOTOMETRY_COLUMN_KEYS)[number];
export type SortDirection = "asc" | "desc";

export const PEAK_SEARCH_RADIUS_PX = 8;
export const MIN_APERTURE_RADIUS_PX = 2;
export const MAX_APERTURE_RADIUS_PX = 60;
export const MAX_SKY_OUTER_RADIUS_PX = 512;
export const FALLBACK_APERTURE_RADIUS_PX = 6;
export const APERTURE_RANGE_HINT = `Aperture radius must be between ${MIN_APERTURE_RADIUS_PX} and ${MAX_APERTURE_RADIUS_PX} px.`;
const PEAK_SEARCH_BOX_PX = 2 * PEAK_SEARCH_RADIUS_PX + 1;
export const BATCH_SEMANTICS_TEXT = `Each position snaps to the brightest pixel in the ${PEAK_SEARCH_BOX_PX} x ${PEAK_SEARCH_BOX_PX} px box around it (${PEAK_SEARCH_RADIUS_PX} px each way) and is then recentred on that light, so a brighter neighbour beyond ${PEAK_SEARCH_RADIUS_PX} px can take over the input; check the measured x/y. Two inputs on one star measure the same star (flagged as duplicates). The aperture radius is fixed for every source (leave it blank for 1.5 x FWHM per star, or ${FALLBACK_APERTURE_RADIUS_PX} px where the FWHM is not measured). When the curve of growth has no plateau before the sky annulus, the correction, total flux and EE radii are empty.`;
export const STARS_ELSEWHERE_NOTICE =
  "The detected stars came from the RGB composite, not from the image this table measures, so they are not offered here; go back to the file and detect again.";

const POSITION_SEPARATOR = /[\s,;]+/;
const COMMENT_PREFIX = "#";

function finiteOrNull(v: number | null | undefined): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

export function rowValue(row: PhotometryTableRow, key: PhotometryColumnKey): CsvCell {
  const p = row.photometry;
  switch (key) {
    case "index":
      return row.index;
    case "label":
      return row.label;
    case "error":
      return row.error;
    case "ra":
      return row.sky?.ra ?? null;
    case "dec":
      return row.sky?.dec ?? null;
    case "saturated":
      return p ? p.saturated : null;
    case "err_used":
      return p ? p.err_used : null;
    case "n_masked":
      return p ? p.n_masked : null;
    case "mag":
      return finiteOrNull(row.magnitude?.mag);
    case "mag_err":
      return finiteOrNull(row.magnitude?.magErr);
    case "mag_system":
      return row.magnitude?.system ?? null;
    case "upper_limit_jy":
      return row.limit?.limitJy ?? null;
    case "upper_limit_ab":
      return row.limit?.limitAb ?? null;
    default:
      return p ? finiteOrNull(p[key]) : null;
  }
}

const MISSING_CELL = "--";
const MAG_DECIMALS = 3;
const FLUX_DIGITS = 4;
const FLUX_EXPONENT_FROM = 1e6;

export function fluxCellText(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) return MISSING_CELL;
  return Math.abs(value) >= FLUX_EXPONENT_FROM ? value.toPrecision(FLUX_DIGITS) : formatSignificantFixed(value, FLUX_DIGITS);
}

export function abCellText(mag: number | null | undefined, err: number | null | undefined): string {
  if (mag == null || !Number.isFinite(mag)) return MISSING_CELL;
  return err != null && Number.isFinite(err) && err > 0 ? formatWithUncertainty(mag, err) : mag.toFixed(MAG_DECIMALS);
}

const OTHER_COLUMN_UNITS = "AB, inst mag; FWHM, EE50 px";

export function tableUnitsCaption(unit: string | null | undefined): string {
  const flux = unit ? `flux ${unit}` : "flux in the pixel unit of the image (not stated in the header)";
  return `units: ${flux}; ${OTHER_COLUMN_UNITS}`;
}

export const NO_DETECTED_STARS_HINT =
  "No detected stars yet. Run Detect Stars in the Astrometry tool (Stars section), or choose another source of positions above.";

const SURFACE_BRIGHTNESS_UNIT = "MJy/sr";
const PIXEL_SUM_SUFFIX = " x pix";

export function summedFluxUnit(photcal: PhotCal | null | undefined): string | null {
  if (photcal?.convention.kind === "jwst_mjy_sr") return `${photcal.bunit ?? SURFACE_BRIGHTNESS_UNIT}${PIXEL_SUM_SUFFIX}`;
  if (photcal?.convention.kind === "flux_density_per_pixel") return photcal.bunit ?? photcal.convention.unit;
  return photcal?.bunit ?? null;
}

export const PHOTOMETRY_CSV_COLUMNS: CsvColumn<PhotometryTableRow>[] = PHOTOMETRY_COLUMN_KEYS.map((key) => ({
  header: key,
  value: (row) => rowValue(row, key),
}));

export interface PhotometryCsvContext {
  exportedUtc: string;
  fileName: string | null;
  image: string | null;
  positions: "stars" | "regions" | "pasted" | "sky";
  bunit: string | null;
  summedUnit: string | null;
  calibration: string | null;
  calibrationNotes: readonly string[];
  magSystem: MagSystem | null;
  zeroPoint: UserZeroPoint | null;
  filter: { dataset: string; filter: string; pivot_um: number; source: string } | null;
  pixelScaleArcsec: number | null;
  radiusUnit: RadiusUnit;
  typedRadii: RadiiInput;
  gain: number | null;
  excludeDq: boolean;
  masked: boolean;
  errLayer: string | null;
  dqLayer: string | null;
  warnings: readonly string[];
  limitSigma: number;
  fCorr: number | null;
  fCorrRadius: string | null;
}

export const PHOTOMETRY_WEIGHTS_NOTE = "5 x 5 subsampled overlap of the aperture with boundary pixels";
export const PHOTOMETRY_ERROR_NOTE =
  "with an error layer (err_used true): sqrt(sum (w sigma)^2) over the aperture, w the overlap weight, plus n_ap^2 sigma_sky^2 / n_sky; without one: sqrt(net_flux / gain + n_ap sigma_sky^2 + n_ap^2 sigma_sky^2 / n_sky), the first term only when a gain is given";
export const PHOTOMETRY_POSITIONS_NOTE = `x, y are measured centroids in 0-based pixels (integer pixel centres); each input position snaps to the brightest pixel within ${PEAK_SEARCH_RADIUS_PX} px and is recentred`;
export const PHOTOMETRY_CORRECTION_NOTE =
  "flux_total / net_flux from the curve of growth of each star; empty when the curve has no plateau before the sky annulus";
export const PHOTOMETRY_LIMIT_NOTE =
  "a row is a limit when flux_jy < limit_sigma x flux_err_jy x f_corr; then upper_limit_jy = limit_sigma x flux_err_jy x f_corr and upper_limit_ab = -2.5 log10(upper_limit_jy) + 8.90; f_corr is 1 unless the line f_corr names a factor; the measured flux and error are kept";

const POSITION_SOURCE_TEXT: Record<PhotometryCsvContext["positions"], string> = {
  stars: "detected stars",
  regions: "point regions",
  pasted: "pasted pixel positions",
  sky: "sky positions (ICRS) placed with the WCS of the image",
};
const ZERO_POINT_SYSTEM_TEXT: Record<UserZeroPoint["system"], string> = { ab: "AB", vega: "Vega" };
const AUTOMATIC_APERTURE_TEXT = `automatic: 1.5 x FWHM of each star, ${FALLBACK_APERTURE_RADIUS_PX} px where the FWHM is not measured`;
const AUTOMATIC_ANNULUS_TEXT = "automatic: 2 to 3 x the aperture radius";
const NO_ERROR_LAYER_TEXT = "none: errors from the sky scatter (column err_used is false)";
const SENTENCE_DIGITS = 6;

function sentenceNumber(value: number): string {
  return String(Number(value.toPrecision(SENTENCE_DIGITS)));
}

function magSystemText(magSystem: MagSystem | null): string {
  if (magSystem === null) return "none";
  return magSystem === "unknown" ? "unknown (AB assumed)" : magSystem;
}

function zeroPointText(zeroPoint: UserZeroPoint | null): string {
  if (zeroPoint === null) return "none";
  const text = `${sentenceNumber(zeroPoint.zp)} ${ZERO_POINT_SYSTEM_TEXT[zeroPoint.system]}`;
  return zeroPoint.per_second ? `${text}, per second` : text;
}

function filterText(filter: PhotometryCsvContext["filter"]): string {
  if (filter === null) return "not identified";
  return `${filter.dataset} ${filter.filter}, pivot ${sentenceNumber(filter.pivot_um)} um, from the ${filter.source}`;
}

function apertureRadiusText(context: PhotometryCsvContext): string {
  const aperture = context.typedRadii.aperture;
  if (aperture === undefined) return AUTOMATIC_APERTURE_TEXT;
  const typed = `${sentenceNumber(aperture)} ${context.radiusUnit}`;
  const converted =
    context.radiusUnit === "px"
      ? radiusInArcsec(aperture, context.pixelScaleArcsec)
      : radiusInPixels(aperture, "arcsec", context.pixelScaleArcsec);
  if (converted === null) return typed;
  return `${typed} (${sentenceNumber(converted)} ${context.radiusUnit === "px" ? "arcsec" : "px"})`;
}

function skyAnnulusText(context: PhotometryCsvContext): string {
  const { skyInner, skyOuter } = context.typedRadii;
  if (skyInner === undefined && skyOuter === undefined) return AUTOMATIC_ANNULUS_TEXT;
  const side = (radius: number | undefined) => (radius === undefined ? "auto" : sentenceNumber(radius));
  return `${side(skyInner)} to ${side(skyOuter)} ${context.radiusUnit}`;
}

function dqExclusionText(context: PhotometryCsvContext): string {
  if (!context.excludeDq) return "off";
  return context.masked ? "applied" : "requested, not applied";
}

function depthFactorText(context: PhotometryCsvContext): string {
  if (context.fCorr === null) return "not applied";
  const radius = context.fCorrRadius === null ? "" : ` (${context.fCorrRadius})`;
  return `${sentenceNumber(context.fCorr)} from the Depth section${radius}, applied on request to rows measured with an error layer`;
}

export function photometryCsvProvenance(context: PhotometryCsvContext, nRows: number): ProvenanceEntry[] {
  return [
    ...exportEntries(context.fileName, context.exportedUtc),
    ["image", context.image],
    ["rows", nRows],
    ["position_source", POSITION_SOURCE_TEXT[context.positions]],
    ["bunit", context.bunit],
    ["calibration", context.calibration ?? "none"],
    ["mag_system", magSystemText(context.magSystem)],
    ["zero_point_typed", zeroPointText(context.zeroPoint)],
    ["filter", filterText(context.filter)],
    ["pixel_scale_arcsec", context.pixelScaleArcsec],
    ["aperture_radius", apertureRadiusText(context)],
    ["sky_annulus", skyAnnulusText(context)],
    ["gain", context.gain ?? "none"],
    ["dq_exclusion", dqExclusionText(context)],
    ["error_layer", context.errLayer ?? NO_ERROR_LAYER_TEXT],
    ["mask_layer", context.dqLayer ?? "none"],
    ["aperture_weights", PHOTOMETRY_WEIGHTS_NOTE],
    ["flux_error", PHOTOMETRY_ERROR_NOTE],
    ["limit_sigma", context.limitSigma],
    ["f_corr", depthFactorText(context)],
    ["upper_limit", PHOTOMETRY_LIMIT_NOTE],
    ["aperture_correction", PHOTOMETRY_CORRECTION_NOTE],
    ["positions", PHOTOMETRY_POSITIONS_NOTE],
    ...context.calibrationNotes.map((note): ProvenanceEntry => ["calibration_note", note]),
    ...[...new Set(context.warnings)].map((warning): ProvenanceEntry => ["warning", warning]),
  ];
}

export function photometryCsvUnits(context: PhotometryCsvContext): Record<string, string> {
  const pixel = context.bunit ?? "image unit";
  const summed = context.summedUnit ?? summedPixelUnit(context.bunit) ?? "image unit, summed over the aperture pixels";
  return {
    x: "pixel (0-based)",
    y: "pixel (0-based)",
    ra: "deg (ICRS)",
    dec: "deg (ICRS)",
    net_flux: summed,
    flux_err: summed,
    mag_inst: "mag (instrumental: -2.5 log10 of net_flux)",
    mag_ab: "mag (AB)",
    mag_ab_err: "mag",
    flux_jy: "Jy",
    flux_err_jy: "Jy",
    fwhm: "pixel",
    aperture_radius: "pixel",
    sky_inner: "pixel",
    sky_outer: "pixel",
    bg_mean: pixel,
    bg_sigma: pixel,
    flux_total: summed,
    mag_ab_total: "mag (AB)",
    ee50_radius: "pixel",
    ee80_radius: "pixel",
    mag: "mag (system of the column mag_system)",
    mag_err: "mag",
    upper_limit_jy: "Jy",
    upper_limit_ab: "mag (AB)",
  };
}

export function photometryTableCsv(rows: PhotometryTableRow[], context?: PhotometryCsvContext): string {
  const body = buildCsv(PHOTOMETRY_CSV_COLUMNS, rows);
  if (!context) return body;
  return withProvenance(
    [...provenanceLines(photometryCsvProvenance(context, rows.length)), ...unitLines(photometryCsvUnits(context))],
    body,
    CSV_LINE_END,
  );
}

export function parsePositions(text: string): { points: [number, number][]; errors: string[] } {
  const points: [number, number][] = [];
  const errors: string[] = [];
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (line === "" || line.startsWith(COMMENT_PREFIX)) continue;
    const parts = line.split(POSITION_SEPARATOR).filter((part) => part !== "");
    const x = parts.length >= 2 ? Number(parts[0]) : Number.NaN;
    const y = parts.length >= 2 ? Number(parts[1]) : Number.NaN;
    if (parts.length !== 2 || !Number.isFinite(x) || !Number.isFinite(y)) {
      errors.push(`line ${i + 1}: expected "x y" or "x,y", got "${line}"`);
      continue;
    }
    points.push([x, y]);
  }
  return { points, errors };
}

function cellKey(cx: number, cy: number): string {
  return `${cx}:${cy}`;
}

export function flagDuplicates(rows: PhotometryTableRow[], tolerancePx = 1): Set<number> {
  const flagged = new Set<number>();
  if (!(tolerancePx > 0) || !Number.isFinite(tolerancePx)) return flagged;
  const cells = new Map<string, { x: number; y: number }[]>();
  const tolerance2 = tolerancePx * tolerancePx;
  for (const row of rows) {
    const p = row.photometry;
    if (!p || !Number.isFinite(p.x) || !Number.isFinite(p.y)) continue;
    const cx = Math.floor(p.x / tolerancePx);
    const cy = Math.floor(p.y / tolerancePx);
    let duplicate = false;
    for (let dy = -1; dy <= 1 && !duplicate; dy++) {
      for (let dx = -1; dx <= 1 && !duplicate; dx++) {
        const seen = cells.get(cellKey(cx + dx, cy + dy));
        if (!seen) continue;
        duplicate = seen.some((q) => (q.x - p.x) ** 2 + (q.y - p.y) ** 2 <= tolerance2);
      }
    }
    if (duplicate) {
      flagged.add(row.index);
      continue;
    }
    const key = cellKey(cx, cy);
    const bucket = cells.get(key);
    if (bucket) bucket.push({ x: p.x, y: p.y });
    else cells.set(key, [{ x: p.x, y: p.y }]);
  }
  return flagged;
}

function sortableValue(cell: CsvCell): number | string | null {
  if (cell === null || cell === undefined) return null;
  if (typeof cell === "boolean") return cell ? 1 : 0;
  if (typeof cell === "number") return Number.isFinite(cell) ? cell : null;
  return cell;
}

export function sortRows(rows: PhotometryTableRow[], key: PhotometryColumnKey, dir: SortDirection): PhotometryTableRow[] {
  const sign = dir === "asc" ? 1 : -1;
  const decorated = rows.map((row, position) => ({ row, position, value: sortableValue(rowValue(row, key)) }));
  decorated.sort((a, b) => {
    if (a.value === null && b.value === null) return a.position - b.position;
    if (a.value === null) return 1;
    if (b.value === null) return -1;
    const cmp =
      typeof a.value === "number" && typeof b.value === "number"
        ? a.value - b.value
        : String(a.value).localeCompare(String(b.value));
    return cmp !== 0 ? sign * cmp : a.position - b.position;
  });
  return decorated.map((d) => d.row);
}

export function medianSnr(rows: PhotometryTableRow[]): number | null {
  const values = rows
    .map((row) => row.photometry?.snr)
    .filter((v): v is number => typeof v === "number" && Number.isFinite(v))
    .sort((a, b) => a - b);
  const n = values.length;
  if (n === 0) return null;
  return n % 2 === 1 ? values[(n - 1) / 2] : (values[n / 2 - 1] + values[n / 2]) / 2;
}

export function photometryCsvFileName(filePath: string): string {
  const base = parseImageRef(filePath).path.split(/[\\/]/).pop() ?? "image";
  const stem = base.replace(/\.(fits?|fts|asdf)(\.(gz|fz|bz2))?$/i, "");
  return `${stem}_photometry.csv`;
}

export function apertureRadiusInRange(radius: number | undefined): boolean {
  return radius === undefined || (radius >= MIN_APERTURE_RADIUS_PX && radius <= MAX_APERTURE_RADIUS_PX);
}

export function plateauCaption(plateauRadius: number | null | undefined): string {
  return plateauRadius != null && Number.isFinite(plateauRadius) ? ` (growth curve plateau at r=${plateauRadius.toFixed(2)} px)` : "";
}

const APERTURE_LINE_COLOR = "#fbbf24";
const SKY_LINE_COLOR = "#a1a1aa";
const PLATEAU_LINE_COLOR = "#ffffff";

export function growthReferenceLines(phot: StarPhotometry): PlotReferenceLine[] {
  const candidates: { value: number | null | undefined; label: string; color: string; dashed: boolean }[] = [
    { value: phot.aperture_radius, label: "r_ap", color: APERTURE_LINE_COLOR, dashed: false },
    { value: phot.sky_inner, label: "sky in", color: SKY_LINE_COLOR, dashed: false },
    { value: phot.sky_outer, label: "sky out", color: SKY_LINE_COLOR, dashed: false },
    { value: phot.plateau_radius, label: "plateau", color: PLATEAU_LINE_COLOR, dashed: true },
    { value: phot.ee50_radius, label: "EE50", color: PLATEAU_LINE_COLOR, dashed: true },
  ];
  return candidates
    .filter((c): c is typeof c & { value: number } => typeof c.value === "number" && Number.isFinite(c.value))
    .map((c) => ({ axis: "x", value: c.value, label: c.label, color: c.color, dashed: c.dashed }));
}
