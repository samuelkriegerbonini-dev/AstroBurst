import { effectiveFrameChoice, effectiveMode, type SpectrumViewState, type YLimitsMode } from "../hooks/useSpectrumViewStore";
import type { CorrectionFrame, RadialVelocityCorrectionResult, SpectralAxisInfo, SpectralAxisMode } from "../shared/types/spectral";
import type { FluxDisplayChoice, Spectrum1d, WavelengthDisplayUnit, WavelengthFrameChoice } from "../shared/types/spectrum1d";
import { formatFrameDelta } from "./cubeNavigation";
import { bandMarks, lineById, lineListAvailability, lineMarks, type BandMark, type LineListAvailability, type LineMark } from "./lineList";
import { PLUS_MINUS, formatQuantity } from "./lineMeasure";
import { clampDomain, logTicks } from "./plotInteraction";
import { formatAxisTick, formatLogTickLabel } from "./plotScale";
import { errorBandKind, type ErrorBandKind, type RegionViewErrors } from "./regionErrors";
import { applyCorrectionKms, formatAxis, formatAxisValue, modeLabel, type FormattedAxis } from "./spectralAxis";
import { binAxis, binSpectrum, binningNote, effectiveBinFactor, type BinnedSpectrum } from "./spectrumBinning";
import { documentAxis, documentDqExclusions } from "./spectrumDocument";
import { fluxUnitLabel } from "./spectrumExport";
import { channelWidthAt, clampRange, nearestChannel, type ChannelRange, type PlotMapping } from "./spectrumRange";
import {
  WAVELENGTH_UNIT_FACTOR,
  WAVELENGTH_UNIT_LABEL,
  fluxAxisTitle,
  fluxDisplay,
  scaleWavelengths,
  wavelengthUnitApplies,
} from "./spectrumUnits";

export const SPECTRUM_PLOT_HEIGHT = 180;
export const SPECTRUM_PLOT_PAD = { top: 10, bottom: 24, left: 62, right: 12 } as const;

const Y_PAD_FRACTION = 0.05;
const ROBUST_LOW_PERCENTILE = 1;
const ROBUST_HIGH_PERCENTILE = 99;
const FAR_OUTLIER_SPANS = 10;
const LINEAR_Y_TICKS = 4;
const MAX_LOG_TICKS = 10;

export interface SpectrumPlotView {
  xDomain: [number, number] | null;
  logY: boolean;
  yLimits: YLimitsMode;
}

const DEFAULT_PLOT_VIEW: SpectrumPlotView = { xDomain: null, logY: false, yLimits: "full" };

export interface SpectrumFluxState {
  choice: FluxDisplayChoice;
  scale: number | null;
  invertY: boolean;
  note: string | null;
}

export const NATIVE_FLUX_STATE: Readonly<SpectrumFluxState> = Object.freeze({ choice: "native", scale: 1, invertY: false, note: null });

export interface SpectrumAxisInput {
  formatted: FormattedAxis | null;
  mode: SpectralAxisMode;
  correction: CorrectionFrame;
  correctionResult: RadialVelocityCorrectionResult | null;
  fallbackWavelengths: number[] | null;
  fallbackUnit: string | null | undefined;
  wavelengthUnit?: WavelengthDisplayUnit;
}

export interface SpectrumBinsInput {
  factor: number;
  first: number[];
  last: number[];
  nUsed: number[];
  y: number[];
  err: number[] | null;
  note: string;
}

export interface SpectrumPlotBins extends SpectrumBinsInput {
  x: number[];
}

export interface SpectrumPlotInput {
  series: number[];
  dqExcluded: boolean[] | null;
  axis: SpectrumAxisInput;
  errors: RegionViewErrors | null;
  readoutErr: number[] | null;
  fluxUnit: string;
  color: string;
  yLabel: string | null;
  errorsWidenRange: boolean;
  view?: SpectrumPlotView;
  fluxState?: SpectrumFluxState;
  bins?: SpectrumBinsInput | null;
}

