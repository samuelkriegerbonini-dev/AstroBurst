import type {
  CorrectionFrame,
  SpectralAxisInfo,
  SpectralAxisKind,
  SpectralAxisMode,
  VelocityConvention,
} from "../shared/types/spectral";
import {
  SPEED_OF_LIGHT_KMS,
  defaultRestUm,
  frequencyGhzFromWavelengthUm,
  isVelocityKind,
  vacuumToAirUm,
  velocityKms,
  wavelengthFromVelocityUm,
  wavelengthUmFromFrequencyGhz,
  type FormattedAxis,
} from "./spectralAxis";
import { axisValueToPixel, type PlotMapping } from "./spectrumRange";

export type LineFamily = "optical" | "nir" | "radio";

export const LINE_FAMILIES: readonly LineFamily[] = ["optical", "nir", "radio"];

export interface RestLine {
  id: string;
  label: string;
  family: LineFamily;
  vacuumUm: number;
  restGhz: number | null;
}

export const MAX_REDSHIFT = 100;
export const LINE_PICK_TOLERANCE_PX = 6;
export const LABEL_CHAR_PX = 5.5;
export const LABEL_GAP_PX = 4;
export const LABEL_PAD_PX = 3;

const REDSHIFT_DECIMALS = 6;
const SYSTEMIC_DECIMALS = 1;
const MICRON = "μm";
const UNSHIFTED_CORRECTION_METHOD_PREFIX = "already in";

function wavelengthLine(id: string, label: string, family: "optical" | "nir", vacuumUm: number): RestLine {
  return { id, label, family, vacuumUm, restGhz: null };
}

function radioLine(id: string, label: string, restGhz: number): RestLine {
  return { id, label, family: "radio", vacuumUm: wavelengthUmFromFrequencyGhz(restGhz), restGhz };
}

export const REST_LINES: readonly RestLine[] = [
  wavelengthLine("h_alpha", "Hα", "optical", 0.656461),
  wavelengthLine("h_beta", "Hβ", "optical", 0.486268),
  wavelengthLine("oiii_4959", "[OIII] 4959", "optical", 0.49603),
  wavelengthLine("oiii_5007", "[OIII] 5007", "optical", 0.500824),
  wavelengthLine("nii_6548", "[NII] 6548", "optical", 0.654986),
  wavelengthLine("nii_6583", "[NII] 6583", "optical", 0.658527),
  wavelengthLine("sii_6716", "[SII] 6716", "optical", 0.671829),
  wavelengthLine("sii_6731", "[SII] 6731", "optical", 0.673267),
  wavelengthLine("oii_3727", "[OII] 3727", "optical", 0.372709),
  wavelengthLine("oii_3729", "[OII] 3729", "optical", 0.372988),
  wavelengthLine("pa_alpha", "Paα", "nir", 1.875613),
  wavelengthLine("pa_beta", "Paβ", "nir", 1.282159),
  wavelengthLine("br_gamma", "Brγ", "nir", 2.16612),
  wavelengthLine("hei_1083", "He I 1.083", "nir", 1.083331),
  wavelengthLine("h2_2122", "H2 1-0 S(1)", "nir", 2.121834),
  wavelengthLine("feii_1644", "[FeII] 1.644", "nir", 1.644),
  radioLine("co_1_0", "CO 1-0", 115.2712018),
  radioLine("co_2_1", "CO 2-1", 230.538),
  radioLine("co_3_2", "CO 3-2", 345.7959899),
  radioLine("hcn_1_0", "HCN 1-0", 88.6316023),
  radioLine("hcop_1_0", "HCO+ 1-0", 89.1885247),
  radioLine("cii_158", "[CII] 158", 1900.5369),
  radioLine("hi_21cm", "HI 21 cm", 1.420405751768),
];

export function lineById(id: string): RestLine | null {
  return REST_LINES.find((line) => line.id === id) ?? null;
}

export function familyLabel(family: LineFamily): string {
  switch (family) {
    case "optical":
      return "Optical";
    case "nir":
      return "NIR";
    case "radio":
      return "Radio";
  }
}

export function conventionForKind(kind: SpectralAxisKind): VelocityConvention | null {
  switch (kind) {
    case "vrad":
      return "radio";
    case "vopt":
      return "optical";
    case "velo":
      return "relativistic";
    default:
      return null;
  }
}

export function displayConvention(axis: SpectralAxisInfo | null, convention: VelocityConvention): VelocityConvention {
  return (axis === null ? null : conventionForKind(axis.kind)) ?? convention;
}

export function velocityKmsFromRedshift(z: number, convention: VelocityConvention): number {
  return velocityKms(1 + z, 1, convention);
}

