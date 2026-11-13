import type { PlacedCatalogRow } from "../shared/types/catalog";
import type {
  CatalogueFluxUnit,
  CompareAperture,
  CompareApertureUsed,
  CompareBandInfo,
  CompareColumn,
  CompareCut,
  CompareCutOp,
  CompareMapping,
  ComparePreset,
  CompareProbe,
  CompareReport,
  CompareRequest,
  CompareRow,
} from "../shared/types/catalogCompare";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { exportEntries, provenanceLines, summedPixelUnit, unitLines, withProvenance, type ProvenanceEntry } from "./csvProvenance";
import { decimalDraftText, decimalFieldState, firstFieldError, parseDecimalText } from "./decimalText";
import { formatWithUncertainty, MISSING_VALUE } from "./formatUncertainty";
import { exportStem } from "./imageRef";
import { isFiniteNumber } from "./values";

export const CUSTOM_PRESET_KEY = "custom";
export const CATALOGUE_FLUX_UNITS: readonly CatalogueFluxUnit[] = ["nanomaggy", "Jy", "mJy", "uJy", "nJy", "zero_point", "ab_mag"];
export const FLUX_UNIT_LABELS: Readonly<Record<CatalogueFluxUnit, string>> = {
  nanomaggy: "nanomaggy",
  Jy: "Jy",
  mJy: "mJy",
  uJy: "uJy",
  nJy: "nJy",
  zero_point: "counts with a zero point",
  ab_mag: "AB magnitude",
};
export const COMPARE_CUT_OPS: readonly CompareCutOp[] = ["==", "!=", "<", "<=", ">=", ">"];
export const MAX_SHOWN_ROWS = 200;

export type CompareErrorKind = "none" | "flux_err" | "flux_ivar";
export type CompareRadiusUnit = "arcsec" | "px";

export interface CompareCutDraft {
  column: string;
  op: CompareCutOp;
  value: string;
}

export interface CompareForm {
  presetKey: string;
  catalogueBand: string;
  idColumn: string;
  raColumn: string;
  decColumn: string;
  fluxColumn: string;
  fluxUnit: CatalogueFluxUnit;
  zeroPointText: string;
  errorKind: CompareErrorKind;
  errorColumn: string;
  classColumn: string;
  missingValueText: string;
  radiusText: string;
  radiusUnit: CompareRadiusUnit;
  localSky: boolean;
  annulusInnerText: string;
  annulusOuterText: string;
  cuts: CompareCutDraft[];
  minSnrText: string;
  maxSnrText: string;
  minNeighbourText: string;
  excludeDq: boolean;
}

export type CompareRequestResult = { request: CompareRequest; error: null } | { request: null; error: string };

const POSITION_COLUMN_PAIRS: readonly (readonly [string, string])[] = [
  ["ra", "dec"],
  ["alphawin_j2000", "deltawin_j2000"],
  ["ramean", "decmean"],
];

const ANNULUS_ORDER_ERROR = "Sky annulus: the outer radius must be larger than the inner radius";

export function presetKeyOf(preset: ComparePreset): string {
  return `${preset.id}:${preset.band}`;
}

export function presetLabel(preset: ComparePreset): string {
  return `${preset.label}, ${preset.band}`;
}

function columnNamed(columns: readonly CompareColumn[], lowerName: string): string | null {
  return columns.find((column) => column.name.toLowerCase() === lowerName)?.name ?? null;
}

export function guessPositionColumns(columns: readonly CompareColumn[]): { ra: string; dec: string } {
  for (const [raName, decName] of POSITION_COLUMN_PAIRS) {
    const ra = columnNamed(columns, raName);
    const dec = columnNamed(columns, decName);
    if (ra !== null && dec !== null) return { ra, dec };
  }
  return { ra: "", dec: "" };
}

export function emptyCompareForm(probe: CompareProbe | null): CompareForm {
  const position = guessPositionColumns(probe?.columns ?? []);
  return {
    presetKey: CUSTOM_PRESET_KEY,
    catalogueBand: "",
    idColumn: "",
    raColumn: position.ra,
    decColumn: position.dec,
    fluxColumn: "",
    fluxUnit: "uJy",
    zeroPointText: "",
    errorKind: "none",
    errorColumn: "",
    classColumn: "",
    missingValueText: "",
    radiusText: "",
    radiusUnit: "arcsec",
    localSky: false,
    annulusInnerText: "",
    annulusOuterText: "",
    cuts: [],
    minSnrText: "",
    maxSnrText: "",
    minNeighbourText: "",
    excludeDq: false,
  };
}

