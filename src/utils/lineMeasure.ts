import type { GaussianFitResult, LineMeasurement, SpectralAxisInfo, SpectrumSource } from "../shared/types/spectral";
import { airToVacuumUm } from "./spectralAxis";
import { axisValueToPixel, channelAxisValue, type PlotMapping } from "./spectrumRange";

export interface ContinuumParams {
  level: number;
  slope: number;
  reference: number;
}

export interface GaussianParams {
  amplitude: number;
  centre: number;
  sigma: number;
}

export interface PlotFrame {
  top: number;
  height: number;
  yMin: number;
  yMax: number;
}

export interface PlotPoint {
  x: number;
  y: number;
}

export interface LineOverlay {
  continuum: PlotPoint[];
  model: PlotPoint[];
}

export interface MeasurementRow {
  label: string;
  value: string;
}

export const FWHM_PER_SIGMA = 2.354820045030949;
export const MODEL_SUBSAMPLES = 4;
export const UNAVAILABLE = "n/a";
export const PLUS_MINUS = "±";

const MIN_Y_RANGE = 1e-10;
const GENERAL_DIGITS = 4;
const EXPONENT_BELOW = 1e-3;
const EXPONENT_ABOVE = 1e6;
const AXIS_DECIMALS = 5;
const VELOCITY_DECIMALS = 1;

export const CSV_COLUMNS = [
  "z0",
  "z1",
  "n_channels",
  "axis_unit",
  "flux_unit",
  "continuum_level",
  "continuum_slope",
  "continuum_sigma",
  "continuum_reference",
  "continuum_linear",
  "continuum_channels",
  "flux",
  "flux_err",
  "equivalent_width",
  "centroid",
  "sigma",
  "fwhm",
  "peak",
  "peak_channel",
  "snr",
  "velocity_centroid_kms",
  "velocity_sigma_kms",
  "velocity_fwhm_kms",
  "velocity_rest_um",
  "velocity_convention",
  "velocity_shift_applied_kms",
  "fit_amplitude",
  "fit_centre",
  "fit_sigma",
  "fit_amplitude_err",
  "fit_centre_err",
  "fit_sigma_err",
  "fit_chi2",
  "fit_dof",
  "fit_iterations",
  "fit_converged",
] as const;