export interface SpectrumPlotModel {
  n: number;
  xValues: number[] | null;
  xLabel: string;
  xUnit: string;
  xScale: number;
  wavelengthUnit: WavelengthDisplayUnit | null;
  fluxState: SpectrumFluxState;
  series: number[];
  dqExcluded: boolean[] | null;
  errors: RegionViewErrors | null;
  errorBand: ErrorBandKind;
  readoutErr: number[] | null;
  fluxUnit: string;
  color: string;
  yLabel: string | null;
  xMin: number;
  xMax: number;
  xExtent: [number, number];
  yMin: number;
  yMax: number;
  logY: boolean;
  offScale: number;
  farOutliers: number;
  logHidden: number;
  bins: SpectrumPlotBins | null;
}

interface PlotAxis {
  values: number[] | null;
  label: string;
  unit: string;
  scale: number;
  wavelengthUnit: WavelengthDisplayUnit | null;
}

function arrayMinMax(arr: number[]): [number, number] {
  let min = Infinity;
  let max = -Infinity;
  for (let i = 0; i < arr.length; i++) {
    const v = arr[i];
    if (Number.isFinite(v)) {
      if (v < min) min = v;
      if (v > max) max = v;
    }
  }
  return [min === Infinity ? 0 : min, max === -Infinity ? 1 : max];
}

function legacyConversion(rawUnit: string | null | undefined): { factor: number; label: string } {
  const unit = rawUnit ? rawUnit.trim().toUpperCase() : null;
  if (!unit) return { factor: 1, label: "Channel" };
  if (unit === "M") return { factor: 1e6, label: "μm" };
  if (unit === "CM") return { factor: 1e4, label: "μm" };
  if (unit === "MM") return { factor: 1e3, label: "μm" };
  if (unit === "UM") return { factor: 1, label: "μm" };
  if (unit === "NM") return { factor: 1, label: "nm" };
  if (unit === "ANGSTROM" || unit === "A") return { factor: 0.1, label: "nm" };
  if (unit === "HZ") return { factor: 1e-9, label: "GHz" };
  if (unit === "KHZ") return { factor: 1e-6, label: "GHz" };
  if (unit === "MHZ") return { factor: 1e-3, label: "GHz" };
  if (unit === "GHZ") return { factor: 1, label: "GHz" };
  return { factor: 1, label: unit.toLowerCase() };
}

function plotAxis(input: SpectrumAxisInput, n: number): PlotAxis {
  const { formatted, mode, correction, correctionResult, fallbackWavelengths } = input;
  if (formatted && n > 0 && formatted.values.length === n) {
    if (wavelengthUnitApplies(mode, formatted.unit)) {
      const unit = input.wavelengthUnit ?? "um";
      const values = unit === "um" ? formatted.values : scaleWavelengths(formatted.values, unit);
      const label = WAVELENGTH_UNIT_LABEL[unit];
      return { values, label: `${modeLabel(mode)} (${label})`, unit: label, scale: WAVELENGTH_UNIT_FACTOR[unit], wavelengthUnit: unit };
    }
    const values = mode === "velocity" ? applyCorrectionKms(formatted.values, correctionResult, correction) : formatted.values;
    const label = mode === "velocity" && correction !== "none" && correctionResult ? `${formatted.label}, ${correction}` : formatted.label;
    return { values, label, unit: formatted.unit, scale: 1, wavelengthUnit: null };
  }
  if (fallbackWavelengths && n > 0 && fallbackWavelengths.length === n) {
    const conv = legacyConversion(input.fallbackUnit);
    return {
      values: fallbackWavelengths.map((w) => w * conv.factor),
      label: `Wavelength (${conv.label})`,
      unit: conv.label,
      scale: 1,
      wavelengthUnit: null,
    };
  }
  return { values: null, label: "Channel", unit: "ch", scale: 1, wavelengthUnit: null };
}

export function percentile(sorted: ArrayLike<number>, p: number): number {
  const m = sorted.length;
  if (m === 0) return NaN;
  const position = (p / 100) * (m - 1);
  const below = Math.floor(position);
  const above = Math.min(below + 1, m - 1);
  return sorted[below] + (sorted[above] - sorted[below]) * (position - below);
}

function paddedRange(lo: number, hi: number): [number, number] {
  const span = hi - lo;
  const pad = span > 0 ? span * Y_PAD_FRACTION : Math.max(Math.abs(hi), 1) * Y_PAD_FRACTION;
  return [lo - pad, hi + pad];
}