export function formFromPreset(preset: ComparePreset): CompareForm {
  const m = preset.mapping;
  const a = preset.aperture;
  const errorKind: CompareErrorKind = m.flux_err !== null ? "flux_err" : m.flux_ivar !== null ? "flux_ivar" : "none";
  const radiusUnit: CompareRadiusUnit = a.radius_arcsec === null && a.radius_px !== null ? "px" : "arcsec";
  const radius = radiusUnit === "px" ? a.radius_px : a.radius_arcsec;
  const annulus = radiusUnit === "px" ? a.annulus_px : a.annulus_arcsec;
  return {
    presetKey: presetKeyOf(preset),
    catalogueBand: preset.band,
    idColumn: m.id ?? "",
    raColumn: m.ra,
    decColumn: m.dec,
    fluxColumn: m.flux,
    fluxUnit: m.flux_unit,
    zeroPointText: decimalDraftText(m.zero_point),
    errorKind,
    errorColumn: m.flux_err ?? m.flux_ivar ?? "",
    classColumn: m.class ?? "",
    missingValueText: decimalDraftText(m.missing_value),
    radiusText: decimalDraftText(radius),
    radiusUnit,
    localSky: annulus !== null,
    annulusInnerText: annulus === null ? "" : String(annulus[0]),
    annulusOuterText: annulus === null ? "" : String(annulus[1]),
    cuts: preset.cuts.map((cut) => ({ column: cut.column, op: cut.op, value: String(cut.value) })),
    minSnrText: decimalDraftText(preset.min_snr),
    maxSnrText: decimalDraftText(preset.max_snr),
    minNeighbourText: decimalDraftText(preset.min_neighbour_arcsec),
    excludeDq: false,
  };
}

export function initialCompareForm(probe: CompareProbe, imageBand: string | null = null): CompareForm {
  const band = imageBand?.toLowerCase() ?? null;
  const preset = probe.presets.find((p) => band !== null && p.band.toLowerCase() === band) ?? probe.presets[0];
  return preset === undefined ? emptyCompareForm(probe) : formFromPreset(preset);
}

export function presetOfForm(form: CompareForm, probe: CompareProbe | null): ComparePreset | null {
  return probe?.presets.find((preset) => presetKeyOf(preset) === form.presetKey) ?? null;
}

export function cutText(cut: CompareCut): string {
  return `${cut.column} ${cut.op} ${cut.value}`;
}

function refused(error: string): CompareRequestResult {
  return { request: null, error };
}

function cutDraftError(draft: CompareCutDraft, index: number): string | null {
  if (draft.column === "") return `Cut ${index + 1}: choose a column`;
  if (draft.value.trim() === "") return `Cut ${index + 1}: type a value`;
  return null;
}

function cutOf(draft: CompareCutDraft, columns: readonly CompareColumn[]): CompareCut {
  const numeric = columns.some((column) => column.name === draft.column && column.numeric);
  const number = numeric ? parseDecimalText(draft.value) : null;
  return { column: draft.column, op: draft.op, value: number ?? draft.value.trim() };
}

function columnOrNull(name: string): string | null {
  return name === "" ? null : name;
}

