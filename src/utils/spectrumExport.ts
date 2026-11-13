import type { RegionSpectrum } from "../shared/types/cube";
import type { Region, RegionShape } from "../shared/types/regions";
import type {
  CorrectionFrame,
  LineMeasurementSource,
  RadialVelocityCorrectionResult,
  SpectralAxisInfo,
  SpectralAxisMode,
  SpectrumSource,
  VelocityConvention,
} from "../shared/types/spectral";
import type { FluxDisplayChoice, Spectrum1d, WavelengthDisplayUnit } from "../shared/types/spectrum1d";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { provenanceLines, unitLines, type ProvenanceEntry } from "./csvProvenance";
import { spectrumSourceLabel } from "./lineMeasure";
import { shapeSummary } from "./regionGeometry";
import { documentLayout } from "./spectrumDocument";
import { regionViewErrors, windowsText, type RegionViewErrors } from "./regionErrors";
import { AIR_VACUUM_FORMULA, applyCorrectionKms, defaultRestUm, formatAxis, wavelengthFromVelocityUm } from "./spectralAxis";
import {
  fluxDisplay,
  scaleWavelengths,
  wavelengthUnitApplies,
  WAVELENGTH_DISPLAY_UNITS,
  WAVELENGTH_UNIT_ASCII,
  WAVELENGTH_UNIT_LABEL,
  type FluxDisplay,
} from "./spectrumUnits";
import { binAxis, binSpectrum, effectiveBinFactor, BINNING_BASIS, type BinnedSpectrum } from "./spectrumBinning";

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
  source: LineMeasurementSource;
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
  fileSpectrum?: Spectrum1d | null;
  software?: string | null;
  display?: SpectrumExportDisplay | null;
}