function paddedLogRange(a: number, b: number): [number, number] {
  const la = Math.log10(a);
  const lb = Math.log10(b);
  const d = lb - la;
  if (d === 0) return [a / Math.sqrt(10), a * Math.sqrt(10)];
  return [10 ** (la - Y_PAD_FRACTION * d), 10 ** (lb + Y_PAD_FRACTION * d)];
}

type PlotLimits = Pick<SpectrumPlotModel, "xMin" | "xMax" | "xExtent" | "yMin" | "yMax" | "logY" | "offScale" | "farOutliers" | "logHidden">;

interface PlottedSamples {
  x: number[] | null;
  y: number[];
  errors: RegionViewErrors | null;
}

function plotLimits(n: number, xValues: number[] | null, plotted: PlottedSamples, view: SpectrumPlotView): PlotLimits {
  if (n === 0) return { xMin: 0, xMax: 1, xExtent: [0, 1], yMin: 0, yMax: 1, logY: false, offScale: 0, farOutliers: 0, logHidden: 0 };
  const xExtent: [number, number] = xValues ? arrayMinMax(xValues) : [0, n - 1];
  const domain = view.xDomain;
  const [xMin, xMax] = domain !== null && domain[1] - domain[0] > 0 ? clampDomain(domain, xExtent) : xExtent;

  const { errors } = plotted;
  const samples = plotted.y.length;
  const visible = new Float64Array(samples);
  let count = 0;
  let bandLow = Infinity;
  let bandHigh = -Infinity;
  for (let i = 0; i < samples; i++) {
    const x = plotted.x ? plotted.x[i] : i;
    const y = plotted.y[i];
    if (!(x >= xMin && x <= xMax) || !Number.isFinite(y)) continue;
    visible[count++] = y;
    if (errors === null) continue;
    for (const band of [errors.err, errors.scaled]) {
      const e = band !== null && band.length === samples ? band[i] : NaN;
      if (!Number.isFinite(e) || e < 0) continue;
      bandLow = Math.min(bandLow, y - e);
      bandHigh = Math.max(bandHigh, y + e);
    }
  }
  const sorted = visible.subarray(0, count).sort();
  const lo = percentile(sorted, ROBUST_LOW_PERCENTILE);
  const hi = percentile(sorted, ROBUST_HIGH_PERCENTILE);
  const spread = hi - lo;
  let farOutliers = 0;
  if (spread > 0) {
    for (let k = 0; k < count; k++) {
      if (sorted[k] < lo - FAR_OUTLIER_SPANS * spread || sorted[k] > hi + FAR_OUTLIER_SPANS * spread) farOutliers++;
    }
  }

  let firstPositive = 0;
  while (firstPositive < count && sorted[firstPositive] <= 0) firstPositive++;
  const logY = view.logY && firstPositive < count;
  const robust = view.yLimits === "robust";
  let yRange: [number, number];
  if (count === 0) yRange = paddedRange(0, 1);
  else if (logY) {
    const positive = sorted.subarray(firstPositive);
    const a = robust ? percentile(positive, ROBUST_LOW_PERCENTILE) : positive[0];
    const b = robust ? percentile(positive, ROBUST_HIGH_PERCENTILE) : positive[positive.length - 1];
    yRange = paddedLogRange(a, b);
  } else if (robust) yRange = paddedRange(lo, hi);
  else yRange = paddedRange(Math.min(sorted[0], bandLow), Math.max(sorted[count - 1], bandHigh));
  const [yMin, yMax] = yRange;

  let offScale = 0;
  for (let k = logY ? firstPositive : 0; k < count; k++) {
    if (sorted[k] < yMin || sorted[k] > yMax) offScale++;
  }
  return { xMin, xMax, xExtent, yMin, yMax, logY, offScale, farOutliers, logHidden: logY ? firstPositive : 0 };
}