export function compareRequestOf(form: CompareForm, imagePath: string, probe: CompareProbe): CompareRequestResult {
  if (form.raColumn === "") return refused("Choose the RA column");
  if (form.decColumn === "") return refused("Choose the Dec column");
  if (form.fluxColumn === "") return refused("Choose the flux column");
  if (form.errorKind !== "none" && form.errorColumn === "") return refused("Choose the error column");
  const usesZeroPoint = form.fluxUnit === "zero_point";
  const radius = decimalFieldState(form.radiusText, { min: 0.001 });
  const zeroPoint = decimalFieldState(form.zeroPointText);
  const missingValue = decimalFieldState(form.missingValueText, { optional: true });
  const annulusInner = decimalFieldState(form.annulusInnerText, { min: 0.001 });
  const annulusOuter = decimalFieldState(form.annulusOuterText, { min: 0.001 });
  const minSnr = decimalFieldState(form.minSnrText, { optional: true, min: 0 });
  const maxSnr = decimalFieldState(form.maxSnrText, { optional: true, min: 0 });
  const minNeighbour = decimalFieldState(form.minNeighbourText, { optional: true, min: 0.001 });
  const fieldError = firstFieldError([
    ["Aperture radius", radius],
    ...(usesZeroPoint ? [["Zero point", zeroPoint] as const] : []),
    ["Missing value", missingValue],
    ...(form.localSky ? [["Sky annulus inner", annulusInner] as const, ["Sky annulus outer", annulusOuter] as const] : []),
    ["Minimum S/N", minSnr],
    ["Maximum S/N", maxSnr],
    ["Nearest neighbour (arcsec)", minNeighbour],
  ]);
  if (fieldError !== null) return refused(fieldError);
  const annulus: [number, number] | null =
    form.localSky && annulusInner.value !== null && annulusOuter.value !== null ? [annulusInner.value, annulusOuter.value] : null;
  if (annulus !== null && annulus[0] >= annulus[1]) return refused(ANNULUS_ORDER_ERROR);
  const cutError = form.cuts.map(cutDraftError).find((error) => error !== null) ?? null;
  if (cutError !== null) return refused(cutError);
  const inArcsec = form.radiusUnit === "arcsec";
  const aperture: CompareAperture = {
    radius_arcsec: inArcsec ? radius.value : null,
    radius_px: inArcsec ? null : radius.value,
    annulus_arcsec: inArcsec ? annulus : null,
    annulus_px: inArcsec ? null : annulus,
  };
  const band = form.catalogueBand.trim();
  return {
    request: {
      band: { path: imagePath, label: null, dataset: null },
      table_path: probe.table_path,
      preset: presetOfForm(form, probe)?.id ?? null,
      catalogue_band: band === "" ? null : band,
      mapping: {
        id: columnOrNull(form.idColumn),
        ra: form.raColumn,
        dec: form.decColumn,
        flux: form.fluxColumn,
        flux_unit: form.fluxUnit,
        zero_point: usesZeroPoint ? zeroPoint.value : null,
        flux_err: form.errorKind === "flux_err" ? form.errorColumn : null,
        flux_ivar: form.errorKind === "flux_ivar" ? form.errorColumn : null,
        class: columnOrNull(form.classColumn),
        missing_value: missingValue.value,
      },
      aperture,
      cuts: form.cuts.map((draft) => cutOf(draft, probe.columns)),
      min_snr: minSnr.value,
      max_snr: maxSnr.value,
      min_neighbour_arcsec: minNeighbour.value,
      exclude_dq: form.excludeDq,
    },
    error: null,
  };
}

function presetSettingsText(
  settings: Pick<CompareRequest, "mapping" | "aperture" | "cuts" | "min_snr" | "max_snr" | "min_neighbour_arcsec">,
  band: string | null,
): string {
  const m = settings.mapping;
  const a = settings.aperture;
  return JSON.stringify([
    m.id,
    m.ra,
    m.dec,
    m.flux,
    m.flux_unit,
    m.zero_point,
    m.flux_err,
    m.flux_ivar,
    m.class,
    m.missing_value,
    a.radius_arcsec,
    a.radius_px,
    a.annulus_arcsec,
    a.annulus_px,
    settings.cuts.map((cut) => [cut.column, cut.op, cut.value]),
    settings.min_snr,
    settings.max_snr,
    settings.min_neighbour_arcsec,
    band,
  ]);
}

export function formEdited(form: CompareForm, probe: CompareProbe | null): boolean {
  if (probe === null) return false;
  const preset = presetOfForm(form, probe);
  if (preset === null) return false;
  const result = compareRequestOf(form, "", probe);
  if (result.request === null) return true;
  return presetSettingsText(result.request, result.request.catalogue_band) !== presetSettingsText(preset, preset.band);
}

export interface CompareRun {
  report: CompareReport;
  request: CompareRequest;
  presetEdited: boolean;
}

export interface CompareHeadline {
  counts: string;
  ratio: string;
  offset: string | null;
  errRatio: string | null;
  zeroPoint: string | null;
}

const RATIO_NOT_AVAILABLE = "not available";

export function microJy(valueJy: number | null | undefined): number | null {
  return isFiniteNumber(valueJy) ? Number((valueJy * 1e6).toPrecision(12)) : null;
}

export function signedFixed(value: number, digits: number): string {
  const text = value.toFixed(digits);
  if (Number(text) === 0) return (0).toFixed(digits);
  return value > 0 ? `+${text}` : text;
}

function fixedOrMissing(value: number | null, digits: number): string {
  return value === null ? MISSING_VALUE : value.toFixed(digits);
}

function offsetText(ratioMedian: number, offsetMag: number | null): string {
  const percent = `${signedFixed((ratioMedian - 1) * 100, 3)} percent`;
  return offsetMag === null ? percent : `${percent}, ${signedFixed(offsetMag, 4)} mag`;
}

