import type { RegionSpectrum } from "../shared/types/cube";
import type { Region, RegionShape } from "../shared/types/regions";
import type {
  CorrectionFrame,
  RadialVelocityCorrectionResult,
  SpectralAxisInfo,
  SpectralAxisMode,
  SpectrumSource,
  VelocityConvention,
} from "../shared/types/spectral";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { spectrumSourceLabel } from "./lineMeasure";
import { shapeSummary } from "./regionGeometry";
import { applyCorrectionKms, defaultRestUm, formatAxis, wavelengthFromVelocityUm } from "./spectralAxis";

export type SpectrumView = "sum" | "mean" | "jy";

export interface AxisColumn {
  values: number[] | null;
  label: string;
  unit: string;
  header: string;
}

export type RestOrigin = "header" | "user";

export type VacuumAxis =
  | { values: number[]; restUm: number | null; restOrigin: RestOrigin | null; reason: null }
  | { values: null; restUm: null; restOrigin: null; reason: string };

export interface SpectrumExportInput {
  fileName: string;
  axis: SpectralAxisInfo | null;
  mode: SpectralAxisMode;
  restUm: number | null;
  convention: VelocityConvention;
  correction: CorrectionFrame;
  correctionResult: RadialVelocityCorrectionResult | null;
  source: SpectrumSource;
  regionId: string | null;
  regionText: string | null;
  backgroundId: string | null;
  view: SpectrumView;
  bunit: string | null;
  values: number[];
  region: RegionSpectrum | null;
  pixelFluxJy: number[] | null;
  pixelFluxJyError: string | null;
  sky: { ra: number; dec: number } | null;
  exportedAtUtc: string;
}

export const CHANNEL_AXIS_LABEL = "Channel";
export const CHANNEL_AXIS_UNIT = "ch";
export const NO_JY_REASON = "no Jy calibration (BUNIT is not MJy/sr)";
export const SKY_FRAME_LABEL = "ICRS";

const CHANNEL_HEADER = "channel";
const VACUUM_HEADER = "wavelength_vacuum_um";
const NATIVE_FLUX_UNIT = "native";
const REGION_SUM_UNIT_SUFFIX = " x pix";
const JY_UNIT = "Jy";
const NOT_STATED = "not stated";
const AS_STORED = "as stored";
const NO_METHOD = "-";
const EMPTY_FIELD = "-";
const DEFAULT_CSV_NAME = "spectrum.csv";
const UNIT_SLUGS: Record<string, string> = { "μm": "um", "km/s": "kms", GHz: "ghz", ch: "" };
const IMAGE_NOTE = "loaded cube; extracted from the cube on disk, not from the processed step";
const SKY_NOTE = "no WCS or off-sky position";
const VELOCITY_FRAME_NOTE = "the shift is applied to velocity columns only; wavelength_vacuum_um is as stored";
const REGION_JY_BASIS = "aperture sum x PIXAR_SR x 1e6 (total over the region, also in the mean view)";
const PIXEL_JY_BASIS = "pixel value x PIXAR_SR x 1e6";

export function numericSeries(values: readonly unknown[]): number[] {
  return values.map((v) => (typeof v === "number" ? v : NaN));
}

function slug(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

export function axisColumnHeader(label: string, unit: string): string {
  const base = slug(label.replace(/\([^)]*\)/g, ""));
  const unitSlug = unit in UNIT_SLUGS ? UNIT_SLUGS[unit] : slug(unit);
  return [base, unitSlug].filter((part) => part !== "").join("_") || "axis";
}

export function comparisonAxis(
  axis: SpectralAxisInfo | null,
  mode: SpectralAxisMode,
  restUm: number | null,
  convention: VelocityConvention,
  correction: CorrectionFrame,
  correctionResult: RadialVelocityCorrectionResult | null,
  channelCount: number,
): AxisColumn {
  const own = formatAxis(axis, mode, restUm, convention);
  if (own && own.values.length === channelCount) {
    const values = mode === "velocity" ? applyCorrectionKms(own.values, correctionResult, correction) : own.values;
    const label =
      mode === "velocity" && correction !== "none" && correctionResult ? `${own.label}, ${correction}` : own.label;
    return { values, label, unit: own.unit, header: axisColumnHeader(label, own.unit) };
  }
  return {
    values: null,
    label: CHANNEL_AXIS_LABEL,
    unit: CHANNEL_AXIS_UNIT,
    header: axisColumnHeader(CHANNEL_AXIS_LABEL, CHANNEL_AXIS_UNIT),
  };
}