export function buildSpectrumPlotModel(input: SpectrumPlotInput): SpectrumPlotModel {
  const n = input.series.length;
  const axis = plotAxis(input.axis, n);
  const dqExcluded = input.dqExcluded;
  const series = dqExcluded ? input.series.map((value, i) => (dqExcluded[i] ? NaN : value)) : input.series;
  const fluxState = input.fluxState ?? NATIVE_FLUX_STATE;
  const view = input.view ?? DEFAULT_PLOT_VIEW;
  const binsIn = input.bins ?? null;
  const bins =
    binsIn === null
      ? null
      : { ...binsIn, x: axis.values ? binAxis(axis.values, binsIn.factor) : binsIn.first.map((a, k) => (a + binsIn.last[k]) / 2) };
  const plotted: PlottedSamples =
    bins === null
      ? { x: axis.values, y: series, errors: input.errorsWidenRange ? input.errors : null }
      : { x: bins.x, y: bins.y, errors: input.errorsWidenRange ? { err: bins.err, scaled: null } : null };
  return {
    n,
    xValues: axis.values,
    xLabel: axis.label,
    xUnit: axis.unit,
    xScale: axis.scale,
    wavelengthUnit: axis.wavelengthUnit,
    fluxState,
    series,
    dqExcluded,
    errors: input.errors,
    errorBand: errorBandKind(input.errors),
    readoutErr: input.readoutErr,
    fluxUnit: input.fluxUnit,
    color: input.color,
    yLabel: input.yLabel,
    ...plotLimits(n, axis.values, plotted, { ...view, logY: view.logY && !fluxState.invertY }),
    bins,
  };
}

export interface SpectrumPlotPartsInput {
  axis: SpectralAxisInfo | null;
  mode: SpectralAxisMode;
  view: SpectrumViewState;
  correction: CorrectionFrame;
  correctionResult: RadialVelocityCorrectionResult | null;
  shiftKms: number | null;
  series: number[];
  dqExcluded: boolean[] | null;
  errors: RegionViewErrors | null;
  errorsWidenRange: boolean;
  readoutErr: number[] | null;
  fluxUnit: string;
  color: string;
  fallbackWavelengths: number[] | null;
  fallbackUnit: string | null | undefined;
  nativeFluxUnit?: string | null;
}

export interface SpectrumPlotParts {
  formatted: FormattedAxis | null;
  availability: LineListAvailability;
  model: SpectrumPlotModel;
  marks: LineMark[];
  bands: BandMark[];
  pickedMarkId: string | null;
}

export interface DocumentPlot extends SpectrumPlotParts {
  axis: SpectralAxisInfo;
  frameChoice: WavelengthFrameChoice | null;
  mode: SpectralAxisMode;
}

type DisplayedFlux = Pick<SpectrumPlotInput, "series" | "errors" | "readoutErr" | "fluxUnit" | "yLabel" | "fluxState" | "bins">;

function vacuumWavelengthsFor(axis: SpectralAxisInfo | null, view: SpectrumViewState, n: number): number[] | null {
  if (axis === null || (axis.kind !== "wave" && axis.kind !== "awav" && axis.kind !== "freq")) return null;
  const values = formatAxis(axis, "wavelength_vac", null, view.convention)?.values;
  return values !== undefined && values.length === n ? values : null;
}

function binsInput(binned: BinnedSpectrum, y: number[], err: number[] | null): SpectrumBinsInput {
  return { factor: binned.factor, first: binned.first, last: binned.last, nUsed: binned.nUsed, y, err, note: binningNote(binned) };
}

function displayedFlux(input: SpectrumPlotPartsInput): DisplayedFlux {
  const { nativeFluxUnit, view, series, errors } = input;
  const factor = effectiveBinFactor(view.binFactor, series.length);
  const binned = factor > 1 ? binSpectrum(series, errors?.err ?? null, input.dqExcluded, factor) : null;
  if (typeof nativeFluxUnit !== "string" || view.fluxUnit === "native") {
    return {
      series,
      errors,
      readoutErr: input.readoutErr,
      fluxUnit: input.fluxUnit,
      yLabel: `Flux (${input.fluxUnit})`,
      fluxState: NATIVE_FLUX_STATE,
      bins: binned === null ? null : binsInput(binned, binned.flux, binned.err),
    };
  }
  const vacuumUm = vacuumWavelengthsFor(input.axis, view, series.length);
  const shown = fluxDisplay({ flux: series, err: errors?.err ?? null, vacuumUm, fluxUnit: nativeFluxUnit }, view.fluxUnit);
  const shownBins =
    binned === null
      ? null
      : fluxDisplay(
          { flux: binned.flux, err: binned.err, vacuumUm: vacuumUm === null ? null : binAxis(vacuumUm, factor), fluxUnit: nativeFluxUnit },
          view.fluxUnit,
        );
  return {
    series: shown.values,
    errors: errors ? { err: shown.errors, scaled: null } : null,
    readoutErr: shown.errors,
    fluxUnit: shown.unit,
    yLabel: fluxAxisTitle(shown),
    fluxState: { choice: shown.choice, scale: shown.scale, invertY: shown.invertY, note: shown.reason },
    bins: binned === null || shownBins === null ? null : binsInput(binned, shownBins.values, shownBins.errors),
  };
}