export interface SpectrumExportDisplay {
  wavelengthUnit: WavelengthDisplayUnit;
  fluxUnit: FluxDisplayChoice;
  binFactor: number;
  dqExcluded: boolean[] | null;
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
const UNIT_SLUGS: Record<string, string> = { "μm": "um", "km/s": "kms", GHz: "ghz", ch: "", nm: "nm", "Å": "angstrom" };
const IMAGE_NOTE = "loaded cube; extracted from the cube on disk, not from the processed step";
const SKY_NOTE = "no WCS or off-sky position";
const VELOCITY_FRAME_NOTE = "the shift is applied to velocity columns only; wavelength_vacuum_um is as stored";
const REGION_JY_BASIS = "aperture sum x PIXAR_SR x 1e6 (total over the region, also in the mean view)";
const PIXEL_JY_BASIS = "pixel value x PIXAR_SR x 1e6";
const ERR_HEADER = "err";
const ERR_SCALED_HEADER = "err_scaled";
const ERR_BASIS: Record<SpectrumView, string> = {
  sum: "the aperture sum",
  mean: "the aperture mean (mean_err = aperture-sum error / used weight)",
  jy: "the aperture sum in Jy",
};
const ERR_SCALED_BASIS = "err_scaled = err x the noise factor";
const DQ_HEADER = "dq";
const DQ_TABLE_KEY = "dq_table";
const FILE_IMAGE_NOTE = "spectrum file, read from disk";
const FILE_SKY_NOTE = "spectrum file: no sky position is read for a 1D spectrum";
const FILE_JY_REASON = "spectrum file: flux exported in its own unit (flux_unit)";
const AIR_HEADER = "wavelength_air_um";
const AIR_NOTE_KEY = "wavelength_air_note";
const AIR_NOTE = `${VACUUM_HEADER} converted from the spectrum's air wavelengths with ${AIR_VACUUM_FORMULA}; ${AIR_HEADER} holds the spectrum's own air axis`;
const PRECISION_KEY = "precision";
const F32_WIDENED = "f32_widened";
const INSTRUMENT_CARDS: readonly (readonly [string, readonly string[]])[] = [
  ["instrument", ["INSTRUME", "instrument"]],
  ["disperser", ["GRATING", "disperser"]],
  ["filter", ["FILTER", "filter"]],
  ["detector", ["DETECTOR", "detector"]],
  ["telescope", ["TELESCOP", "telescope"]],
  ["target", ["TARGNAME", "OBJECT", "target"]],
  ["source_type", ["SRCTYPE", "source_type"]],
  ["date_obs", ["DATE-OBS", "date_obs"]],
];

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

export function fluxUnitLabel(bunit: string | null, view: SpectrumView, kind: LineMeasurementSource["kind"]): string {
  const trimmed = (bunit ?? "").trim().replace(/^'+|'+$/g, "").trim();
  const base = trimmed === "" ? NATIVE_FLUX_UNIT : trimmed;
  if (view === "jy") return JY_UNIT;
  if (view === "mean" || kind !== "region") return base;
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

export { fileBaseName } from "./values";

function fileStem(filePath: string | null): string {
  const withoutFragment = (filePath ?? "").split("#")[0];
  const base = withoutFragment.split(/[\\/]/).pop() ?? "";
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(0, dot) : base;
}

export function spectrumCsvFileName(
  filePath: string | null,
  source: LineMeasurementSource,
  display: SpectrumExportDisplay | null = null,
): string {
  const stem = fileStem(filePath);
  if (stem === "") return DEFAULT_CSV_NAME;
  if (source.kind === "pixel") return `${stem}_spectrum_pixel_${source.x}_${source.y}.csv`;
  if (source.kind === "region") return `${stem}_spectrum_region.csv`;
  const factor = display?.binFactor ?? 1;
  return Number.isInteger(factor) && factor > 1 ? `${stem}_spectrum_bin${factor}.csv` : `${stem}_spectrum.csv`;
}

type JyColumn = { values: number[]; reason: null } | { values: null; reason: string };

interface Binning {
  bins: BinnedSpectrum;
  appliedDqTable: string | null;
}

interface ExportPlan {
  values: number[];
  n: number;
  vacuum: VacuumAxis;
  air: number[] | null;
  displayed: AxisColumn;
  wavelengthUnit: WavelengthDisplayUnit | null;
  jy: JyColumn;
  unit: string;
  applied: AppliedFrame;
  regionErrors: RegionViewErrors | null;
  err: number[] | null;
  dq: { values: number[]; table: string } | null;
  binning: Binning | null;
  fluxView: FluxDisplay | null;
  fluxNote: string | null;
}

interface ExportColumn extends CsvColumn<number> {
  unit: string | null;
}

function viewAxis(input: SpectrumExportInput, own: AxisColumn): Pick<ExportPlan, "displayed" | "wavelengthUnit"> {
  const unit = input.display?.wavelengthUnit ?? "um";
  if (unit === "um" || own.values === null || !wavelengthUnitApplies(input.mode, own.unit)) {
    return { displayed: own, wavelengthUnit: null };
  }
  const unitLabel = WAVELENGTH_UNIT_LABEL[unit];
  const label = own.label.replace(`(${WAVELENGTH_UNIT_LABEL.um})`, `(${unitLabel})`);
  return {
    displayed: { values: scaleWavelengths(own.values, unit), label, unit: unitLabel, header: axisColumnHeader(label, unitLabel) },
    wavelengthUnit: unit,
  };
}

function viewFlux(
  input: SpectrumExportInput,
  values: number[],
  err: number[] | null,
  vacuum: VacuumAxis,
): Pick<ExportPlan, "fluxView" | "fluxNote"> {
  const spectrum = input.source.kind === "file" ? (input.fileSpectrum ?? null) : null;
  const choice = input.display?.fluxUnit ?? "native";
  if (spectrum === null || choice === "native") return { fluxView: null, fluxNote: null };
  const shown = fluxDisplay({ flux: values, err, vacuumUm: vacuum.values, fluxUnit: spectrum.flux_unit }, choice);
  return shown.choice === choice ? { fluxView: shown, fluxNote: null } : { fluxView: null, fluxNote: shown.reason };
}

function jyColumn(input: SpectrumExportInput, n: number): JyColumn {
  if (input.source.kind === "file") return { values: null, reason: FILE_JY_REASON };
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

function fileAirAxis(input: SpectrumExportInput, n: number): number[] | null {
  const axis = input.axis;
  if (input.source.kind !== "file" || axis === null || axis.kind !== "awav") return null;
  return axis.values.length === n ? axis.values : null;
}

function regionErrorsOf(input: SpectrumExportInput, n: number): RegionViewErrors | null {
  const region = input.source.kind === "region" ? input.region : null;
  if (region === null || region.err === null) return null;
  const errors = regionViewErrors(region, input.view);
  if (errors.err === null || errors.err.length !== n) return null;
  return errors.scaled !== null && errors.scaled.length !== n ? { err: errors.err, scaled: null } : errors;
}

function binningOf(input: SpectrumExportInput, values: number[]): Binning | null {
  const spectrum = input.source.kind === "file" ? (input.fileSpectrum ?? null) : null;
  const display = input.display ?? null;
  if (spectrum === null || display === null) return null;
  const factor = effectiveBinFactor(display.binFactor, values.length);
  if (factor <= 1) return null;
  const mask = display.dqExcluded !== null && display.dqExcluded.length === values.length ? display.dqExcluded : null;
  return { bins: binSpectrum(values, spectrum.err, mask, factor), appliedDqTable: mask === null ? null : spectrum.dq_table };
}

function binnedValues(values: number[], binning: Binning | null): number[] {
  return binning === null ? values : binAxis(values, binning.bins.factor);
}

function exportPlan(input: SpectrumExportInput): ExportPlan {
  const channels = numericSeries(input.values);
  const n = channels.length;
  const channelVacuum = vacuumUmAxis(input.axis, input.restUm, input.convention, n);
  const channelAir = fileAirAxis(input, n);
  const { displayed, wavelengthUnit } = viewAxis(
    input,
    comparisonAxis(input.axis, input.mode, input.restUm, input.convention, input.correction, input.correctionResult, n),
  );
  const binning = binningOf(input, channels);
  const values = binning === null ? channels : binning.bins.flux;
  const vacuum: VacuumAxis =
    channelVacuum.values === null ? channelVacuum : { ...channelVacuum, values: binnedValues(channelVacuum.values, binning) };
  const err = binning === null ? fileErr(input) : binning.bins.err;
  return {
    values,
    n,
    vacuum,
    air: channelAir === null ? null : binnedValues(channelAir, binning),
    displayed: displayed.values === null ? displayed : { ...displayed, values: binnedValues(displayed.values, binning) },
    wavelengthUnit,
    jy: jyColumn(input, n),
    unit: fluxUnitLabel(input.bunit, input.view, input.source.kind),
    applied: appliedFrame(input.mode, input.correction, input.correctionResult),
    regionErrors: regionErrorsOf(input, n),
    err,
    dq: binning === null ? fileDq(input) : null,
    binning,
    ...viewFlux(input, values, err, vacuum),
  };
}

function provenanceLine(key: string, value: string): string {
  return value === "" ? `# ${key}:` : `# ${key}: ${value}`;
}

function sourceWhere(input: SpectrumExportInput): string {
  const source = input.source;
  if (source.kind === "pixel") return `pixel (${source.x}, ${source.y})`;
  if (source.kind === "region") return shapeSummary(source.shape);
  return input.fileSpectrum ? documentLayout(input.fileSpectrum.source) : EMPTY_FIELD;
}

function sourceLine(input: SpectrumExportInput): string {
  const source = input.source;
  const where = sourceWhere(input);
  const text = input.regionText === null ? EMPTY_FIELD : input.regionText.replace(/[\r\n]+/g, " ");
  const background =
    source.kind === "region" && source.background
      ? `annulus ${input.backgroundId ?? EMPTY_FIELD} (${shapeSummary(source.background)})`
      : "none";
  return `${spectrumSourceLabel(source)}; ${where}; region: ${input.regionId ?? EMPTY_FIELD}; text: ${text}; background: ${background}`;
}

function skyLines(sky: { ra: number; dec: number } | null, note: string): string[] {
  const usable = sky !== null && Number.isFinite(sky.ra) && Number.isFinite(sky.dec);
  const lines = [
    provenanceLine("sky_frame", SKY_FRAME_LABEL),
    provenanceLine("sky_ra_deg", usable ? String(sky.ra) : ""),
    provenanceLine("sky_dec_deg", usable ? String(sky.dec) : ""),
  ];
  if (!usable) lines.push(provenanceLine("sky_note", note));
  return lines;
}

function cardText(text: string): string {
  return text.trim().replace(/^'+|'+$/g, "").trim();
}

function instrumentLines(input: SpectrumExportInput): string[] {
  const cards = input.source.kind === "file" ? (input.fileSpectrum?.cards ?? null) : null;
  if (cards === null) return [];
  const entries: ProvenanceEntry[] = [];
  for (const [key, names] of INSTRUMENT_CARDS) {
    const text = names.map((name) => cardText(cards[name] ?? "")).find((value) => value !== "");
    if (text !== undefined) entries.push([key, text]);
  }
  return provenanceLines(entries);
}

function displayLines(plan: ExportPlan): string[] {
  if (plan.fluxView !== null) {
    return provenanceLines([
      ["display_flux_unit", plan.fluxView.unit],
      ["display_flux_basis", plan.fluxView.basis],
    ]);
  }
  return plan.fluxNote === null ? [] : provenanceLines([["display_flux_note", plan.fluxNote]]);
}

function binningLines(plan: ExportPlan): string[] {
  const binning = plan.binning;
  if (binning === null) return [];
  const bins = binning.bins;
  const count = bins.flux.length;
  const lastWidth = bins.last[count - 1] - bins.first[count - 1] + 1;
  const shortLast = lastWidth < bins.factor ? `, the last bin holds ${lastWidth}` : "";
  const used = bins.nUsed.reduce((total, n) => total + n, 0);
  const dq = binning.appliedDqTable === null ? "not applied" : `${binning.appliedDqTable} exclusions applied before binning`;
  return provenanceLines([
    ["binning", `factor ${bins.factor}, ${count} ${count === 1 ? "bin" : "bins"} of ${bins.channels} channels${shortLast}; ${BINNING_BASIS}`],
    ["binning_samples", `${used} used, ${bins.nExcluded} excluded by DQ, ${bins.nNonFinite} not finite`],
    ["binning_dq", dq],
  ]);
}

function provenanceWithPlan(input: SpectrumExportInput, plan: ExportPlan, columns: ExportColumn[]): string[] {
  const own = [...exportLines(input, plan), ...instrumentLines(input), ...binningLines(plan), ...displayLines(plan)];
  const software = (input.software ?? "").trim();
  if (software === "") {
    return [provenanceLine("file", input.fileName), ...own, provenanceLine("exported_utc", input.exportedAtUtc)];
  }
  return [
    ...provenanceLines([
      ["software", software],
      ["exported_utc", input.exportedAtUtc],
      ["file", input.fileName],
    ]),
    ...own,
    ...unitLines(Object.fromEntries(columns.map((column) => [column.header, column.unit]))),
  ];
}

function exportLines(input: SpectrumExportInput, plan: ExportPlan): string[] {
  const isFile = input.source.kind === "file";
  const lines = [
    provenanceLine("image", `${input.fileName} (${isFile ? FILE_IMAGE_NOTE : IMAGE_NOTE})`),
    provenanceLine("source", sourceLine(input)),
    ...skyLines(input.sky, isFile ? FILE_SKY_NOTE : SKY_NOTE),
    ...frameLines(input.axis?.specsys ?? null, input.mode, input.correction, input.correctionResult).map((line) => `# ${line}`),
    ...vacuumLines(plan.vacuum).map((line) => `# ${line}`),
    ...(plan.air !== null ? [provenanceLine(AIR_NOTE_KEY, AIR_NOTE)] : []),
    provenanceLine("view", input.view),
    provenanceLine("flux_unit", plan.unit),
  ];
  if (plan.jy.values !== null) {
    lines.push(provenanceLine("flux_jy_basis", input.source.kind === "region" ? REGION_JY_BASIS : PIXEL_JY_BASIS));
  } else {
    lines.push(provenanceLine("flux_jy_note", plan.jy.reason));
  }
  lines.push(...regionErrorLines(input, plan));
  const dqTable = plan.dq?.table ?? null;
  if (dqTable !== null) lines.push(provenanceLine(DQ_TABLE_KEY, dqTable));
  if (input.fileSpectrum?.precision === F32_WIDENED) lines.push(provenanceLine(PRECISION_KEY, F32_WIDENED));
  return lines;
}

function regionErrorLines(input: SpectrumExportInput, plan: ExportPlan): string[] {
  const region = input.region;
  if (plan.regionErrors === null || region === null) return [];
  const scaled = plan.regionErrors.scaled !== null ? `; ${ERR_SCALED_BASIS}` : "";
  const lines = [
    provenanceLine("err_source", region.err_source),
    provenanceLine("err_basis", `err is the propagated error of the flux column, ${ERR_BASIS[input.view]}${scaled}`),
  ];
  const factor = region.noise_factor;
  if (factor !== null) {
    const windows = windowsText(factor.windows, " and ", "-");
    lines.push(provenanceLine("noise_factor", `${factor.factor} +- ${factor.factor_se} (${factor.n_channels} line-free channels, continuum ${windows})`));
    if (factor.warning !== null) lines.push(provenanceLine("noise_factor_warning", factor.warning));
  }
  return lines;
}

function fileErr(input: SpectrumExportInput): number[] | null {
  return input.fileSpectrum?.err ?? null;
}

function fileDq(input: SpectrumExportInput): { values: number[]; table: string } | null {
  const spectrum = input.fileSpectrum ?? null;
  return spectrum?.dq ? { values: spectrum.dq, table: spectrum.dq_table } : null;
}

function asciiUnit(unit: string): string {
  const display = WAVELENGTH_DISPLAY_UNITS.find((candidate) => WAVELENGTH_UNIT_LABEL[candidate] === unit);
  return display === undefined ? unit : WAVELENGTH_UNIT_ASCII[display];
}

function exportColumns(input: SpectrumExportInput, plan: ExportPlan): ExportColumn[] {
  const fluxUnit = plan.unit === NATIVE_FLUX_UNIT ? null : plan.unit;
  const bins = plan.binning?.bins ?? null;
  const columns: ExportColumn[] =
    bins === null
      ? [{ header: CHANNEL_HEADER, value: (i) => i, unit: null }]
      : [
          { header: "bin", value: (i) => i, unit: null },
          { header: "channel_first", value: (i) => bins.first[i], unit: null },
          { header: "channel_last", value: (i) => bins.last[i], unit: null },
          { header: "n_used", value: (i) => bins.nUsed[i], unit: null },
        ];
  const vacuumValues = plan.vacuum.values;
  if (vacuumValues !== null) columns.push({ header: VACUUM_HEADER, value: (i) => vacuumValues[i], unit: WAVELENGTH_UNIT_ASCII.um });
  const airValues = plan.air;
  if (airValues !== null) columns.push({ header: AIR_HEADER, value: (i) => airValues[i], unit: WAVELENGTH_UNIT_ASCII.um });
  const displayedValues = plan.displayed.values;
  if (displayedValues !== null && plan.displayed.header !== CHANNEL_HEADER) {
    columns.push({ header: plan.displayed.header, value: (i) => displayedValues[i], unit: asciiUnit(plan.displayed.unit) });
  }
  columns.push({ header: "flux", value: (i) => plan.values[i], unit: fluxUnit });
  columns.push({ header: "flux_unit", value: () => plan.unit, unit: null });
  const jyValues = plan.jy.values;
  if (jyValues !== null) columns.push({ header: "flux_jy", value: (i) => jyValues[i], unit: JY_UNIT });
  const region = input.source.kind === "region" ? input.region : null;
  if (region) {
    columns.push({ header: "npix", value: () => region.npix, unit: null });
    columns.push({ header: "bg_subtracted", value: () => region.bg_subtracted, unit: null });
    columns.push({ header: "n_bg", value: () => region.n_bg, unit: null });
  }
  const regionErr = plan.regionErrors?.err ?? null;
  if (regionErr !== null) columns.push({ header: ERR_HEADER, value: (i) => regionErr[i], unit: fluxUnit });
  const regionScaled = plan.regionErrors?.scaled ?? null;
  if (regionScaled !== null) columns.push({ header: ERR_SCALED_HEADER, value: (i) => regionScaled[i], unit: fluxUnit });
  const err = plan.err;
  if (err !== null) columns.push({ header: ERR_HEADER, value: (i) => err[i], unit: fluxUnit });
  const dq = plan.dq;
  if (dq !== null) columns.push({ header: DQ_HEADER, value: (i) => dq.values[i], unit: null });
  const view = plan.fluxView;
  if (view !== null) {
    columns.push({ header: "flux_display", value: (i) => view.values[i], unit: view.unit });
    const viewErrors = view.errors;
    if (err !== null && viewErrors !== null) {
      columns.push({ header: "err_display", value: (i) => viewErrors[i], unit: view.unit });
    }
  }
  return columns;
}

export function spectrumProvenance(input: SpectrumExportInput): string[] {
  const plan = exportPlan(input);
  return provenanceWithPlan(input, plan, exportColumns(input, plan));
}

export function spectrumCsv(input: SpectrumExportInput): string {
  const plan = exportPlan(input);
  const columns = exportColumns(input, plan);
  const rows = Array.from({ length: plan.values.length }, (_, i) => i);
  return provenanceWithPlan(input, plan, columns).join(CSV_LINE_END) + CSV_LINE_END + buildCsv(columns, rows);
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
  const fileParams: Record<string, string | boolean | null> =
    input.source.kind === "file"
      ? { err_column: plan.err !== null, dq_column: plan.dq !== null, dq_table: plan.dq?.table ?? null }
      : {};
  const binning = plan.binning;
  const binParams: Record<string, number | boolean> =
    binning === null
      ? {}
      : {
          bin_factor: binning.bins.factor,
          n_bins: binning.bins.flux.length,
          bin_dq_requested: binning.appliedDqTable !== null,
          bin_dq_excluded: binning.bins.nExcluded,
        };
  const regionParams: Record<string, string | number | null> = region
    ? { err_source: region.err_source, noise_factor: region.noise_factor?.factor ?? null }
    : {};
  const viewParams: Record<string, string> = {
    ...(plan.wavelengthUnit !== null ? { wavelength_unit: plan.wavelengthUnit } : {}),
    ...(plan.fluxView !== null ? { display_flux_unit: plan.fluxView.unit } : {}),
  };
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
      ...fileParams,
      ...regionParams,
      ...viewParams,
      ...binParams,
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
