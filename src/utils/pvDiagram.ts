import type { ProcessedResult } from "../shared/types/preview";
import type { PvDiagramResult, PvLine, PvOffsetUnit, PvRun, PvRunParams } from "../shared/types/pv";
import type { Region } from "../shared/types/regions";
import type {
  CorrectionFrame,
  RadialVelocityCorrectionResult,
  SpectralAxisInfo,
  SpectralAxisMode,
} from "../shared/types/spectral";
import type { ProfileSeries } from "../components/regions/ProfilePlot";
import { CSV_LINE_END, buildCsv, type CsvColumn } from "./catalogCsv";
import { exportStem } from "./imageRef";
import { planeTag } from "./exportSources";
import { availableModes } from "./spectralAxis";

export const PV_STEP_RANGE: readonly [number, number] = [0.05, 1024];
export const PV_WIDTH_RANGE: readonly [number, number] = [0, 4096];
export const DEFAULT_STEP_TEXT = "1";
export const DEFAULT_WIDTH_TEXT = "1";
export const CHANNEL_FALLBACK_MODE: SpectralAxisMode = "wavelength_vac";

const RIDGE_COLOR = "#a78bfa";
const PEAK_COLOR = "#fbbf24";
const MICRON = "μm";
const PV_LABEL_PREFIX = "PV · ";
const NO_LINE_REASON = "draw a line region on the cube";
const STEP_REASON = `step must be ${PV_STEP_RANGE[0]} to ${PV_STEP_RANGE[1]} px`;
const WIDTH_REASON = `width must be ${PV_WIDTH_RANGE[0]} to ${PV_WIDTH_RANGE[1]} px`;
const RANGE_REASON = "channel range must be two channels of the cube in order";
const REST_REASON = "velocity mode needs a rest wavelength";
const ZERO_LENGTH_REASON = "line has zero length";

export function pickLineRegion(regions: Region[], selectedId: string | null): Region | null {
  const selected = selectedId === null ? undefined : regions.find((r) => r.id === selectedId);
  if (selected && selected.shape.shape === "line") return selected;
  return regions.find((r) => r.shape.shape === "line") ?? null;
}

export function lineOf(region: Region): PvLine | null {
  const shape = region.shape;
  if (shape.shape !== "line") return null;
  return { x0: shape.x1, y0: shape.y1, x1: shape.x2, y1: shape.y2 };
}

function parseBounded(text: string, range: readonly [number, number]): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  return Number.isFinite(value) && value >= range[0] && value <= range[1] ? value : null;
}

export function parseStep(text: string): number | null {
  return parseBounded(text, PV_STEP_RANGE);
}

export function parseWidth(text: string): number | null {
  return parseBounded(text, PV_WIDTH_RANGE);
}

export function parseChannel(text: string, frames: number): number | null {
  const trimmed = text.trim();
  if (trimmed === "" || frames <= 0) return null;
  const value = Number(trimmed);
  return Number.isInteger(value) && value >= 0 && value <= frames - 1 ? value : null;
}

export interface PvRunInput {
  line: PvLine | null;
  step: number | null;
  width: number | null;
  z0: number | null;
  z1: number | null;
  mode: SpectralAxisMode;
  restUm: number | null;
  axis: SpectralAxisInfo | null;
}

function needsRest(axis: SpectralAxisInfo | null): boolean {
  return axis !== null && (axis.kind === "wave" || axis.kind === "awav" || axis.kind === "freq");
}

export function pvEffectiveMode(axis: SpectralAxisInfo | null, mode: SpectralAxisMode): SpectralAxisMode {
  const modes = availableModes(axis);
  if (modes.length === 0) return CHANNEL_FALLBACK_MODE;
  return modes.includes(mode) ? mode : modes[0];
}

export function pvRunBlocker(input: PvRunInput): string | null {
  if (input.line === null) return NO_LINE_REASON;
  if (input.step === null) return STEP_REASON;
  if (input.width === null) return WIDTH_REASON;
  if (input.z0 === null || input.z1 === null || input.z0 >= input.z1) return RANGE_REASON;
  if (input.mode === "velocity" && needsRest(input.axis) && input.restUm === null) return REST_REASON;
  const { x0, y0, x1, y1 } = input.line;
  if (Math.hypot(x1 - x0, y1 - y0) <= 0) return ZERO_LENGTH_REASON;
  return null;
}

export function pvShiftKms(
  correction: CorrectionFrame,
  result: RadialVelocityCorrectionResult | null,
  mode: SpectralAxisMode,
): number | null {
  if (mode !== "velocity" || correction === "none" || result === null) return null;
  return correction === "barycentric" ? result.barycentric_kms : result.heliocentric_kms;
}