export function spectrumPlotParts(input: SpectrumPlotPartsInput): SpectrumPlotParts {
  const { axis, mode, view } = input;
  const formatted = formatAxis(axis, mode, view.restUm, view.convention);
  const model = buildSpectrumPlotModel({
    ...displayedFlux(input),
    dqExcluded: input.dqExcluded,
    axis: {
      formatted,
      mode,
      correction: input.correction,
      correctionResult: input.correctionResult,
      fallbackWavelengths: input.fallbackWavelengths,
      fallbackUnit: input.fallbackUnit,
      wavelengthUnit: view.wavelengthUnit,
    },
    color: input.color,
    errorsWidenRange: input.errorsWidenRange,
    view: { xDomain: view.xDomain, logY: view.logY, yLimits: view.yLimits },
  });
  const availability = lineListAvailability(axis, mode, view.restUm, formatted, input.series.length);
  const marksInput =
    availability.ok && axis
      ? {
          axis,
          mode,
          restUm: view.restUm,
          convention: view.convention,
          shiftKms: input.shiftKms,
          redshift: view.redshift,
          families: view.lineFamilies,
          xMin: model.xMin / model.xScale,
          xMax: model.xMax / model.xScale,
        }
      : null;
  const marks = view.lineListVisible && marksInput !== null ? lineMarks(marksInput) : [];
  const bands = view.bandSpecies.length > 0 && marksInput !== null ? bandMarks({ ...marksInput, species: view.bandSpecies }) : [];
  const pickedLine = view.pickedLineId === null ? null : lineById(view.pickedLineId);
  const pickedMarkId = pickedLine !== null && view.restUm === pickedLine.vacuumUm ? pickedLine.id : null;
  return { formatted, availability, model, marks, bands, pickedMarkId };
}

export function documentPlot(spectrum: Spectrum1d, view: SpectrumViewState): DocumentPlot {
  const frameChoice = effectiveFrameChoice(view, spectrum);
  const mode = effectiveMode(view, spectrum);
  const axis = documentAxis(spectrum.axis, frameChoice);
  const parts = spectrumPlotParts({
    axis,
    mode,
    view,
    correction: "none",
    correctionResult: null,
    shiftKms: null,
    series: spectrum.flux,
    dqExcluded: view.useDq ? documentDqExclusions(spectrum) : null,
    errors: spectrum.err ? { err: spectrum.err, scaled: null } : null,
    errorsWidenRange: false,
    readoutErr: spectrum.err,
    fluxUnit: fluxUnitLabel(spectrum.flux_unit, "sum", "file"),
    color: "#a855f7",
    fallbackWavelengths: null,
    fallbackUnit: null,
    nativeFluxUnit: spectrum.flux_unit,
  });
  return { ...parts, axis, frameChoice, mode };
}

export function spectrumPlotMapping(model: SpectrumPlotModel, width: number): PlotMapping {
  return {
    width,
    padLeft: SPECTRUM_PLOT_PAD.left,
    padRight: SPECTRUM_PLOT_PAD.right,
    xMin: model.xMin,
    xMax: model.xMax,
    xValues: model.xValues,
    n: model.n,
  };
}

export function bandChannelRange(band: Pick<BandMark, "lo" | "hi">, model: SpectrumPlotModel): ChannelRange | null {
  const m = spectrumPlotMapping(model, 400);
  const a = nearestChannel(band.lo * model.xScale, m);
  const b = nearestChannel(band.hi * model.xScale, m);
  if (a === null || b === null) return null;
  return clampRange({ z0: Math.min(a, b), z1: Math.max(a, b) }, model.n);
}

export function rangeBand(range: ChannelRange | null, bands: readonly BandMark[], model: SpectrumPlotModel): BandMark | null {
  if (range === null) return null;
  return (
    bands.find((band) => {
      const covered = bandChannelRange(band, model);
      return covered !== null && covered.z0 === range.z0 && covered.z1 === range.z1;
    }) ?? null
  );
}

