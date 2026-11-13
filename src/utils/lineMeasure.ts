import type {
  CorrectionFrame,
  GaussianFitResult,
  LineMeasurement,
  MeasuredLineSource,
  SeriesSpectrumSource,
  SpectralAxisInfo,
} from "../shared/types/spectral";
import { SIGMA_MAD_LABEL } from "./analysisLabels";
import { provenanceLines, unitLines, withProvenance } from "./csvProvenance";
import { formatWithUncertainty } from "./formatUncertainty";
import { formatRedshift } from "./lineList";
import { spectrumUnitOf } from "./measurementLog";
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

export type AxisDisplayUnit = "um" | "nm" | "A";

export interface LineRowOptions {
  axisDisplayUnit?: AxisDisplayUnit;
}

export interface LineCsvContext {
  fileName?: string | null;
  dq?: string | null;
  provenance?: { software: string; exportedUtc: string } | null;
}

export const FWHM_PER_SIGMA = 2.354820045030949;
export const MODEL_SUBSAMPLES = 4;
export const UNAVAILABLE = "n/a";
export const PLUS_MINUS = "±";
export const POOR_FIT_CHI2_RED = 3;

const MICRONS_TO_DISPLAY: Record<AxisDisplayUnit, number> = { um: 1, nm: 1e3, A: 1e4 };
const RELIABLE_ERRORS_MIN_SNR = 10;
const WIDTH_ERROR_LIMIT_PERCENT = 15;
const SI_FLUX_UNIT = "W m^-2";
const CGS_FLUX_UNIT = "erg s^-1 cm^-2";

const GENERAL_DIGITS = 4;
const EXPONENT_BELOW = 1e-3;
const EXPONENT_ABOVE = 1e6;
const AXIS_DECIMALS = 5;
const MAX_AXIS_DECIMALS = 10;
const VELOCITY_DECIMALS = 1;
const UNDECLARED_AXIS_FRAME = "axis frame not declared (no SPECSYS)";
const NO_ERR_SOURCE = "none: flux error from the continuum scatter";
const NIRISS_SOSS_DETECTOR = "NIS";

export const CSV_COLUMNS = [
  "source_kind",
  "source_x",
  "source_y",
  "source_region",
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
  "velocity_axis_frame",
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
  "source_hdu",
  "err_used",
  "err_source",
  "flux_err_pixel",
  "flux_err_level",
  "flux_err_slope",
  "n_dq_excluded",
  "n_nonfinite",
  "n_bad_err",
  "velocity_redshift",
  "velocity_source_centroid_kms",
  "velocity_source_sigma_kms",
  "velocity_source_fwhm_kms",
  "continuum_chi2_red",
  "n_channels_nominal",
  "equivalent_width_err",
  "centroid_err",
  "sigma_err",
  "fwhm_err",
  "errors_reliable",
  "width_errors_reliable",
  "velocity_centroid_err_kms",
  "velocity_sigma_err_kms",
  "velocity_fwhm_err_kms",
  "velocity_source_centroid_err_kms",
  "velocity_source_sigma_err_kms",
  "velocity_source_fwhm_err_kms",
  "velocity_z_line",
  "velocity_z_line_err",
  "peak_axis",
  "wavelength_frame",
  "line_id",
  "line_label",
  "redshift",
  "flux_si",
  "flux_si_err",
  "flux_cgs",
  "flux_cgs_err",
  "flux_physical_basis",
  "file",
  "dq",
  "source_series",
] as const;

type LineCsvColumn = (typeof CSV_COLUMNS)[number];

const FLUX_UNIT_COLUMNS: readonly LineCsvColumn[] = ["flux", "flux_err", "flux_err_pixel", "flux_err_level", "flux_err_slope"];
const SPECTRUM_UNIT_COLUMNS: readonly LineCsvColumn[] = ["continuum_level", "continuum_sigma", "peak", "fit_amplitude", "fit_amplitude_err"];
const AXIS_UNIT_COLUMNS: readonly LineCsvColumn[] = [
  "equivalent_width",
  "equivalent_width_err",
  "centroid",
  "centroid_err",
  "sigma",
  "sigma_err",
  "fwhm",
  "fwhm_err",
  "continuum_reference",
  "peak_axis",
  "fit_centre",
  "fit_sigma",
  "fit_centre_err",
  "fit_sigma_err",
];