function absentVacuum(reason: string): VacuumAxis {
  return { values: null, restUm: null, restOrigin: null, reason };
}

function kindConvention(kind: "vrad" | "vopt" | "velo"): VelocityConvention {
  switch (kind) {
    case "vrad":
      return "radio";
    case "vopt":
      return "optical";
    case "velo":
      return "relativistic";
  }
}

export function vacuumUmAxis(
  axis: SpectralAxisInfo | null,
  restUm: number | null,
  convention: VelocityConvention,
  channelCount: number,
): VacuumAxis {
  if (!axis) return absentVacuum("no spectral axis");
  const mismatch = (length: number) => absentVacuum(`axis has ${length} channels, spectrum has ${channelCount}`);
  if (axis.kind === "wave" || axis.kind === "awav" || axis.kind === "freq") {
    const formatted = formatAxis(axis, "wavelength_vac", null, convention);
    if (!formatted) return absentVacuum(`axis kind ${axis.kind} has no vacuum wavelength`);
    if (formatted.values.length !== channelCount) return mismatch(formatted.values.length);
    return { values: formatted.values, restUm: null, restOrigin: null, reason: null };
  }
  if (axis.kind === "vrad" || axis.kind === "vopt" || axis.kind === "velo") {
    const headerRest = defaultRestUm(axis);
    const typedRest = restUm !== null && Number.isFinite(restUm) && restUm > 0 ? restUm : null;
    const rest = headerRest ?? typedRest;
    if (rest === null) return absentVacuum("velocity axis without a rest wavelength (RESTFRQ/RESTWAV absent, none typed)");
    if (axis.values.length !== channelCount) return mismatch(axis.values.length);
    const convertWith = kindConvention(axis.kind);
    return {
      values: axis.values.map((v) => wavelengthFromVelocityUm(v, rest, convertWith)),
      restUm: rest,
      restOrigin: headerRest !== null ? "header" : "user",
      reason: null,
    };
  }
  return absentVacuum(`axis kind ${axis.kind} has no vacuum wavelength`);
}

interface AppliedFrame {
  frame: string;
  shift: number;
  method: string;
}

function appliedFrame(
  mode: SpectralAxisMode,
  correction: CorrectionFrame,
  correctionResult: RadialVelocityCorrectionResult | null,
): AppliedFrame {
  if (mode !== "velocity" || correction === "none" || correctionResult === null) {
    return { frame: AS_STORED, shift: 0, method: NO_METHOD };
  }
  const shift = correction === "barycentric" ? correctionResult.barycentric_kms : correctionResult.heliocentric_kms;
  return { frame: correction, shift, method: correctionResult.method };
}

export function frameLines(
  specsys: string | null,
  mode: SpectralAxisMode,
  correction: CorrectionFrame,
  correctionResult: RadialVelocityCorrectionResult | null,
): string[] {
  const applied = appliedFrame(mode, correction, correctionResult);
  return [
    `stored_frame: ${specsys ?? NOT_STATED}`,
    `velocity_frame: ${applied.frame}; shift_kms: ${applied.shift}; method: ${applied.method}; ${VELOCITY_FRAME_NOTE}`,
  ];
}

export function vacuumLines(vacuum: VacuumAxis): string[] {
  const lines: string[] = [];
  if (vacuum.restUm !== null) lines.push(`rest_um: ${vacuum.restUm} (${vacuum.restOrigin})`);
  if (vacuum.values === null) lines.push(`wavelength_note: ${vacuum.reason}`);
  return lines;
}

export function fluxUnitLabel(bunit: string | null, view: SpectrumView, kind: "pixel" | "region"): string {
  const trimmed = (bunit ?? "").trim().replace(/^'+|'+$/g, "").trim();
  const base = trimmed === "" ? NATIVE_FLUX_UNIT : trimmed;
  if (view === "jy") return JY_UNIT;
  if (view === "mean" || kind === "pixel") return base;
  return `${base}${REGION_SUM_UNIT_SUFFIX}`;
}

export function hasArea(shape: RegionShape): boolean {
  return shape.shape === "circle" || shape.shape === "box" || shape.shape === "ellipse" || shape.shape === "polygon";
}

export function linkedAnnulus(regions: Region[], region: Region): RegionShape | null {
  if (!region.backgroundId) return null;
  const linked = regions.find((r) => r.id === region.backgroundId) ?? null;
  return linked && linked.shape.shape === "annulus" ? linked.shape : null;
}