export function redshiftFromVelocityKms(velocityKmsValue: number, convention: VelocityConvention): number {
  return wavelengthFromVelocityUm(velocityKmsValue, 1, convention) - 1;
}

function redshiftInRange(z: number): boolean {
  return Number.isFinite(z) && z > -1 && z <= MAX_REDSHIFT;
}

function parseFiniteNumber(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : null;
}

export function parseRedshiftInput(text: string): number | null {
  const z = parseFiniteNumber(text);
  return z !== null && redshiftInRange(z) ? z : null;
}

export function parseSystemicInput(text: string, convention: VelocityConvention): number | null {
  const velocity = parseFiniteNumber(text);
  if (velocity === null) return null;
  const z = redshiftFromVelocityKms(velocity, convention);
  return redshiftInRange(z) ? z : null;
}

export function formatRedshift(z: number): string {
  return String(Number(z.toFixed(REDSHIFT_DECIMALS)));
}

export function formatSystemicKms(velocityKmsValue: number): string {
  return velocityKmsValue.toFixed(SYSTEMIC_DECIMALS);
}

export function resyncRedshiftText(text: string, redshift: number): string {
  const parsed = parseRedshiftInput(text);
  const shown = formatRedshift(redshift);
  return parsed !== null && formatRedshift(parsed) === shown ? text : shown;
}

export function resyncSystemicText(text: string, redshift: number, convention: VelocityConvention): string {
  const shown = formatSystemicKms(velocityKmsFromRedshift(redshift, convention));
  const keeps = parseSystemicInput(text, convention) !== null && formatSystemicKms(Number(text.trim())) === shown;
  return keeps ? text : shown;
}

export function observedVacuumUm(restVacuumUm: number, z: number): number {
  return restVacuumUm * (1 + z);
}

export function storedVacuumUm(observedUm: number, shiftKms: number | null): number {
  return observedUm / (1 + (shiftKms ?? 0) / SPEED_OF_LIGHT_KMS);
}

function isWavelengthOrFrequencyKind(axis: SpectralAxisInfo): boolean {
  return axis.kind === "wave" || axis.kind === "awav" || axis.kind === "freq";
}

function headerRestKind(axis: SpectralAxisInfo): boolean {
  return isVelocityKind(axis) || axis.kind === "zopt";
}

function wavelengthDisplayValue(
  storedUm: number,
  mode: SpectralAxisMode,
  restUm: number | null,
  convention: VelocityConvention,
  addKms: number,
): number | null {
  switch (mode) {
    case "wavelength_vac":
      return storedUm;
    case "wavelength_air":
      return vacuumToAirUm(storedUm);
    case "frequency":
      return frequencyGhzFromWavelengthUm(storedUm);
    case "velocity":
      return restUm === null || !(restUm > 0) ? null : velocityKms(storedUm, restUm, convention) + addKms;
  }
}

export function lineDisplayValue(
  observedUm: number,
  axis: SpectralAxisInfo,
  mode: SpectralAxisMode,
  restUm: number | null,
  convention: VelocityConvention,
  shiftKms: number | null,
): number | null {
  const stored = storedVacuumUm(observedUm, shiftKms);
  const addKms = shiftKms ?? 0;
  const kindConvention = conventionForKind(axis.kind);
  if (kindConvention !== null) {
    const headerRest = defaultRestUm(axis);
    return headerRest === null ? null : velocityKms(stored, headerRest, kindConvention) + addKms;
  }
  if (axis.kind === "zopt") {
    const headerRest = defaultRestUm(axis);
    return headerRest === null ? null : velocityKms(stored / headerRest, 1, convention) + addKms;
  }
  if (isWavelengthOrFrequencyKind(axis)) return wavelengthDisplayValue(stored, mode, restUm, convention, addKms);
  return null;
}

export type LineListAvailability = { ok: true } | { ok: false; reason: string };

export function lineListAvailability(
  axis: SpectralAxisInfo | null,
  mode: SpectralAxisMode,
  restUm: number | null,
  formatted: FormattedAxis | null,
  n: number,
): LineListAvailability {
  if (axis === null) {
    return { ok: false, reason: "no spectral axis for this cube: rest lines need a WAVE, AWAV, FREQ, velocity or ZOPT axis" };
  }
  if (axis.kind === "unknown") {
    return { ok: false, reason: `axis ${axis.ctype} is not a spectral axis: rest lines are hidden on the channel display` };
  }
  if (headerRestKind(axis) && defaultRestUm(axis) === null) {
    return { ok: false, reason: `no RESTFRQ/RESTWAV in the header: rest lines cannot be placed on the ${axis.ctype} axis` };
  }
  if (formatted === null) {
    if (mode === "velocity" && (restUm === null || !(restUm > 0))) {
      return { ok: false, reason: "velocity display needs a rest wavelength: type one or choose a wavelength or frequency axis" };
    }
    return { ok: false, reason: `axis ${axis.ctype} cannot be formatted: rest lines are hidden on the channel display` };
  }
  if (n === 0 || formatted.values.length !== n) {
    return {
      ok: false,
      reason: `spectral axis has ${formatted.values.length} values for a spectrum of ${n} channels: rest lines are hidden on the channel display`,
    };
  }
  return { ok: true };
}