export function spectrumPlottedMapping(model: SpectrumPlotModel, width: number): PlotMapping {
  const mapping = spectrumPlotMapping(model, width);
  return model.bins === null ? mapping : { ...mapping, xValues: model.bins.x, n: model.bins.y.length };
}

export function plottedCount(model: SpectrumPlotModel): number {
  return model.bins === null ? model.n : model.bins.y.length;
}

interface SpectrumYTick {
  value: number;
  label: string;
}

function spectrumYTicks(model: SpectrumPlotModel, range: number): SpectrumYTick[] {
  if (!model.logY) {
    return Array.from({ length: LINEAR_Y_TICKS + 1 }, (_, i) => {
      const value = model.yMax - (i / LINEAR_Y_TICKS) * range;
      return { value, label: formatAxisTick(value, range) };
    });
  }
  const values = logTicks(model.yMin, model.yMax);
  const shown = values.length > MAX_LOG_TICKS ? values.filter((v) => v === 10 ** Math.round(Math.log10(v))) : values;
  return shown.map((value) => ({ value, label: formatLogTickLabel(value) }));
}

export function spectrumYScale(
  model: SpectrumPlotModel,
  height: number,
): { plotTop: number; plotBottom: number; plotHeight: number; range: number; toY: (value: number) => number; ticks: SpectrumYTick[] } {
  const plotTop = SPECTRUM_PLOT_PAD.top;
  const plotBottom = height - SPECTRUM_PLOT_PAD.bottom;
  const plotHeight = plotBottom - plotTop;
  const { yMin, yMax } = model;
  const range = yMax > yMin ? yMax - yMin : 1;
  const ticks = spectrumYTicks(model, range);
  if (model.fluxState.invertY) {
    return { plotTop, plotBottom, plotHeight, range, ticks, toY: (value: number) => plotTop + ((value - yMin) / range) * plotHeight };
  }
  if (!model.logY) {
    return { plotTop, plotBottom, plotHeight, range, ticks, toY: (value: number) => plotTop + plotHeight - ((value - yMin) / range) * plotHeight };
  }
  const logMin = Math.log10(yMin);
  const logRange = Math.log10(yMax) - logMin;
  const toY = (value: number) => (value > 0 ? plotTop + plotHeight - ((Math.log10(value) - logMin) / logRange) * plotHeight : plotBottom + 1);
  return { plotTop, plotBottom, plotHeight, range, ticks, toY };
}

function readoutFlux(y: number, err: number | undefined, unit: string): string {
  return `${formatQuantity(y)}${Number.isFinite(err) ? ` ${PLUS_MINUS} ${formatQuantity(err)}` : ""} ${unit}`;
}

function binReadout(model: SpectrumPlotModel, bins: SpectrumPlotBins, index: number): string | null {
  const x = bins.x[index];
  const y = bins.y[index];
  if (!Number.isFinite(y) || !Number.isFinite(x)) return null;
  const first = bins.first[index];
  const last = bins.last[index];
  const channels = first === last ? `ch ${first}` : `ch ${first}–${last}`;
  const flux = readoutFlux(y, bins.err?.[index], model.fluxUnit);
  return model.xValues
    ? `bin ${index} (${channels}) · ${formatAxisValue(x, model.xUnit, channelWidthAt(bins.x, index))} ${model.xUnit} → ${flux}`
    : `bin ${index} (${channels}) → ${flux}`;
}

export function spectrumReadout(model: SpectrumPlotModel, channel: number, referenceChannel: number | null): string | null {
  if (model.bins !== null) return binReadout(model, model.bins, channel);
  const x = model.xValues ? model.xValues[channel] : channel;
  const y = model.series[channel];
  if (!Number.isFinite(y) || !Number.isFinite(x)) return null;
  const delta = referenceChannel !== null ? `, ${formatFrameDelta(channel, referenceChannel)}` : "";
  const flux = readoutFlux(y, model.readoutErr?.[channel], model.fluxUnit);
  return model.xValues
    ? `ch ${channel}${delta} · ${formatAxisValue(x, model.xUnit, channelWidthAt(model.xValues, channel))} ${model.xUnit} → ${flux}`
    : `ch ${channel}${delta} → ${flux}`;
}