export function comparisonCandidates(regions: Region[]): Region[] {
  return regions.filter((r) => r.props.include && hasArea(r.shape));
}

export function regionForShape(regions: Region[], shape: RegionShape): Region | null {
  const key = JSON.stringify(shape);
  return regions.find((r) => JSON.stringify(r.shape) === key) ?? null;
}

export function regionForSource(regions: Region[], source: SpectrumSource): Region | null {
  return source.kind === "region" ? regionForShape(regions, source.shape) : null;
}

export function regionLabel(region: Region, ordinal: number): string {
  const text = region.props.text?.trim() ?? "";
  return text !== "" ? text : `R${ordinal}`;
}

export function fileBaseName(filePath: string): string {
  return filePath.split(/[\\/]/).pop() ?? filePath;
}

function fileStem(filePath: string | null): string {
  const withoutFragment = (filePath ?? "").split("#")[0];
  const base = withoutFragment.split(/[\\/]/).pop() ?? "";
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(0, dot) : base;
}

export function spectrumCsvFileName(filePath: string | null, source: SpectrumSource): string {
  const stem = fileStem(filePath);
  if (stem === "") return DEFAULT_CSV_NAME;
  return source.kind === "pixel" ? `${stem}_spectrum_pixel_${source.x}_${source.y}.csv` : `${stem}_spectrum_region.csv`;
}

type JyColumn = { values: number[]; reason: null } | { values: null; reason: string };

interface ExportPlan {
  values: number[];
  n: number;
  vacuum: VacuumAxis;
  displayed: AxisColumn;
  jy: JyColumn;
  unit: string;
  applied: AppliedFrame;
}

function jyColumn(input: SpectrumExportInput, n: number): JyColumn {
  const raw = input.source.kind === "region" ? (input.region?.flux_jy ?? null) : input.pixelFluxJy;
  if (raw === null) {
    if (input.source.kind === "pixel" && input.pixelFluxJyError !== null) {
      return { values: null, reason: `Jy refetch failed: ${input.pixelFluxJyError}` };
    }
    return { values: null, reason: NO_JY_REASON };
  }
  const values = numericSeries(raw);
  if (values.length !== n) return { values: null, reason: `flux_jy has ${values.length} channels, spectrum has ${n}` };
  return { values, reason: null };
}

function exportPlan(input: SpectrumExportInput): ExportPlan {
  const values = numericSeries(input.values);
  const n = values.length;
  return {
    values,
    n,
    vacuum: vacuumUmAxis(input.axis, input.restUm, input.convention, n),
    displayed: comparisonAxis(input.axis, input.mode, input.restUm, input.convention, input.correction, input.correctionResult, n),
    jy: jyColumn(input, n),
    unit: fluxUnitLabel(input.bunit, input.view, input.source.kind),
    applied: appliedFrame(input.mode, input.correction, input.correctionResult),
  };
}

function provenanceLine(key: string, value: string): string {
  return value === "" ? `# ${key}:` : `# ${key}: ${value}`;
}

function sourceLine(input: SpectrumExportInput): string {
  const source = input.source;
  const where = source.kind === "pixel" ? `pixel (${source.x}, ${source.y})` : shapeSummary(source.shape);
  const text = input.regionText === null ? EMPTY_FIELD : input.regionText.replace(/[\r\n]+/g, " ");
  const background =
    source.kind === "region" && source.background
      ? `annulus ${input.backgroundId ?? EMPTY_FIELD} (${shapeSummary(source.background)})`
      : "none";
  return `${spectrumSourceLabel(source)}; ${where}; region: ${input.regionId ?? EMPTY_FIELD}; text: ${text}; background: ${background}`;
}

function skyLines(sky: { ra: number; dec: number } | null): string[] {
  const usable = sky !== null && Number.isFinite(sky.ra) && Number.isFinite(sky.dec);
  const lines = [
    provenanceLine("sky_frame", SKY_FRAME_LABEL),
    provenanceLine("sky_ra_deg", usable ? String(sky.ra) : ""),
    provenanceLine("sky_dec_deg", usable ? String(sky.dec) : ""),
  ];
  if (!usable) lines.push(provenanceLine("sky_note", SKY_NOTE));
  return lines;
}