export interface LineMark {
  id: string;
  label: string;
  family: LineFamily;
  restVacuumUm: number;
  value: number;
}

export interface LineMarksInput {
  axis: SpectralAxisInfo;
  mode: SpectralAxisMode;
  restUm: number | null;
  convention: VelocityConvention;
  shiftKms: number | null;
  redshift: number;
  families: readonly LineFamily[];
  xMin: number;
  xMax: number;
}

export function lineMarks(input: LineMarksInput): LineMark[] {
  const marks: LineMark[] = [];
  for (const line of REST_LINES) {
    if (!input.families.includes(line.family)) continue;
    const observed = observedVacuumUm(line.vacuumUm, input.redshift);
    const value = lineDisplayValue(observed, input.axis, input.mode, input.restUm, input.convention, input.shiftKms);
    if (value === null || !Number.isFinite(value) || value < input.xMin || value > input.xMax) continue;
    marks.push({ id: line.id, label: line.label, family: line.family, restVacuumUm: line.vacuumUm, value });
  }
  return marks;
}

export interface PlacedLineMark extends LineMark {
  px: number;
  labelX: number;
  labelWidth: number;
  labelShown: boolean;
}

export function placeLineMarks(
  marks: readonly LineMark[],
  m: PlotMapping,
  charPx: number = LABEL_CHAR_PX,
  gapPx: number = LABEL_GAP_PX,
): PlacedLineMark[] {
  const ordered = marks
    .map((mark) => ({ mark, px: axisValueToPixel(mark.value, m) }))
    .filter((entry) => Number.isFinite(entry.px))
    .sort((a, b) => a.px - b.px);
  const rightEdge = m.width - m.padRight;
  const placed: PlacedLineMark[] = [];
  let lastShownEnd = -Infinity;
  for (const { mark, px } of ordered) {
    const labelWidth = mark.label.length * charPx;
    const labelX = Math.min(px + LABEL_PAD_PX, rightEdge - labelWidth);
    const labelShown = labelX >= lastShownEnd + gapPx;
    if (labelShown) lastShownEnd = labelX + labelWidth;
    placed.push({ ...mark, px, labelX, labelWidth, labelShown });
  }
  return placed;
}

export function nearestLineMark(marks: readonly LineMark[], m: PlotMapping, px: number, tolerancePx: number): LineMark | null {
  let best: LineMark | null = null;
  let bestDistance = Infinity;
  for (const mark of marks) {
    const distance = Math.abs(axisValueToPixel(mark.value, m) - px);
    if (distance <= tolerancePx && distance < bestDistance) {
      best = mark;
      bestDistance = distance;
    }
  }
  return best;
}

export function lineClickSetsRest(axis: SpectralAxisInfo | null): boolean {
  return axis !== null && isWavelengthOrFrequencyKind(axis);
}

export function lineClickHint(axis: SpectralAxisInfo | null): string {
  if (lineClickSetsRest(axis)) return "click a line to set the rest wavelength (double-click away from a line to jump to a channel)";
  if (axis !== null && headerRestKind(axis)) return "rest wavelength fixed by the header (RESTFRQ/RESTWAV)";
  return "";
}

function correctionShiftsAxis(correction: CorrectionFrame, method: string | null): boolean {
  return correction !== "none" && method !== null && !method.startsWith(UNSHIFTED_CORRECTION_METHOD_PREFIX);
}

export function redshiftFrameLabel(correction: CorrectionFrame, method: string | null, specsys: string | null): string {
  if (correctionShiftsAxis(correction, method)) return `z and v_sys in the ${correction} frame`;
  return specsys ? `z and v_sys in the stored frame (SPECSYS ${specsys})` : "z and v_sys in the stored frame";
}

export function pickedLineLabel(line: RestLine): string {
  const rest = `rest ← ${line.label} ${line.vacuumUm.toFixed(6)} ${MICRON} vacuum`;
  return line.restGhz === null ? rest : `${rest} (${line.restGhz} GHz)`;
}
