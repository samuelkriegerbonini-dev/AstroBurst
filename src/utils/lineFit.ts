import type { CubeResult } from "../components/analysis/SpectroscopyPanel";
import type {
  ContinuumWindows,
  LineFitComponentFit,
  LineFitComponents,
  LineFitConfig,
  LineFitPlane,
  LineFitResult,
  LineFitSpaxel,
  LineFitUnits,
} from "../shared/types/cube";
import type { DisplayHint } from "../shared/types/preview";
import type { VelocityConvention } from "../shared/types/spectral";
import { PLUS_MINUS, UNAVAILABLE, formatQuantity, formatVelocity, type MeasurementRow, type PlotFrame, type PlotPoint } from "./lineMeasure";
import { finiteExtent, formatAxisTick, formatTickLabels, niceTicks } from "./plotScale";
import { formatAxisValue } from "./spectralAxis";
import {
  axisValueToPixel,
  channelAxisValue,
  defaultContinuumWindows,
  windowsAreValid,
  type ChannelRange,
  type PlotMapping,
} from "./spectrumRange";

export const LINE_FIT_LABEL_PREFIX = "Line fit";
export const DEFAULT_LINE_FIT_SNR = 3;
export const RESOLVING_POWER_HINT = "resolving power R of the grating, not the sampling; leave empty to keep sigma observed";
export const VELOCITY_PLANES: readonly LineFitPlane[] = ["velocity", "c1_velocity", "c2_velocity"];
export const INTEGER_PLANES: readonly LineFitPlane[] = ["mask", "ncomp"];
export const COMPONENT_COLORS: readonly string[] = ["rgba(96,165,250,0.95)", "rgba(248,113,113,0.95)"];

export type InspectSampleState = "used" | "dropped" | "unused";

export interface InspectPoint {
  x: number;
  y: number;
}

export interface InspectSample extends InspectPoint {
  channel: number;
  err: number | null;
  state: InspectSampleState;
}

export interface InspectMarker {
  x: number;
  label: string;
  component: number | null;
}

export interface LineFitInspectPlot {
  unit: string;
  xDomain: [number, number];
  yDomain: [number, number];
  xTicks: number[];
  xTickLabels: string[];
  yTicks: [number, number];
  yTickLabels: [string, string];
  samples: InspectSample[];
  lineWindow: [number, number];
  continuumWindows: [number, number][];
  continuum: InspectPoint[];
  total: InspectPoint[];
  components: InspectPoint[][];
  markers: InspectMarker[];
}

const VELOCITY_UNIT = "km/s";
const VELOCITY_DECIMALS = 1;
const SNR_DECIMALS = 1;
const MIN_Y_RANGE = 1e-10;
const COMPONENT_PREFIX = /^c([12])_(.+)$/;
const COMPONENT_NAMES = ["c1 (blue)", "c2 (red)"];
const UNRESOLVED_SIGMA = "unresolved (σ ≤ σ_inst)";
const MASK_FITTED_BIT = 1;
const NOT_ATTEMPTED_STATUS = "no fit attempted (fewer than 4 usable line samples or no usable continuum sample)";
const AXIS_UNIT_LABELS: Readonly<Record<string, string>> = { um: "μm" };
const INSPECT_TICK_COUNTS = [4, 5, 3, 6, 7];
const MIN_INSPECT_TICKS = 3;
const MAX_INSPECT_TICKS = 4;
const TICK_EPSILON = 1e-9;
const Y_PAD_FRACTION = 0.08;
const FLAT_PAD_FRACTION = 0.05;

const BASE_LABELS: Readonly<Record<string, string>> = {
  flux: "flux",
  velocity: "V",
  sigma_obs: "σ obs",
  sigma_corr: "σ corr",
  flux_err: "flux err",
  v_err: "V err",
  sigma_err: "σ err",
  chi2_red: "χ²_red",
  snr: "S/N",
  mask: "mask",
  ncomp: "ncomp",
};

const FLUX_PLANES = new Set(["flux", "flux_err"]);
const KMS_PLANES = new Set(["velocity", "v_err", "sigma_obs", "sigma_corr", "sigma_err"]);