export function pvSpectralLabel(run: PvRun): string {
  const { result, params } = run;
  if (result.spectral_unit === "ch") return "Channel";
  switch (result.spectral_mode) {
    case "wavelength_vac":
      return `Vacuum wavelength (${MICRON})`;
    case "wavelength_air":
      return `Air wavelength (${MICRON})`;
    case "frequency":
      return "Frequency (GHz)";
    case "velocity": {
      const base = result.spectral_convention === null ? "Velocity (km/s)" : `Velocity, ${result.spectral_convention} (km/s)`;
      const corrected = result.spectral_shift_kms !== 0 && params.correction !== "none";
      return corrected ? `${base}, ${params.correction}` : base;
    }
  }
}

export function offsetLabel(unit: PvOffsetUnit): string {
  return unit === "arcsec" ? "offset (arcsec)" : "offset (px)";
}

export function pvRecordLabel(params: PvRunParams, res: PvDiagramResult): string {
  return `${PV_LABEL_PREFIX}ch ${res.z0}-${res.z1} · step ${params.stepPx} px · width ${params.widthPx} px`;
}

export function isPvRecord(processed: Pick<ProcessedResult, "label"> | null): boolean {
  return processed !== null && processed.label.startsWith(PV_LABEL_PREFIX);
}

export function ridgeSeries(res: PvDiagramResult): ProfileSeries[] {
  return [
    { x: res.offsets, y: res.ridge, color: RIDGE_COLOR, label: "ridge" },
    { x: res.offsets, y: res.peak_spectral, color: PEAK_COLOR, label: "peak", mode: "points" },
  ];
}

function unitTag(unit: string): string {
  return unit.replace(/\//g, "_");
}

export function pvRidgeCsv(run: PvRun): string {
  const { params, result } = run;
  const { line } = params;
  const offsetTag = result.offset_unit === "arcsec" ? "arcsec" : "px";
  const spectralTag = unitTag(result.spectral_unit);
  const provenance = [
    `# file ${params.filePath}`,
    `# slit ${line.x0},${line.y0} -> ${line.x1},${line.y1} (px, 0-based)`,
    `# step_px ${params.stepPx}, width_px ${params.widthPx}, n_across ${result.n_across}`,
    `# channels ${result.z0}-${result.z1}`,
    `# spectral axis: ${pvSpectralLabel(run)}`,
    `# fits: ${result.fits_path}`,
  ];
  const columns: CsvColumn<number>[] = [
    { header: `offset_${offsetTag}`, value: (i) => result.offsets[i] },
    { header: `ridge_${spectralTag}`, value: (i) => result.ridge[i] },
    { header: "ridge_channel", value: (i) => result.ridge_channel[i] },
    { header: "peak_channel", value: (i) => result.peak_channel[i] },
    { header: `peak_${spectralTag}`, value: (i) => result.peak_spectral[i] },
    { header: "peak_value", value: (i) => result.peak_value[i] },
  ];
  const rows = result.offsets.map((_, i) => i);
  return provenance.join(CSV_LINE_END) + CSV_LINE_END + buildCsv(columns, rows);
}

export function pvCsvFileName(filePath: string | null): string {
  if (filePath === null) return `${exportStem("")}_pv.csv`;
  return `${exportStem(filePath)}${planeTag(filePath)}_pv.csv`;
}

export function pvOnScreen(processed: Pick<ProcessedResult, "fitsPath"> | null, run: PvRun | null): boolean {
  return processed !== null && run !== null && processed.fitsPath === run.result.fits_path;
}

export interface PvLogRecords {
  params: Record<string, string | number | null>;
  values: Record<string, string | number | null>;
  unit: string | null;
}

export function pvLogRecords(run: PvRun): PvLogRecords {
  const { params, result } = run;
  const s = result.summary;
  return {
    params: {
      region_id: params.regionId,
      x0: params.line.x0,
      y0: params.line.y0,
      x1: params.line.x1,
      y1: params.line.y1,
      step_px: params.stepPx,
      width_px: params.widthPx,
      z0: params.z0,
      z1: params.z1,
      mode: params.mode,
      rest_um: params.restUm,
      convention: params.convention,
      correction: params.correction,
      shift_kms: params.velocityShiftKms,
    },
    values: {
      ridge_gradient: s.ridge_gradient,
      ridge_gradient_unit: s.ridge_gradient_unit,
      ridge_span: s.ridge_span,
      n_ridge_valid: s.n_ridge_valid,
      peak_value: s.peak_value,
      peak_offset: s.peak_offset,
      peak_spectral: s.peak_spectral,
      slit_length: s.slit_length,
      slit_pa_deg: s.slit_pa_deg,
      offset_step: s.offset_step,
    },
    unit: s.bunit,
  };
}