function finite(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function formatQuantity(value: number | null | undefined, digits: number = GENERAL_DIGITS): string {
  if (!finite(value)) return UNAVAILABLE;
  const magnitude = Math.abs(value);
  if (magnitude !== 0 && (magnitude < EXPONENT_BELOW || magnitude >= EXPONENT_ABOVE)) return value.toExponential(digits - 1);
  return value.toPrecision(digits);
}

export function formatAxisQuantity(value: number | null | undefined): string {
  return finite(value) ? value.toFixed(AXIS_DECIMALS) : UNAVAILABLE;
}

export function formatVelocity(value: number | null | undefined): string {
  return finite(value) ? `${value.toFixed(VELOCITY_DECIMALS)} km/s` : UNAVAILABLE;
}

export function continuumParams(m: LineMeasurement): ContinuumParams | null {
  if (!finite(m.continuum_level) || !finite(m.continuum_reference)) return null;
  return { level: m.continuum_level, slope: finite(m.continuum_slope) ? m.continuum_slope : 0, reference: m.continuum_reference };
}

export function continuumAt(continuum: ContinuumParams, x: number): number {
  return continuum.level + continuum.slope * (x - continuum.reference);
}

export function gaussianModel(fit: GaussianParams, continuum: ContinuumParams, xs: number[]): number[] {
  const twoSigma2 = 2 * fit.sigma * fit.sigma;
  return xs.map((x) => {
    const d = x - fit.centre;
    const line = twoSigma2 > 0 ? fit.amplitude * Math.exp((-d * d) / twoSigma2) : 0;
    return continuumAt(continuum, x) + line;
  });
}

export function continuumModel(m: LineMeasurement, xs: number[]): number[] {
  const continuum = continuumParams(m);
  if (!continuum) return xs.map(() => NaN);
  return xs.map((x) => continuumAt(continuum, x));
}

export function convergedFit(fit: GaussianFitResult | null): GaussianParams | null {
  if (!fit || !fit.converged || !finite(fit.amplitude) || !finite(fit.centre) || !finite(fit.sigma)) return null;
  return { amplitude: fit.amplitude, centre: fit.centre, sigma: fit.sigma };
}

export function measurementAxisValues(axis: SpectralAxisInfo | null): number[] | null {
  if (!axis || axis.kind === "unknown") return null;
  return axis.kind === "awav" ? axis.values.map(airToVacuumUm) : axis.values;
}

export function spectrumSourceKey(source: SpectrumSource, view: string): string {
  return `${source.kind}:${view}:${JSON.stringify(source)}`;
}

export function measurementSpan(m: LineMeasurement): { first: number; last: number } {
  const [[a0, a1], [b0, b1]] = m.continuum_windows;
  return { first: Math.min(a0, b0, m.z0), last: Math.max(a1, b1, m.z1) };
}

function samplePolyline(
  first: number,
  last: number,
  measurementAxis: number[],
  mapping: PlotMapping,
  frame: PlotFrame,
  evaluate: (x: number) => number,
): PlotPoint[] {
  const n = mapping.n;
  if (n <= 0 || measurementAxis.length !== n) return [];
  const lo = Math.max(0, Math.min(first, last));
  const hi = Math.min(n - 1, Math.max(first, last));
  if (lo > hi) return [];
  const left = mapping.padLeft;
  const right = mapping.width - mapping.padRight;
  const yRange = Math.max(frame.yMax - frame.yMin, MIN_Y_RANGE);
  const toY = (y: number) => frame.top + frame.height - ((y - frame.yMin) / yRange) * frame.height;
  const points: PlotPoint[] = [];
  const push = (xMeasured: number, xDisplay: number) => {
    const y = evaluate(xMeasured);
    if (!Number.isFinite(xMeasured) || !Number.isFinite(xDisplay) || !Number.isFinite(y)) return;
    const px = axisValueToPixel(xDisplay, mapping);
    if (px < left || px > right) return;
    const py = Math.min(frame.top + frame.height, Math.max(frame.top, toY(y)));
    points.push({ x: px, y: py });
  };
  for (let i = lo; i <= hi; i++) {
    push(measurementAxis[i], channelAxisValue(i, mapping));
    if (i === hi) break;
    const xm0 = measurementAxis[i];
    const xm1 = measurementAxis[i + 1];
    const xd0 = channelAxisValue(i, mapping);
    const xd1 = channelAxisValue(i + 1, mapping);
    for (let k = 1; k < MODEL_SUBSAMPLES; k++) {
      const t = k / MODEL_SUBSAMPLES;
      push(xm0 + (xm1 - xm0) * t, xd0 + (xd1 - xd0) * t);
    }
  }
  return points;
}

export function lineOverlayPolylines(
  result: LineMeasurement,
  measurementAxis: number[],
  mapping: PlotMapping,
  frame: PlotFrame,
): LineOverlay {
  const continuum = continuumParams(result);
  if (!continuum) return { continuum: [], model: [] };
  const span = measurementSpan(result);
  const continuumLine = samplePolyline(span.first, span.last, measurementAxis, mapping, frame, (x) => continuumAt(continuum, x));
  const fit = convergedFit(result.fit);
  const model = fit
    ? samplePolyline(result.z0, result.z1, measurementAxis, mapping, frame, (x) => gaussianModel(fit, continuum, [x])[0])
    : [];
  return { continuum: continuumLine, model };
}

function withVelocity(axisText: string, kms: number | null | undefined, hasVelocity: boolean): string {
  return hasVelocity ? `${axisText} / ${formatVelocity(kms)}` : axisText;
}

export function lineMeasurementRows(m: LineMeasurement): MeasurementRow[] {
  const unit = m.axis_unit;
  const v = m.velocity;
  const rows: MeasurementRow[] = [
    { label: "Flux", value: `${formatQuantity(m.flux)} ${PLUS_MINUS} ${formatQuantity(m.flux_err)} ${m.flux_unit}` },
    { label: "Equivalent width", value: `${formatQuantity(m.equivalent_width)} ${unit}` },
    { label: "Centroid", value: withVelocity(`${formatAxisQuantity(m.centroid)} ${unit}`, v?.centroid_kms, v !== null) },
    { label: "Sigma", value: withVelocity(`${formatAxisQuantity(m.sigma)} ${unit}`, v?.sigma_kms, v !== null) },
    { label: "FWHM", value: withVelocity(`${formatAxisQuantity(m.fwhm)} ${unit}`, v?.fwhm_kms, v !== null) },
    { label: "Peak", value: `${formatQuantity(m.peak)} at ch ${m.peak_channel}` },
    { label: "SNR", value: finite(m.snr) ? m.snr.toFixed(1) : UNAVAILABLE },
    {
      label: "Continuum",
      value: `level ${formatQuantity(m.continuum_level)}, slope ${formatQuantity(m.continuum_slope)}, sigma ${formatQuantity(m.continuum_sigma)}, ${m.continuum_channels} ch ${m.continuum_linear ? "linear" : "constant"}`,
    },
  ];
  if (v) {
    rows.push({
      label: "Velocity frame",
      value: `${v.convention}, rest ${formatAxisQuantity(v.rest_um)} um${v.shift_applied_kms !== 0 ? `, shift ${formatVelocity(v.shift_applied_kms)}` : ", topocentric"}`,
    });
  }
  if (m.fit) {
    const f = m.fit;
    rows.push({
      label: "Gaussian fit",
      value: `A ${formatQuantity(f.amplitude)} ${PLUS_MINUS} ${formatQuantity(f.amplitude_err)}, c ${formatAxisQuantity(f.centre)} ${PLUS_MINUS} ${formatAxisQuantity(f.centre_err)}, s ${formatAxisQuantity(f.sigma)} ${PLUS_MINUS} ${formatAxisQuantity(f.sigma_err)} ${unit}, chi2/dof ${formatQuantity(f.chi2)}/${f.dof}, ${f.iterations} it`,
    });
  }
  return rows;
}

function csvCell(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "number") return Number.isFinite(value) ? String(value) : "";
  if (typeof value === "boolean") return value ? "true" : "false";
  const text = String(value);
  return /[",\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

export function lineMeasurementCsv(m: LineMeasurement): string {
  const v = m.velocity;
  const f = m.fit;
  const values: unknown[] = [
    m.z0,
    m.z1,
    m.n_channels,
    m.axis_unit,
    m.flux_unit,
    m.continuum_level,
    m.continuum_slope,
    m.continuum_sigma,
    m.continuum_reference,
    m.continuum_linear,
    m.continuum_channels,
    m.flux,
    m.flux_err,
    m.equivalent_width,
    m.centroid,
    m.sigma,
    m.fwhm,
    m.peak,
    m.peak_channel,
    m.snr,
    v?.centroid_kms ?? null,
    v?.sigma_kms ?? null,
    v?.fwhm_kms ?? null,
    v?.rest_um ?? null,
    v?.convention ?? null,
    v?.shift_applied_kms ?? null,
    f?.amplitude ?? null,
    f?.centre ?? null,
    f?.sigma ?? null,
    f?.amplitude_err ?? null,
    f?.centre_err ?? null,
    f?.sigma_err ?? null,
    f?.chi2 ?? null,
    f?.dof ?? null,
    f?.iterations ?? null,
    f?.converged ?? null,
  ];
  return `${CSV_COLUMNS.join(",")}\n${values.map(csvCell).join(",")}`;
}