export function compareHeadline(report: CompareReport): CompareHeadline {
  const counts = `${report.n_table} rows in the table, ${report.n_on_image} on the image, ${report.n_after_cuts} after the cuts, ${report.n_measured} measured, ${report.n_used} used`;
  const median = report.ratio_median;
  const errMedian = report.err_ratio_median;
  const zeroPoint = report.implied_zero_point_ab;
  return {
    counts,
    ratio:
      median === null
        ? RATIO_NOT_AVAILABLE
        : `${formatWithUncertainty(median, report.ratio_median_err)} (scatter ${formatWithUncertainty(report.ratio_nmad, null)}, N = ${report.n_ratio})`,
    offset: median === null ? null : offsetText(median, report.ratio_offset_mag),
    errRatio:
      errMedian === null
        ? null
        : `${errMedian.toFixed(3)} (scatter ${fixedOrMissing(report.err_ratio_nmad, 3)}, N = ${report.n_err})`,
    zeroPoint:
      zeroPoint === null
        ? null
        : `${zeroPoint.toFixed(3)} AB (scatter ${fixedOrMissing(report.implied_zero_point_nmad, 3)} mag, N = ${report.n_zero_point})`,
  };
}

export function apertureUsedText(aperture: CompareApertureUsed): string {
  const annulus = aperture.annulus_px;
  const sky =
    aperture.local_sky && annulus !== null
      ? `local sky from ${annulus[0].toFixed(2)} to ${annulus[1].toFixed(2)} px`
      : "no local sky";
  return `r = ${aperture.radius_px.toFixed(2)} px (${aperture.radius_arcsec.toFixed(2)} arcsec at ${aperture.pixel_scale_arcsec.toFixed(4)} arcsec per px), ${sky}`;
}

export function rejectedText(item: { reason: string; count: number }): string {
  return `${item.count} x ${item.reason}`;
}

export function fluxCellUjy(valueJy: number | null, errJy: number | null): string {
  return formatWithUncertainty(microJy(valueJy), microJy(errJy));
}

export function ratioCell(ratio: number | null): string {
  return isFiniteNumber(ratio) ? ratio.toFixed(4) : MISSING_VALUE;
}

export function orderedRows(rows: readonly CompareRow[]): CompareRow[] {
  return [...rows.filter((row) => row.used), ...rows.filter((row) => !row.used)];
}

export const COMPARE_CSV_COLUMNS: CsvColumn<CompareRow>[] = [
  { header: "row", value: (r) => r.row },
  { header: "id", value: (r) => r.id },
  { header: "ra_deg", value: (r) => r.ra_deg },
  { header: "dec_deg", value: (r) => r.dec_deg },
  { header: "x", value: (r) => r.x },
  { header: "y", value: (r) => r.y },
  { header: "class", value: (r) => r.class },
  { header: "flux_cat_ujy", value: (r) => microJy(r.flux_cat_jy) },
  { header: "flux_err_cat_ujy", value: (r) => microJy(r.flux_err_cat_jy) },
  { header: "snr_cat", value: (r) => r.snr_cat },
  { header: "flux_native", value: (r) => r.flux_native },
  { header: "flux_err_native", value: (r) => r.flux_err_native },
  { header: "flux_ujy", value: (r) => microJy(r.flux_jy) },
  { header: "flux_err_ujy", value: (r) => microJy(r.flux_err_jy) },
  { header: "ratio", value: (r) => r.ratio },
  { header: "masked_fraction", value: (r) => r.masked_fraction },
  { header: "used", value: (r) => r.used },
  { header: "reason", value: (r) => r.reason },
];

export interface CompareCsvContext {
  exportedUtc: string;
  request: CompareRequest;
  presetEdited: boolean;
}

const NONE_TEXT = "none";
const LIST_SEPARATOR = "; ";
const COMPARE_CSV_SUFFIX = "_catalog_compare.csv";

export function compareRowsCsv(rows: readonly CompareRow[]): string {
  return buildCsv(COMPARE_CSV_COLUMNS, [...rows]);
}

function listOrNone(items: readonly string[]): string {
  return items.length === 0 ? NONE_TEXT : items.join(LIST_SEPARATOR);
}

function presetText(preset: string | null, edited: boolean): string {
  if (preset === null) return NONE_TEXT;
  return edited ? `${preset} (edited)` : preset;
}

function errorLayerText(origin: CompareBandInfo["err_origin"]): string {
  if (origin === null) return NONE_TEXT;
  return origin.assumed ? `${origin.source} (${origin.role}, assumed)` : `${origin.source} (${origin.role})`;
}

function errorColumnText(mapping: CompareMapping): string | null {
  if (mapping.flux_err !== null) return `${mapping.flux_err} (error)`;
  if (mapping.flux_ivar !== null) return `${mapping.flux_ivar} (inverse variance)`;
  return null;
}

