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

export type LineFamily = "optical" | "nir" | "mir" | "radio";

export const LINE_FAMILIES: readonly LineFamily[] = ["optical", "nir", "mir", "radio"];

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
const BAND_EDGE_DECIMALS = 6;
const UM_PER_CM = 10000;
const MICRON = "μm";
const UNSHIFTED_CORRECTION_METHOD_PREFIX = "already in";

function wavelengthLine(id: string, label: string, family: "optical" | "nir" | "mir", vacuumUm: number): RestLine {
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
  wavelengthLine("nai_5890", "Na I 5890", "optical", 0.5891583),
  wavelengthLine("nai_5896", "Na I 5896", "optical", 0.5897558),
  wavelengthLine("ki_7665", "K I 7665", "optical", 0.7667009),
  wavelengthLine("ki_7699", "K I 7699", "optical", 0.7701084),
  wavelengthLine("pa_alpha", "Paα", "nir", 1.875613),
  wavelengthLine("pa_beta", "Paβ", "nir", 1.282159),
  wavelengthLine("br_gamma", "Brγ", "nir", 2.16612),
  wavelengthLine("br_alpha", "Brα", "nir", 4.052262),
  wavelengthLine("br_beta", "Brβ", "nir", 2.625872),
  wavelengthLine("pf_beta", "Pfβ", "nir", 4.653778),
  wavelengthLine("hei_2058", "He I 2.058", "nir", 2.05869),
  wavelengthLine("hei_1083", "He I 1.083", "nir", 1.083331),
  wavelengthLine("h2_2122", "H2 1-0 S(1)", "nir", 2.121834),
  wavelengthLine("feii_1644", "[FeII] 1.644", "nir", 1.644),
  wavelengthLine("arii_6985", "[ArII] 6.985", "mir", 6.985274),
  wavelengthLine("ariii_8991", "[ArIII] 8.991", "mir", 8.99138),
  wavelengthLine("h2_s3_9665", "H2 0-0 S(3)", "mir", 9.66492),
  wavelengthLine("siv_10511", "[SIV] 10.511", "mir", 10.5105),
  wavelengthLine("h2_s2_12279", "H2 0-0 S(2)", "mir", 12.27861),
  wavelengthLine("neii_12814", "[NeII] 12.814", "mir", 12.81355),
  wavelengthLine("nev_14322", "[NeV] 14.322", "mir", 14.3217),
  wavelengthLine("neiii_15555", "[NeIII] 15.555", "mir", 15.5551),
  wavelengthLine("h2_s1_17035", "H2 0-0 S(1)", "mir", 17.03484),
  wavelengthLine("siii_18713", "[SIII] 18.713", "mir", 18.713),
  wavelengthLine("nev_24318", "[NeV] 24.318", "mir", 24.3175),
  wavelengthLine("oiv_25890", "[OIV] 25.890", "mir", 25.8903),
  wavelengthLine("feii_25988", "[FeII] 25.988", "mir", 25.98829),
  wavelengthLine("h2_s0_28219", "H2 0-0 S(0)", "mir", 28.21883),
  wavelengthLine("siii_33481", "[SIII] 33.481", "mir", 33.481),
  wavelengthLine("si_ii_34815", "[SiII] 34.815", "mir", 34.8152),
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

export type RestBandKind = "molecular_band" | "atomic_line" | "instrument";

export interface RestBand {
  id: string;
  label: string;
  species: string;
  kind: RestBandKind;
  centreVacuumUm: number;
  loVacuumUm: number;
  hiVacuumUm: number;
  source: string;
}

function molecularBand(
  id: string,
  label: string,
  species: string,
  originPerCm: number,
  loVacuumUm: number,
  hiVacuumUm: number,
  source: string,
): RestBand {
  return { id, label, species, kind: "molecular_band", centreVacuumUm: UM_PER_CM / originPerCm, loVacuumUm, hiVacuumUm, source };
}

function atomicBand(
  id: string,
  label: string,
  species: string,
  centreVacuumUm: number,
  loVacuumUm: number,
  hiVacuumUm: number,
  source: string,
): RestBand {
  return { id, label, species, kind: "atomic_line", centreVacuumUm, loVacuumUm, hiVacuumUm, source };
}

export const REST_BANDS: readonly RestBand[] = [
  molecularBand("h2o_2p7", "H2O 2.7", "H2O", 3756, 2.5, 3.0, "NIST WebBook C7732185: nu3 3756 cm-1 (2.662 um), nu1 3657 cm-1 (2.734 um); display range editorial"),
  molecularBand("h2o_6p3", "H2O 6.3", "H2O", 1595, 5.5, 7.5, "NIST WebBook C7732185: nu2 1595 cm-1 (6.270 um); display range editorial"),
  molecularBand("co2_4p3", "CO2 4.3", "CO2", 2349, 4.15, 4.5, "NIST CCCBDB 124389: nu3 2349 cm-1 (4.257 um); display range editorial"),
  molecularBand("co_4p7", "CO 4.7", "CO", 2143, 4.3, 5.1, "NIST CCCBDB 630080: 1-0 2143 cm-1 (4.666 um); display range from Grant et al., arXiv 2304.11994"),
  molecularBand("ch4_3p3", "CH4 3.3", "CH4", 3019, 3.15, 3.5, "NIST WebBook C74828: nu3 3019 cm-1 (3.312 um); display range editorial"),
  molecularBand("ch4_7p7", "CH4 7.7", "CH4", 1306, 7.3, 8.1, "NIST WebBook C74828: nu4 1306 cm-1 (7.657 um); display range editorial"),
  molecularBand("so2_7p3", "SO2 7.3", "SO2", 1362, 7.1, 7.6, "NIST CCCBDB 7446095: nu3 1362 cm-1 (7.342 um); display range editorial"),
  molecularBand("so2_8p7", "SO2 8.7", "SO2", 1151, 8.4, 9.0, "NIST CCCBDB 7446095: nu1 1151 cm-1 (8.688 um); display range editorial"),
  molecularBand("nh3_3p0", "NH3 3.0", "NH3", 3337, 2.85, 3.05, "NIST WebBook C7664417: nu1 3337 cm-1 (2.997 um), nu3 3444 cm-1 (2.904 um); display range editorial"),
  molecularBand("nh3_6p1", "NH3 6.1", "NH3", 1627, 5.9, 6.4, "NIST WebBook C7664417: nu4 1627 cm-1 (6.146 um); display range editorial"),
  molecularBand("nh3_10p5", "NH3 10.5", "NH3", 932.5, 10.0, 11.0, "NIST WebBook C7664417: nu2 inversion doublet 932.5 and 968.3 cm-1 (10.724 and 10.327 um); display range editorial"),
  molecularBand("h2s_3p8", "H2S 3.8", "H2S", 2615, 3.7, 4.0, "NIST CCCBDB 7783064: nu1 2615 cm-1 (3.824 um), nu3 2626 cm-1 (3.808 um); display range editorial"),
  molecularBand("h2s_8p5", "H2S 8.5", "H2S", 1183, 8.0, 9.0, "NIST CCCBDB 7783064: nu2 1183 cm-1 (8.453 um); display range editorial"),
  molecularBand("hcn_3p0", "HCN 3.0", "HCN", 3312, 2.95, 3.1, "NIST CCCBDB 74908: C-H stretch 3312 cm-1 (3.019 um); display range editorial"),
  molecularBand("hcn_4p8", "HCN 4.8", "HCN", 2089, 4.7, 4.9, "NIST CCCBDB 74908: C-N stretch 2089 cm-1 (4.787 um); display range editorial"),
  molecularBand("c2h2_3p0", "C2H2 3.0", "C2H2", 3289, 2.95, 3.1, "NIST CCCBDB 74862: nu3 3289 cm-1 (3.040 um); display range editorial"),
  atomicBand("na_d", "Na I D", "Na", 0.5891583264, 0.5891583264, 0.5897558147, "NIST ASD vacuum: Na I D2 589.1583264 nm, D1 589.7558147 nm"),
  atomicBand("k_i_doublet", "K I 0.77", "K", 0.7667008906, 0.7667008906, 0.7701083536, "NIST ASD vacuum: K I 766.7008906 and 770.1083536 nm"),
  atomicBand("he_i_triplet", "He I 1.083", "He", 1.0833306454, 1.0832057482, 1.0833306454, "NIST ASD vacuum: He I 1083.2057482, 1083.3216761 and 1083.3306454 nm"),
  atomicBand("h_alpha_band", "Hα", "H", 0.65646, 0.656452, 0.656463, "NIST ASD vacuum: H I H-alpha components 656.452 to 656.463 nm, strongest observed line 656.460 nm"),
  {
    id: "nirspec_g395h_gap",
    label: "G395H gap",
    species: "G395H gap",
    kind: "instrument",
    centreVacuumUm: 3.77,
    loVacuumUm: 3.72,
    hiVacuumUm: 3.82,
    source: "JDox NIRSpec BOTS wavelength ranges and gaps: G395H/F290LP SUB2048, 3.72 to 3.82 um between NRS1 and NRS2; observed frame, not redshifted",
  },
];

export const BAND_SPECIES: readonly string[] = [...new Set(REST_BANDS.map((band) => band.species))];

export const DEFAULT_BAND_SPECIES: readonly string[] = ["H2O", "CO2", "CO", "CH4", "SO2"];

export function bandById(id: string): RestBand | null {
  return REST_BANDS.find((band) => band.id === id) ?? null;
}

export function toggleBandSpecies(species: readonly string[], toggled: string): string[] {
  return BAND_SPECIES.filter((name) => (name === toggled ? !species.includes(name) : species.includes(name)));
}

export function familyLabel(family: LineFamily): string {
  switch (family) {
    case "optical":
      return "Optical";
    case "nir":
      return "NIR";
    case "mir":
      return "MIR";
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

export interface BandMark {
  id: string;
  label: string;
  species: string;
  kind: RestBandKind;
  lo: number;
  hi: number;
}

function bandEdgeDisplayValue(band: RestBand, edgeVacuumUm: number, input: LineMarksInput): number | null {
  const observed =
    band.kind === "instrument"
      ? edgeVacuumUm * (1 + (input.shiftKms ?? 0) / SPEED_OF_LIGHT_KMS)
      : observedVacuumUm(edgeVacuumUm, input.redshift);
  return lineDisplayValue(observed, input.axis, input.mode, input.restUm, input.convention, input.shiftKms);
}

export function bandMarks(input: LineMarksInput & { species: readonly string[] }): BandMark[] {
  const marks: BandMark[] = [];
  for (const band of REST_BANDS) {
    if (!input.species.includes(band.species)) continue;
    const a = bandEdgeDisplayValue(band, band.loVacuumUm, input);
    const b = bandEdgeDisplayValue(band, band.hiVacuumUm, input);
    if (a === null || b === null || !Number.isFinite(a) || !Number.isFinite(b)) continue;
    const lo = Math.min(a, b);
    const hi = Math.max(a, b);
    if (hi < input.xMin || lo > input.xMax) continue;
    marks.push({
      id: band.id,
      label: band.label,
      species: band.species,
      kind: band.kind,
      lo: Math.max(lo, input.xMin),
      hi: Math.min(hi, input.xMax),
    });
  }
  return marks;
}

export function nearestBandMark(
  marks: readonly BandMark[],
  m: PlotMapping,
  px: number,
  tolerancePx: number = LINE_PICK_TOLERANCE_PX,
): BandMark | null {
  let best: BandMark | null = null;
  let bestSpan = Infinity;
  for (const mark of marks) {
    const p0 = axisValueToPixel(mark.lo, m);
    const p1 = axisValueToPixel(mark.hi, m);
    const left = Math.min(p0, p1);
    const right = Math.max(p0, p1);
    if (px < left - tolerancePx || px > right + tolerancePx) continue;
    const span = right - left;
    if (span < bestSpan) {
      best = mark;
      bestSpan = span;
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

export function pickedBandLabel(band: RestBand): string {
  const lo = Number(band.loVacuumUm.toFixed(BAND_EDGE_DECIMALS));
  const hi = Number(band.hiVacuumUm.toFixed(BAND_EDGE_DECIMALS));
  const range = `range ← ${band.label} ${lo} to ${hi} ${MICRON} vacuum`;
  return band.kind === "molecular_band" ? `${range} (display range)` : range;
}