const VELOCITY_HINT: DisplayHint = { colormap: "rdbu", invert: true, symmetric: true, centre: 0, stretch: "linear", limits: "percentile" };
const INTEGER_HINT: DisplayHint = { colormap: "gray", invert: false, symmetric: false, stretch: "linear", limits: "minmax" };
const VALUE_HINT: DisplayHint = { colormap: "viridis", invert: false, symmetric: false, stretch: "linear", limits: "percentile" };

export interface LineFitRun {
  key: string;
  filePath: string;
  config: LineFitConfig;
  result: LineFitResult;
  lineLabel: string | null;
}

export interface LineFitConfigInput {
  range: ChannelRange;
  windows: ContinuumWindows | null;
  channelCount: number;
  restUm: number | null;
  convention: VelocityConvention;
  snrText: string;
  emissionOnly: boolean;
  useErr: boolean;
  useDq: boolean;
  resolvingPowerText: string;
  components: LineFitComponents;
}

function finite(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function splitPlane(plane: LineFitPlane): { base: string; component: string | null } {
  const match = COMPONENT_PREFIX.exec(plane);
  return match ? { base: match[2], component: `c${match[1]}` } : { base: plane, component: null };
}

export function parseResolvingPower(text: string): number | null | { error: string } {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  if (!Number.isFinite(value) || value <= 0) return { error: `resolving power R must be a positive number, got "${trimmed}"` };
  return value;
}

export function lineFitConfig(input: LineFitConfigInput): LineFitConfig | { error: string } {
  const snrText = input.snrText.trim();
  const snr = Number(snrText);
  if (snrText === "" || !Number.isFinite(snr) || snr < 0) return { error: "line-fit S/N threshold must be a non-negative number" };
  const resolvingPower = parseResolvingPower(input.resolvingPowerText);
  if (resolvingPower !== null && typeof resolvingPower === "object") return resolvingPower;
  if (input.windows && !windowsAreValid(input.windows, input.channelCount)) {
    return { error: `continuum windows must be channel ranges inside 0–${Math.max(input.channelCount - 1, 0)}` };
  }
  return {
    z0: input.range.z0,
    z1: input.range.z1,
    rest_um: input.restUm,
    convention: input.convention,
    continuum: input.windows ?? defaultContinuumWindows(input.range, input.channelCount),
    snr_threshold: snr,
    emission_only: input.emissionOnly,
    use_err: input.useErr,
    use_dq: input.useDq,
    resolving_power: resolvingPower,
    components: input.components,
  };
}

export function planeLabel(plane: LineFitPlane): string {
  const { base, component } = splitPlane(plane);
  const label = BASE_LABELS[base] ?? base;
  return component ? `${label} ${component}` : label;
}

export function planeUnit(result: LineFitResult, plane: LineFitPlane): string | null {
  const { base } = splitPlane(plane);
  if (FLUX_PLANES.has(base)) return result.units.flux;
  if (KMS_PLANES.has(base)) return VELOCITY_UNIT;
  return null;
}

export function isVelocityPlane(plane: LineFitPlane): boolean {
  return VELOCITY_PLANES.includes(plane);
}

export function displayHintFor(plane: LineFitPlane): DisplayHint {
  if (isVelocityPlane(plane)) return { ...VELOCITY_HINT };
  if (INTEGER_PLANES.includes(plane)) return { ...INTEGER_HINT };
  return { ...VALUE_HINT };
}

export function lineFitResultLabel(result: LineFitResult, plane: LineFitPlane, lineLabel: string | null): string {
  const line = lineLabel ?? `ch ${result.z0}-${result.z1}`;
  return `${LINE_FIT_LABEL_PREFIX} ${planeLabel(plane)} · ${line} · ${result.n_fit} px`;
}

export function lineFitCubeResult(run: LineFitRun, plane: LineFitPlane): CubeResult | null {
  const files = run.result.planes[plane];
  if (!files?.previewUrl) return null;
  return {
    label: lineFitResultLabel(run.result, plane, run.lineLabel),
    previewUrl: files.previewUrl,
    fitsPath: files.fits_path || null,
    dimensions: run.result.dimensions ?? null,
    displayHint: displayHintFor(plane),
  };
}

export function displayedLineFitPlane(processed: { fitsPath: string | null } | null, result: LineFitResult): LineFitPlane | null {
  const fitsPath = processed?.fitsPath ?? null;
  if (!fitsPath) return null;
  return result.plane_order.find((plane) => result.planes[plane]?.fits_path === fitsPath) ?? null;
}

export function lineFitSummary(result: LineFitResult): string {
  const base = `${result.n_fit} fitted, ${result.n_masked} below S/N ${result.snr_threshold}, ${result.n_const_continuum} constant continuum, median χ²_red ${formatQuantity(result.median_chi2_red)}, ${result.elapsed_ms} ms`;
  return result.components === "one" ? base : `${base}, ${result.n_two_components} two-component`;
}

export function sigmaCaption(result: LineFitResult): string {
  return `σ ${result.sigma_label}`;
}

export function lineFitFrameNote(specsys: string | null): string {
  return specsys ? `line-fit V in the header frame ${specsys}` : "line-fit V in the header frame (no SPECSYS)";
}

function displayAxisValue(channel: number, mapping: PlotMapping): number {
  const n = mapping.n;
  const i = Math.floor(channel);
  const t = channel - i;
  if (t === 0 || i >= n - 1) return channelAxisValue(Math.min(i, n - 1), mapping);
  const a = channelAxisValue(i, mapping);
  return a + (channelAxisValue(i + 1, mapping) - a) * t;
}

export function lineFitOverlayPolylines(
  spaxel: LineFitSpaxel,
  mapping: PlotMapping,
  frame: PlotFrame,
): { continuum: PlotPoint[]; model: PlotPoint[]; components: PlotPoint[][] } {
  const model = spaxel.model;
  if (!model || mapping.n <= 0) return { continuum: [], model: [], components: [] };
  const left = mapping.padLeft;
  const right = mapping.width - mapping.padRight;
  const yRange = Math.max(frame.yMax - frame.yMin, MIN_Y_RANGE);
  const xs = model.channel.map((channel) => (finite(channel) && channel >= 0 ? axisValueToPixel(displayAxisValue(channel, mapping), mapping) : NaN));
  const polyline = (values: readonly (number | null)[], base: readonly (number | null)[] | null = null): PlotPoint[] => {
    const points: PlotPoint[] = [];
    xs.forEach((px, k) => {
      const offset = base ? base[k] : 0;
      const value = finite(values[k]) && finite(offset) ? (values[k] as number) + offset : null;
      if (!Number.isFinite(px) || !finite(value) || px < left || px > right) return;
      const py = frame.top + frame.height - ((value - frame.yMin) / yRange) * frame.height;
      points.push({ x: px, y: Math.min(frame.top + frame.height, Math.max(frame.top, py)) });
    });
    return points;
  };
  return {
    continuum: polyline(model.continuum),
    model: polyline(model.total),
    components: model.components.map((component) => polyline(component, model.continuum)),
  };
}

function formatKms(value: number | null | undefined): string {
  return finite(value) ? value.toFixed(VELOCITY_DECIMALS) : UNAVAILABLE;
}

function kmsWithError(value: number | null | undefined, error: number | null | undefined): string {
  if (!finite(value)) return UNAVAILABLE;
  return `${formatKms(value)} ${PLUS_MINUS} ${formatKms(error)} ${VELOCITY_UNIT}`;
}

function quantityWithError(value: number | null | undefined, error: number | null | undefined, unit: string): string {
  if (!finite(value)) return UNAVAILABLE;
  return `${formatQuantity(value)} ${PLUS_MINUS} ${formatQuantity(error)}${unit ? ` ${unit}` : ""}`;
}

function unresolvedSigma(fit: LineFitComponentFit, resolvingPower: number | null): boolean {
  return resolvingPower !== null && !finite(fit.sigma_corr_kms) && finite(fit.sigma_kms);
}

function sigmaRow(fit: LineFitComponentFit | null, resolvingPower: number | null): MeasurementRow {
  if (fit && finite(fit.sigma_corr_kms)) {
    return {
      label: "σ corr",
      value: `${formatKms(fit.sigma_corr_kms)} ${VELOCITY_UNIT} (obs ${formatKms(fit.sigma_kms)} ${PLUS_MINUS} ${formatKms(fit.sigma_err_kms)})`,
    };
  }
  if (fit && unresolvedSigma(fit, resolvingPower)) {
    return { label: "σ corr", value: `${UNRESOLVED_SIGMA}, obs ${kmsWithError(fit.sigma_kms, fit.sigma_err_kms)}` };
  }
  return { label: "σ obs", value: kmsWithError(fit?.sigma_kms, fit?.sigma_err_kms) };
}

function convergenceStatus(spaxel: LineFitSpaxel): string {
  if (!spaxel.converged || !spaxel.single) return "did not converge";
  return (spaxel.mask & MASK_FITTED_BIT) === 0 ? "converged, below S/N threshold" : "converged";
}

function fitStatus(spaxel: LineFitSpaxel): string {
  if (spaxel.ncomp === 0) return NOT_ATTEMPTED_STATUS;
  return `${convergenceStatus(spaxel)}, ${spaxel.iterations} it, weights ${spaxel.weighting}, mask ${spaxel.mask}`;
}

function deltaBicText(spaxel: LineFitSpaxel): string {
  if (!finite(spaxel.delta_bic)) return UNAVAILABLE;
  const raw = `${formatQuantity(spaxel.delta_bic)} raw`;
  if (!finite(spaxel.chi2_red)) return raw;
  return `${raw}, ${formatQuantity(spaxel.delta_bic / Math.max(1, spaxel.chi2_red))} / max(1, χ²_red)`;
}

function componentSigma(fit: LineFitComponentFit, resolvingPower: number | null): string {
  if (finite(fit.sigma_corr_kms)) return `σ corr ${formatKms(fit.sigma_corr_kms)} ${VELOCITY_UNIT}`;
  const observed = kmsWithError(fit.sigma_kms, fit.sigma_err_kms);
  return unresolvedSigma(fit, resolvingPower) ? `σ corr ${UNRESOLVED_SIGMA}, obs ${observed}` : `σ ${observed}`;
}

function componentRow(fit: LineFitComponentFit, index: number, units: LineFitUnits, resolvingPower: number | null): MeasurementRow {
  const sigma = componentSigma(fit, resolvingPower);
  return {
    label: COMPONENT_NAMES[index] ?? `c${index + 1}`,
    value: `V ${kmsWithError(fit.velocity_kms, fit.v_err_kms)}, ${sigma}, flux ${quantityWithError(fit.flux, fit.flux_err, units.flux)}, A ${quantityWithError(fit.amplitude, fit.amplitude_err, "")}`,
  };
}

export function lineFitSpaxelRows(spaxel: LineFitSpaxel, units: LineFitUnits, resolvingPower: number | null = null): MeasurementRow[] {
  const fit = spaxel.single;
  const continuum = spaxel.continuum;
  const rows: MeasurementRow[] = [
    { label: "V", value: kmsWithError(fit?.velocity_kms, fit?.v_err_kms) },
    sigmaRow(fit, resolvingPower),
    { label: "Flux", value: quantityWithError(fit?.flux, fit?.flux_err, units.flux) },
    { label: "Amplitude", value: quantityWithError(fit?.amplitude, fit?.amplitude_err, "") },
    {
      label: "χ²_red",
      value: finite(spaxel.chi2_red) ? `${formatQuantity(spaxel.chi2_red)} (dof ${spaxel.dof})` : UNAVAILABLE,
    },
    { label: "S/N", value: finite(fit?.snr) ? fit.snr.toFixed(SNR_DECIMALS) : UNAVAILABLE },
    { label: "ncomp", value: String(spaxel.ncomp) },
    { label: "ΔBIC", value: deltaBicText(spaxel) },
    { label: "Dropped", value: `DQ ${spaxel.dropped_dq.length} ch, ERR ${spaxel.dropped_err.length} ch` },
    {
      label: "Continuum",
      value: continuum
        ? `${continuum.linear ? "linear" : "constant"}, ${continuum.channels} ch, level ${formatQuantity(continuum.intercept)}, sigma ${formatQuantity(continuum.sigma)}`
        : UNAVAILABLE,
    },
    { label: "Fit", value: fitStatus(spaxel) },
  ];
  if (spaxel.ncomp === 2) spaxel.components.forEach((c, i) => rows.push(componentRow(c, i, units, resolvingPower)));
  return rows;
}

export function lineFitSpaxelNotes(spaxel: LineFitSpaxel, runNotes: readonly string[]): string[] {
  const shown = new Set(runNotes);
  return spaxel.notes.filter((note) => !shown.has(note));
}

export function inspectKey(filePath: string, x: number, y: number, run: LineFitRun): string {
  return `${run.key}|${filePath}|${x},${y}`;
}

export function spanAxisValue(spaxel: Pick<LineFitSpaxel, "axis" | "span">, channel: number): number {
  const axis = spaxel.axis;
  const n = axis.length;
  const local = channel - spaxel.span[0];
  if (n === 0 || !Number.isFinite(local)) return NaN;
  const i = Math.max(0, Math.floor(local));
  const t = local - i;
  if (t <= 0 || i >= n - 1) {
    const v = axis[Math.min(i, n - 1)];
    return finite(v) ? v : NaN;
  }
  const a = axis[i];
  const b = axis[i + 1];
  return finite(a) && finite(b) ? a + (b - a) * t : NaN;
}

function sampleState(channel: number, spaxel: LineFitSpaxel, dropped: ReadonlySet<number>, used: ReadonlySet<number>): InspectSampleState {
  if (dropped.has(channel)) return "dropped";
  if (used.has(channel)) return "used";
  return spaxel.continuum_windows.some(([a, b]) => channel >= a && channel <= b) ? "used" : "unused";
}

function inspectPoints(xs: readonly number[], values: readonly (number | null)[], base: readonly (number | null)[] | null): InspectPoint[] {
  const points: InspectPoint[] = [];
  xs.forEach((x, k) => {
    const value = values[k];
    const offset = base ? base[k] : 0;
    if (finite(x) && finite(value) && finite(offset)) points.push({ x, y: value + offset });
  });
  return points;
}

function windowBand(spaxel: LineFitSpaxel, z0: number, z1: number): [number, number] {
  const a = spanAxisValue(spaxel, z0 - 0.5);
  const b = spanAxisValue(spaxel, z1 + 0.5);
  return [Math.min(a, b), Math.max(a, b)];
}

function inspectTicks(lo: number, hi: number): number[] {
  const eps = (hi - lo) * TICK_EPSILON;
  for (const count of INSPECT_TICK_COUNTS) {
    const ticks = niceTicks(lo, hi, count).filter((t) => t >= lo - eps && t <= hi + eps);
    if (ticks.length >= MIN_INSPECT_TICKS && ticks.length <= MAX_INSPECT_TICKS) return ticks;
  }
  return Array.from({ length: MAX_INSPECT_TICKS }, (_, k) => lo + ((hi - lo) * k) / (MAX_INSPECT_TICKS - 1));
}

function paddedDomain([lo, hi]: [number, number]): [number, number] {
  const pad = hi > lo ? (hi - lo) * Y_PAD_FRACTION : Math.max(Math.abs(hi), 1) * FLAT_PAD_FRACTION;
  return [lo - pad, hi + pad];
}

function fittedComponents(spaxel: LineFitSpaxel): LineFitComponentFit[] {
  if (spaxel.ncomp === 2) return spaxel.components;
  return spaxel.single ? [spaxel.single] : [];
}

export function lineFitInspectPlot(spaxel: LineFitSpaxel): LineFitInspectPlot {
  const dropped = new Set([...spaxel.dropped_dq, ...spaxel.dropped_err]);
  const used = new Set(spaxel.channels);
  const samples: InspectSample[] = [];
  spaxel.axis.forEach((x, k) => {
    const y = spaxel.flux[k];
    if (!finite(x) || !finite(y)) return;
    const channel = spaxel.span[0] + k;
    const e = spaxel.err ? spaxel.err[k] : null;
    samples.push({ channel, x, y, err: finite(e) && e > 0 ? e : null, state: sampleState(channel, spaxel, dropped, used) });
  });
  const model = spaxel.model;
  const xs = model ? model.channel.map((c) => (finite(c) ? spanAxisValue(spaxel, c) : NaN)) : [];
  const continuum = model ? inspectPoints(xs, model.continuum, null) : [];
  const total = model ? inspectPoints(xs, model.total, null) : [];
  const components = model && spaxel.ncomp === 2 ? model.components.map((c) => inspectPoints(xs, c, model.continuum)) : [];
  const markers: InspectMarker[] = [];
  fittedComponents(spaxel).forEach((fit, i) => {
    if (!finite(fit.centre)) return;
    const velocity = formatVelocity(fit.velocity_kms);
    markers.push(spaxel.ncomp === 2 ? { x: fit.centre, label: `c${i + 1} ${velocity}`, component: i } : { x: fit.centre, label: velocity, component: null });
  });
  const kept = samples.filter((s) => s.state !== "dropped").map((s) => s.y);
  const curves = [continuum, total, ...components].flatMap((curve) => curve.map((p) => p.y));
  const yTicks = finiteExtent([...kept, ...curves]) ?? finiteExtent(samples.map((s) => s.y)) ?? [0, 1];
  const xExtent = finiteExtent(spaxel.axis) ?? [0, 1];
  const xDomain: [number, number] = xExtent[1] > xExtent[0] ? xExtent : paddedDomain(xExtent);
  const xTicks = inspectTicks(xDomain[0], xDomain[1]);
  const yRange = yTicks[1] - yTicks[0];
  const [w0, w1] = spaxel.continuum_windows;
  return {
    unit: AXIS_UNIT_LABELS[spaxel.axis_unit] ?? spaxel.axis_unit,
    xDomain,
    yDomain: paddedDomain(yTicks),
    xTicks,
    xTickLabels: formatTickLabels(xTicks),
    yTicks,
    yTickLabels: [formatAxisTick(yTicks[0], yRange), formatAxisTick(yTicks[1], yRange)],
    samples,
    lineWindow: windowBand(spaxel, spaxel.z0, spaxel.z1),
    continuumWindows: [windowBand(spaxel, w0[0], w0[1]), windowBand(spaxel, w1[0], w1[1])],
    continuum,
    total,
    components,
    markers,
  };
}

export function lineFitInspectSummary(spaxel: LineFitSpaxel, plot: LineFitInspectPlot): string {
  const count = (state: InspectSampleState) => plot.samples.filter((s) => s.state === state).length;
  const unit = plot.unit ? ` ${plot.unit}` : "";
  const range = `${formatAxisValue(plot.xDomain[0], spaxel.axis_unit)}-${formatAxisValue(plot.xDomain[1], spaxel.axis_unit)}${unit}`;
  const [w0, w1] = spaxel.continuum_windows;
  const head = `Fitted spectrum of spaxel (${spaxel.x}, ${spaxel.y}), ch ${spaxel.span[0]}-${spaxel.span[1]} (${range}): ${plot.samples.length} samples, ${count("used")} used, ${count("dropped")} dropped by DQ or ERR, ${count("unused")} outside the windows`;
  const windows = `line window ch ${spaxel.z0}-${spaxel.z1}, continuum ch ${w0[0]}-${w0[1]} and ${w1[0]}-${w1[1]}`;
  const model =
    plot.total.length === 0
      ? "no model (no fit attempted or the fit did not converge)"
      : plot.components.length > 0
        ? `continuum, total model and ${plot.components.length} components drawn`
        : "continuum and total model drawn";
  const centres = plot.markers.map((m) => (m.component === null ? `centre ${m.label}` : m.label)).join(", ");
  return `${head}; ${windows}; ${model}${centres ? `; ${centres}` : ""}.`;
}