function finite(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function formatQuantity(value: number | null | undefined, digits: number = GENERAL_DIGITS): string {
  if (!finite(value)) return UNAVAILABLE;
  const magnitude = Math.abs(value);
  if (magnitude !== 0 && (magnitude < EXPONENT_BELOW || magnitude >= EXPONENT_ABOVE)) return value.toExponential(digits - 1);
  return value.toPrecision(digits);
}

export function formatAxisQuantity(value: number | null | undefined, resolution?: number | null): string {
  if (!finite(value)) return UNAVAILABLE;
  const decimals =
    finite(resolution) && resolution > 0
      ? Math.min(MAX_AXIS_DECIMALS, Math.max(AXIS_DECIMALS, 1 - Math.floor(Math.log10(resolution))))
      : AXIS_DECIMALS;
  return value.toFixed(decimals);
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

export function lineMeasurementAxis(axis: SpectralAxisInfo | null, documentMode: boolean): number[] | null {
  if (!documentMode) return measurementAxisValues(axis);
  return axis ? axis.values : null;
}

export function spectrumSourceKey(source: MeasuredLineSource, view: string): string {
  return `${source.kind}:${view}:${JSON.stringify(source)}`;
}

function seriesSourceLabel({ detector, sporder, pick }: SeriesSpectrumSource): string {
  const ordersShareDetector = sporder !== null && (sporder >= 2 || (sporder === 1 && detector === NIRISS_SOSS_DETECTOR));
  const series = ordersShareDetector ? `${detector}, order ${sporder}` : detector;
  if (pick.kind === "median") return `time series ${series}, median`;
  if (pick.kind === "mean") return `time series ${series}, mean ${pick.first + 1} to ${pick.last + 1}`;
  return `time series ${series}, integration ${pick.index + 1}`;
}

export function spectrumSourceLabel(source: MeasuredLineSource): string {
  if (source.kind === "pixel") return `pixel (${source.x}, ${source.y})`;
  if (source.kind === "file") return source.hdu === null ? "spectrum file" : `spectrum file, HDU ${source.hdu}`;
  if (source.kind === "series") return seriesSourceLabel(source);
  return `${source.shape.shape} region${source.background ? " with background" : ""}`;
}

export function runForSource<T extends { key: string }>(run: T | null, source: MeasuredLineSource | null, view: string): T | null {
  return run && source && spectrumSourceKey(source, view) === run.key ? run : null;
}

export function momentVelocityFrameNote(specsys: string | null, correction: CorrectionFrame, lineShiftKms: number | null): string {
  const frame = specsys ? `header frame ${specsys}` : "header frame (no SPECSYS)";
  if (correction === "none" || !finite(lineShiftKms) || lineShiftKms === 0) return `M1 in the ${frame}`;
  return `M1 in the ${frame}, without the ${correction} shift of ${formatVelocity(lineShiftKms)} that the line velocity includes`;
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
  const yRange = frame.yMax > frame.yMin ? frame.yMax - frame.yMin : 1;
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

function velocityWithError(kms: number | null | undefined, errKms: number | null | undefined): string {
  return finite(kms) && finite(errKms) && errKms > 0
    ? `${kms.toFixed(VELOCITY_DECIMALS)} ${PLUS_MINUS} ${errKms.toFixed(VELOCITY_DECIMALS)} km/s`
    : formatVelocity(kms);
}

function withVelocity(axisText: string, kms: number | null | undefined, errKms: number | null | undefined, hasVelocity: boolean): string {
  return hasVelocity ? `${axisText} / ${velocityWithError(kms, errKms)}` : axisText;
}

function withError(value: number | null | undefined, err: number | null | undefined, unit: string, plain: string): string {
  if (!finite(value)) return `${UNAVAILABLE} ${unit}`;
  return finite(err) && err > 0 ? formatWithUncertainty(value, err, { unit }) : plain;
}

function scaled(value: number | null | undefined, factor: number): number | null | undefined {
  return finite(value) ? value * factor : value;
}

function signalToNoiseText(snr: number | null): string {
  if (!finite(snr)) return UNAVAILABLE;
  const magnitude = Math.abs(snr).toFixed(1);
  if (snr > 0) return `${magnitude} (emission)`;
  return snr < 0 ? `${magnitude} (absorption)` : magnitude;
}

function fitChi2Text(chi2: number | null, dof: number): string {
  const ratio = finite(chi2) && dof > 0 ? chi2 / dof : null;
  const reduced = ratio === null ? "" : ` = ${formatQuantity(ratio)}${ratio > POOR_FIT_CHI2_RED ? " (poor fit)" : ""}`;
  return `chi2/dof ${formatQuantity(chi2)}/${dof}${reduced}`;
}

function windowChannelCount([[a0, a1], [b0, b1]]: LineMeasurement["continuum_windows"]): number {
  const span = (lo: number, hi: number) => Math.max(0, hi - lo + 1);
  return span(a0, a1) + span(b0, b1) - span(Math.max(a0, b0), Math.min(a1, b1));
}

function errorReliabilityText(m: LineMeasurement): string | null {
  if (m.errors_reliable === false && [m.equivalent_width_err, m.centroid_err, m.sigma_err].some(finite)) {
    const snr = finite(m.snr) ? Math.abs(m.snr).toFixed(1) : UNAVAILABLE;
    return `line S/N ${snr} is below ${RELIABLE_ERRORS_MIN_SNR}: the errors of equivalent width, centroid and widths are indicative`;
  }
  if (m.errors_reliable === true && m.width_errors_reliable === false && finite(m.sigma) && finite(m.sigma_err)) {
    const percent = ((100 * m.sigma_err) / Math.abs(m.sigma)).toFixed(0);
    return `sigma is known to ${percent} percent: the errors of sigma and FWHM are indicative above ${WIDTH_ERROR_LIMIT_PERCENT} percent`;
  }
  return null;
}

export function lineMeasurementRows(m: LineMeasurement, options: LineRowOptions = {}): MeasurementRow[] {
  const displayUnit = m.axis_unit === "um" ? (options.axisDisplayUnit ?? "um") : null;
  const unit = displayUnit ?? m.axis_unit;
  const factor = displayUnit ? MICRONS_TO_DISPLAY[displayUnit] : 1;
  const positionUnit = m.wavelength_frame ? `${unit} ${m.wavelength_frame}` : unit;
  const v = m.velocity;
  const hasVelocity = v !== null;
  const equivalentWidth = scaled(m.equivalent_width, factor);
  const centroid = scaled(m.centroid, factor);
  const sigma = scaled(m.sigma, factor);
  const fwhm = scaled(m.fwhm, factor);
  const spectrumUnit = spectrumUnitOf(m.flux_unit, m.axis_unit);
  const peakValue = spectrumUnit ? `${formatQuantity(m.peak)} ${spectrumUnit}` : formatQuantity(m.peak);
  const peakAxis = scaled(m.peak_axis, factor);
  const rows: MeasurementRow[] = [
    {
      label: "Flux",
      value: withError(m.flux, m.flux_err, m.flux_unit, `${formatQuantity(m.flux)} ${PLUS_MINUS} ${formatQuantity(m.flux_err)} ${m.flux_unit}`),
    },
  ];
  const basis = m.flux_physical_basis;
  if (basis) {
    rows.push({
      label: "Physical flux",
      value: finite(m.flux_si)
        ? `${formatWithUncertainty(m.flux_si, m.flux_si_err, { unit: SI_FLUX_UNIT })} = ${formatWithUncertainty(m.flux_cgs, m.flux_cgs_err, { unit: CGS_FLUX_UNIT })} (${basis})`
        : basis,
    });
  }
  rows.push(
    {
      label: "Equivalent width (absorption > 0, emission < 0)",
      value: withError(equivalentWidth, scaled(m.equivalent_width_err, factor), unit, `${formatQuantity(equivalentWidth)} ${unit}`),
    },
    {
      label: "Centroid",
      value: withVelocity(
        withError(centroid, scaled(m.centroid_err, factor), positionUnit, `${formatAxisQuantity(centroid)} ${positionUnit}`),
        v?.source_centroid_kms,
        v?.source_centroid_err_kms,
        hasVelocity,
      ),
    },
    {
      label: "Sigma (2nd moment)",
      value: withVelocity(
        withError(sigma, scaled(m.sigma_err, factor), unit, `${formatQuantity(sigma)} ${unit}`),
        v?.source_sigma_kms,
        v?.source_sigma_err_kms,
        hasVelocity,
      ),
    },
    {
      label: "FWHM (2.355 x sigma)",
      value: withVelocity(
        withError(fwhm, scaled(m.fwhm_err, factor), unit, `${formatQuantity(fwhm)} ${unit}`),
        v?.source_fwhm_kms,
        v?.source_fwhm_err_kms,
        hasVelocity,
      ),
    },
    {
      label: finite(m.flux) && m.flux < 0 ? "Trough" : "Peak",
      value: finite(peakAxis)
        ? `${peakValue} at ${formatAxisQuantity(peakAxis)} ${positionUnit} (ch ${m.peak_channel})`
        : `${peakValue} at ch ${m.peak_channel}`,
    },
    { label: "SNR", value: signalToNoiseText(m.snr) },
    {
      label: "Continuum",
      value: `level ${formatQuantity(m.continuum_level)}, slope ${formatQuantity(m.continuum_slope)}, ${SIGMA_MAD_LABEL} ${formatQuantity(m.continuum_sigma)}, ${m.continuum_channels} ch ${m.continuum_linear ? "linear" : "constant"}`,
    },
  );
  if (v) {
    const redshift = v.redshift ?? 0;
    const frame = v.axis_frame ? `axis frame ${v.axis_frame}` : UNDECLARED_AXIS_FRAME;
    const shift = v.shift_applied_kms !== 0 ? `, shift ${formatVelocity(v.shift_applied_kms)}` : "";
    rows.push({
      label: "Velocity frame",
      value: `${v.convention}, rest ${formatAxisQuantity(v.rest_um)} um, z = ${formatRedshift(redshift)}, ${frame}${shift}`,
    });
    if (redshift !== 0) {
      rows.push({
        label: "Observer frame (against the laboratory rest wavelength)",
        value: `centroid ${formatVelocity(v.centroid_kms)}, sigma ${formatVelocity(v.sigma_kms)}, FWHM ${formatVelocity(v.fwhm_kms)}`,
      });
    }
    if (finite(v.z_line)) {
      rows.push({
        label: "Line redshift",
        value: `z = ${formatWithUncertainty(v.z_line, v.z_line_err)} (centroid over rest wavelength, minus 1)`,
      });
    }
  }
  if (m.fit) {
    const f = m.fit;
    const centreErr = scaled(f.centre_err, factor);
    const fitWidths = `s ${formatQuantity(scaled(f.sigma, factor))} ${PLUS_MINUS} ${formatQuantity(scaled(f.sigma_err, factor))} ${unit}, FWHM ${formatQuantity(scaled(f.sigma, factor * FWHM_PER_SIGMA))} ${PLUS_MINUS} ${formatQuantity(scaled(f.sigma_err, factor * FWHM_PER_SIGMA))} ${unit}`;
    rows.push({
      label: "Gaussian fit",
      value: `A ${formatQuantity(f.amplitude)} ${PLUS_MINUS} ${formatQuantity(f.amplitude_err)}, c ${formatAxisQuantity(scaled(f.centre, factor), centreErr)} ${PLUS_MINUS} ${formatQuantity(centreErr)}, ${fitWidths}, ${fitChi2Text(f.chi2, f.dof)}, ${f.iterations} it`,
    });
  }
  const chi2Red = m.continuum_chi2_red;
  const scatterScale =
    finite(chi2Red) && chi2Red > 1
      ? `, flux error x ${formatQuantity(Math.sqrt(chi2Red))} from the continuum scatter (chi2_red ${formatQuantity(chi2Red)})`
      : "";
  rows.push({ label: "Error source", value: m.err_used ? `${m.err_source}, weights 1/ERR^2${scatterScale}` : NO_ERR_SOURCE });
  const terms = m.flux_err_terms;
  if (terms) {
    rows.push({
      label: "Error terms",
      value: `pixel ${formatQuantity(terms.pixel)}, level ${formatQuantity(terms.level)}, slope ${formatQuantity(terms.slope)} ${m.flux_unit}`,
    });
  }
  const reliability = errorReliabilityText(m);
  if (reliability) rows.push({ label: "Error reliability", value: reliability });
  const exclusions = `${m.n_dq_excluded} DQ-flagged, ${m.n_nonfinite} non-finite flux, ${m.n_bad_err} ERR <= 0 or non-finite`;
  const nominal = m.n_channels_nominal;
  const continuumNominal = windowChannelCount(m.continuum_windows);
  rows.push({
    label: "Excluded channels",
    value: finite(nominal)
      ? `in line ${nominal - m.n_channels} of ${nominal}, in continuum ${continuumNominal - m.continuum_channels} of ${continuumNominal}: ${exclusions}`
      : exclusions,
  });
  return rows;
}

function csvCell(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "number") return Number.isFinite(value) ? String(value) : "";
  if (typeof value === "boolean") return value ? "true" : "false";
  const text = String(value);
  return /[",\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

function lineCsvUnit(column: LineCsvColumn, m: LineMeasurement, spectrumUnit: string | null): string | null {
  if (FLUX_UNIT_COLUMNS.includes(column)) return m.flux_unit;
  if (SPECTRUM_UNIT_COLUMNS.includes(column)) return spectrumUnit;
  if (column === "continuum_slope") return spectrumUnit ? `${spectrumUnit} per ${m.axis_unit}` : null;
  if (AXIS_UNIT_COLUMNS.includes(column)) return m.axis_unit;
  if (column.endsWith("_kms")) return "km/s";
  if (column === "velocity_rest_um") return "um";
  if (column === "flux_si" || column === "flux_si_err") return SI_FLUX_UNIT;
  if (column === "flux_cgs" || column === "flux_cgs_err") return CGS_FLUX_UNIT;
  return null;
}

function lineCsvUnits(m: LineMeasurement): Record<string, string | null> {
  const spectrumUnit = spectrumUnitOf(m.flux_unit, m.axis_unit);
  return Object.fromEntries(CSV_COLUMNS.map((column) => [column, lineCsvUnit(column, m, spectrumUnit)]));
}

export function lineMeasurementCsv(m: LineMeasurement, context?: LineCsvContext): string {
  const v = m.velocity;
  const f = m.fit;
  const s = m.source;
  const values: unknown[] = [
    s.kind,
    s.kind === "pixel" ? s.x : null,
    s.kind === "pixel" ? s.y : null,
    s.kind === "region" ? JSON.stringify({ shape: s.shape, background: s.background }) : null,
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
    v?.axis_frame ?? null,
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
    s.kind === "file" ? s.hdu : null,
    m.err_used,
    m.err_source,
    m.flux_err_terms?.pixel ?? null,
    m.flux_err_terms?.level ?? null,
    m.flux_err_terms?.slope ?? null,
    m.n_dq_excluded,
    m.n_nonfinite,
    m.n_bad_err,
    v?.redshift ?? null,
    v?.source_centroid_kms ?? null,
    v?.source_sigma_kms ?? null,
    v?.source_fwhm_kms ?? null,
    m.continuum_chi2_red ?? null,
    m.n_channels_nominal ?? null,
    m.equivalent_width_err ?? null,
    m.centroid_err ?? null,
    m.sigma_err ?? null,
    m.fwhm_err ?? null,
    m.errors_reliable ?? null,
    m.width_errors_reliable ?? null,
    v?.centroid_err_kms ?? null,
    v?.sigma_err_kms ?? null,
    v?.fwhm_err_kms ?? null,
    v?.source_centroid_err_kms ?? null,
    v?.source_sigma_err_kms ?? null,
    v?.source_fwhm_err_kms ?? null,
    v?.z_line ?? null,
    v?.z_line_err ?? null,
    m.peak_axis ?? null,
    m.wavelength_frame ?? null,
    m.line_id ?? null,
    m.line_label ?? null,
    m.redshift ?? null,
    m.flux_si ?? null,
    m.flux_si_err ?? null,
    m.flux_cgs ?? null,
    m.flux_cgs_err ?? null,
    m.flux_physical_basis ?? null,
    context?.fileName ?? null,
    context?.dq ?? null,
    s.kind === "series" ? JSON.stringify({ paths: s.paths, detector: s.detector, sporder: s.sporder, pick: s.pick }) : null,
  ];
  const body = `${CSV_COLUMNS.join(",")}\n${values.map(csvCell).join(",")}`;
  const provenance = context?.provenance;
  if (!provenance) return body;
  const lines = [
    ...provenanceLines([
      ["software", provenance.software],
      ["exported_utc", provenance.exportedUtc],
      ["file", context?.fileName ?? null],
      ["line", m.line_label ?? m.line_id ?? null],
      ["source", spectrumSourceLabel(m.source)],
      ["err_source", m.err_source],
      ["flux_physical_basis", m.flux_physical_basis ?? null],
    ]),
    ...unitLines(lineCsvUnits(m)),
  ];
  return withProvenance(lines, body, "\n");
}