export function compareCsvHeaderLines(report: CompareReport, context: CompareCsvContext): string[] {
  const request = context.request;
  const m = request.mapping;
  const b = report.band;
  const entries: ProvenanceEntry[] = [
    ...exportEntries(b.file_name, context.exportedUtc),
    ["catalogue_table", report.table.file_name],
    ["catalogue_format", report.table.format],
    ["catalogue_rows", report.table.n_rows],
    ["preset", presetText(report.preset, context.presetEdited)],
    ["catalogue_band", report.catalogue_band],
    ["image_filter", b.filter === null ? null : `${b.filter.dataset} ${b.filter.filter}`],
    ["image_unit", b.bunit],
    ["calibration", report.calibration_origin],
    ["jy_per_image_unit", b.jy_per_native_unit],
    ["error_layer", errorLayerText(b.err_origin)],
    ["mask", b.dq_source ?? NONE_TEXT],
    ["flagged_pixels", request.exclude_dq === true ? "excluded" : "kept"],
    ["position_columns", `${m.ra}, ${m.dec}`],
    ["id_column", m.id],
    ["flux_column", m.flux],
    ["flux_unit", m.flux_unit],
    ["zero_point", m.zero_point],
    ["error_column", errorColumnText(m)],
    ["class_column", m.class],
    ["missing_value", m.missing_value],
    ["aperture", apertureUsedText(report.aperture)],
    ["cuts", listOrNone(request.cuts.map(cutText))],
    ["min_snr", request.min_snr],
    ["max_snr", request.max_snr],
    ["min_neighbour_arcsec", request.min_neighbour_arcsec],
    ["n_table", report.n_table],
    ["n_on_image", report.n_on_image],
    ["n_after_cuts", report.n_after_cuts],
    ["n_measured", report.n_measured],
    ["n_used", report.n_used],
    ["ratio_median", report.ratio_median],
    ["ratio_median_err", report.ratio_median_err],
    ["ratio_nmad", report.ratio_nmad],
    ["n_ratio", report.n_ratio],
    ["ratio_offset_mag", report.ratio_offset_mag],
    ["err_ratio_median", report.err_ratio_median],
    ["err_ratio_nmad", report.err_ratio_nmad],
    ["n_err", report.n_err],
    ["implied_zero_point_ab", report.implied_zero_point_ab],
    ["implied_zero_point_nmad", report.implied_zero_point_nmad],
    ["n_zero_point", report.n_zero_point],
    ["rejected", listOrNone(report.rejected.map(rejectedText))],
    ...report.notes.map((note): ProvenanceEntry => ["note", note]),
    ...report.warnings.map((warning): ProvenanceEntry => ["warning", warning]),
  ];
  const nativeUnit = summedPixelUnit(b.bunit);
  const units = {
    ra_deg: "deg (ICRS)",
    dec_deg: "deg (ICRS)",
    x: "px (0-based)",
    y: "px (0-based)",
    flux_cat_ujy: "uJy",
    flux_err_cat_ujy: "uJy",
    snr_cat: "catalogue flux over catalogue error",
    flux_native: nativeUnit,
    flux_err_native: nativeUnit,
    flux_ujy: "uJy",
    flux_err_ujy: "uJy",
    ratio: "flux_ujy / flux_cat_ujy",
    masked_fraction: "fraction of the aperture area without data",
  };
  return [...provenanceLines(entries), ...unitLines(units)];
}

export function compareCsvText(report: CompareReport, headerLines: readonly string[]): string {
  return withProvenance(headerLines, compareRowsCsv(report.rows), CSV_LINE_END);
}

export function compareCsvFileName(imagePath: string | null): string {
  return `${exportStem(imagePath ?? "", "image")}${COMPARE_CSV_SUFFIX}`;
}

export function compareMarkRows(rows: readonly CompareRow[]): { placed: PlacedCatalogRow[]; usedIds: ReadonlySet<string> } {
  const placed: PlacedCatalogRow[] = [];
  const usedIds = new Set<string>();
  for (const row of rows) {
    if (!isFiniteNumber(row.x) || !isFiniteNumber(row.y)) continue;
    placed.push({
      id: row.id,
      ra: row.ra_deg,
      dec: row.dec_deg,
      ra_epoch: row.ra_deg,
      dec_epoch: row.dec_deg,
      pm_ra_masyr: null,
      pm_dec_masyr: null,
      g: null,
      bp: null,
      rp: null,
      bp_rp: null,
      parallax_mas: null,
      x: row.x,
      y: row.y,
      on_image: true,
    });
    if (row.used) usedIds.add(row.id);
  }
  return { placed, usedIds };
}