function provenanceWithPlan(input: SpectrumExportInput, plan: ExportPlan): string[] {
  const lines = [
    provenanceLine("file", input.fileName),
    provenanceLine("image", `${input.fileName} (${IMAGE_NOTE})`),
    provenanceLine("source", sourceLine(input)),
    ...skyLines(input.sky),
    ...frameLines(input.axis?.specsys ?? null, input.mode, input.correction, input.correctionResult).map((line) => `# ${line}`),
    ...vacuumLines(plan.vacuum).map((line) => `# ${line}`),
    provenanceLine("view", input.view),
    provenanceLine("flux_unit", plan.unit),
  ];
  if (plan.jy.values !== null) {
    lines.push(provenanceLine("flux_jy_basis", input.source.kind === "region" ? REGION_JY_BASIS : PIXEL_JY_BASIS));
  } else {
    lines.push(provenanceLine("flux_jy_note", plan.jy.reason));
  }
  lines.push(provenanceLine("exported_utc", input.exportedAtUtc));
  return lines;
}

export function spectrumProvenance(input: SpectrumExportInput): string[] {
  return provenanceWithPlan(input, exportPlan(input));
}

export function spectrumCsv(input: SpectrumExportInput): string {
  const plan = exportPlan(input);
  const columns: CsvColumn<number>[] = [{ header: CHANNEL_HEADER, value: (i) => i }];
  const vacuumValues = plan.vacuum.values;
  if (vacuumValues !== null) columns.push({ header: VACUUM_HEADER, value: (i) => vacuumValues[i] });
  const displayedValues = plan.displayed.values;
  if (displayedValues !== null && plan.displayed.header !== CHANNEL_HEADER) {
    columns.push({ header: plan.displayed.header, value: (i) => displayedValues[i] });
  }
  columns.push({ header: "flux", value: (i) => plan.values[i] });
  columns.push({ header: "flux_unit", value: () => plan.unit });
  const jyValues = plan.jy.values;
  if (jyValues !== null) columns.push({ header: "flux_jy", value: (i) => jyValues[i] });
  const region = input.source.kind === "region" ? input.region : null;
  if (region) {
    columns.push({ header: "npix", value: () => region.npix });
    columns.push({ header: "bg_subtracted", value: () => region.bg_subtracted });
    columns.push({ header: "n_bg", value: () => region.n_bg });
  }
  const rows = Array.from({ length: plan.n }, (_, i) => i);
  return provenanceWithPlan(input, plan).join(CSV_LINE_END) + CSV_LINE_END + buildCsv(columns, rows);
}

function finiteMax(values: number[]): number | null {
  let max: number | null = null;
  for (const v of values) {
    if (Number.isFinite(v) && (max === null || v > max)) max = v;
  }
  return max;
}

function finiteSum(values: number[]): number {
  let sum = 0;
  for (const v of values) if (Number.isFinite(v)) sum += v;
  return sum;
}

export function spectrumExportSummary(input: SpectrumExportInput): {
  params: Record<string, string | number | boolean | null>;
  values: Record<string, number | string | null>;
} {
  const plan = exportPlan(input);
  const region = input.source.kind === "region" ? input.region : null;
  return {
    params: {
      source: spectrumSourceLabel(input.source),
      view: input.view,
      mode: input.mode,
      convention: input.convention,
      correction: input.correction,
      stored_frame: input.axis?.specsys ?? NOT_STATED,
      velocity_frame: plan.applied.frame,
      shift_kms: plan.applied.shift,
      rest_um: plan.vacuum.restUm,
      rest_origin: plan.vacuum.restOrigin,
      flux_unit: plan.unit,
      n_channels: plan.n,
      npix: region?.npix ?? null,
      n_bg: region?.n_bg ?? null,
      bg_subtracted: region?.bg_subtracted ?? null,
    },
    values: {
      ra_deg: input.sky?.ra ?? null,
      dec_deg: input.sky?.dec ?? null,
      flux_max: finiteMax(plan.values),
      flux_sum: finiteSum(plan.values),
      flux_jy_max: plan.jy.values !== null ? finiteMax(plan.jy.values) : null,
    },
  };
}

export async function saveCsvDialog(text: string, defaultPath: string, title: string): Promise<string | null> {
  const { save } = await import("@tauri-apps/plugin-dialog");
  const target = await save({ defaultPath, filters: [{ name: "CSV", extensions: ["csv"] }], title });
  if (!target) return null;
  const { writeTextFile } = await import("@tauri-apps/plugin-fs");
  await writeTextFile(target, text);
  return target;
}
