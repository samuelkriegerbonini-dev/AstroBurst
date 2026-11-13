import { describe, expect, it } from "vitest";
import {
  NATIVE_FLUX_STATE,
  SPECTRUM_PLOT_PAD,
  bandChannelRange,
  buildSpectrumPlotModel,
  documentPlot,
  percentile,
  plottedCount,
  rangeBand,
  spectrumPlotMapping,
  spectrumPlotParts,
  spectrumPlottedMapping,
  spectrumReadout,
  spectrumYScale,
  type SpectrumAxisInput,
  type SpectrumPlotInput,
  type SpectrumPlotView,
} from "../spectrumPlotModel";
import { axisValueToPixel, channelWidthAt } from "../spectrumRange";
import { logTicks } from "../plotInteraction";
import { formatLogTickLabel } from "../plotScale";
import { airToVacuumUm, formatAxis, formatAxisValue, type FormattedAxis } from "../spectralAxis";
import { binAxis, binSpectrum, binningNote } from "../spectrumBinning";
import { documentAxis } from "../spectrumDocument";
import { fluxDisplay } from "../spectrumUnits";
import type { RadialVelocityCorrectionResult } from "../../shared/types/spectral";
import type { Spectrum1d } from "../../shared/types/spectrum1d";
import { DEFAULT_SPECTRUM_VIEW, type SpectrumViewState } from "../../hooks/useSpectrumViewStore";

const XS = Array.from({ length: 40 }, (_, i) => 0.6555 + i * 1e-4);

function formattedAxis(values: number[], label = "Vacuum wavelength (μm)", unit = "μm"): FormattedAxis {
  return { values, label, unit, notes: [] };
}

function plotInput(series: number[], extra: Partial<SpectrumPlotInput> = {}, axis: Partial<SpectrumAxisInput> = {}): SpectrumPlotInput {
  return {
    series,
    dqExcluded: null,
    axis: {
      formatted: null,
      mode: "wavelength_vac",
      correction: "none",
      correctionResult: null,
      fallbackWavelengths: null,
      fallbackUnit: null,
      ...axis,
    },
    errors: null,
    readoutErr: null,
    fluxUnit: "native",
    color: "#a855f7",
    yLabel: null,
    errorsWidenRange: true,
    ...extra,
  };
}

const BARYCENTRIC: RadialVelocityCorrectionResult = {
  barycentric_kms: 12.5,
  heliocentric_kms: 11,
  jd_mid: 2460000.5,
  method: "test",
  accuracy_kms: 0.01,
  notes: [],
  ra_deg: 10,
  dec_deg: 20,
  coordinate_source: "test",
};

describe("buildSpectrumPlotModel axis", () => {
  it("takes values, label and unit from the formatted axis", () => {
    const values = [0.6555, 0.6556, 0.6557];
    const model = buildSpectrumPlotModel(plotInput([1, 2, 1], {}, { formatted: formattedAxis(values) }));
    expect(model.xValues).toEqual(values);
    expect(model.xLabel).toBe("Vacuum wavelength (μm)");
    expect(model.xUnit).toBe("μm");
    expect(model.xScale).toBe(1);
    expect(model.n).toBe(3);
    expect(model.xMin).toBe(0.6555);
    expect(model.xMax).toBe(0.6557);
  });

  it("adds the velocity correction and names it", () => {
    const model = buildSpectrumPlotModel(
      plotInput([1, 2, 1], {}, {
        mode: "velocity",
        formatted: formattedAxis([-100, 0, 100], "Velocity, optical (km/s)", "km/s"),
        correction: "barycentric",
        correctionResult: BARYCENTRIC,
      }),
    );
    expect(model.xValues).toEqual([-87.5, 12.5, 112.5]);
    expect(model.xLabel).toBe("Velocity, optical (km/s), barycentric");
    expect(model.xUnit).toBe("km/s");
  });

  it("falls back to the cube wavelengths in their own unit", () => {
    const model = buildSpectrumPlotModel(plotInput([1, 2], {}, { fallbackWavelengths: [5000, 5001], fallbackUnit: "ANGSTROM" }));
    expect(model.xValues).toHaveLength(2);
    expect(model.xValues![0]).toBeCloseTo(500, 9);
    expect(model.xValues![1]).toBeCloseTo(500.1, 9);
    expect(model.xLabel).toBe("Wavelength (nm)");
    expect(model.xUnit).toBe("nm");
  });

  it("uses the channel index when no axis matches the series", () => {
    const model = buildSpectrumPlotModel(plotInput([1, 2, 3, 4], {}, { formatted: formattedAxis([0.6555, 0.6556, 0.6557]) }));
    expect(model.xValues).toBeNull();
    expect(model.xLabel).toBe("Channel");
    expect(model.xUnit).toBe("ch");
    expect(model.xMin).toBe(0);
    expect(model.xMax).toBe(3);
  });
});

describe("buildSpectrumPlotModel limits", () => {
  it("pads the y range by five percent of the span", () => {
    const rising = buildSpectrumPlotModel(plotInput([1, 3]));
    expect(rising.yMin).toBeCloseTo(0.9, 12);
    expect(rising.yMax).toBeCloseTo(3.1, 12);
    const flat = buildSpectrumPlotModel(plotInput([2, 2, 2]));
    expect(flat.yMin).toBeCloseTo(1.9, 12);
    expect(flat.yMax).toBeCloseTo(2.1, 12);
    const empty = buildSpectrumPlotModel(plotInput([]));
    expect({ xMin: empty.xMin, xMax: empty.xMax, yMin: empty.yMin, yMax: empty.yMax }).toEqual({ xMin: 0, xMax: 1, yMin: 0, yMax: 1 });
    const blank = buildSpectrumPlotModel(plotInput([NaN, NaN, NaN]));
    expect(blank.yMin).toBeCloseTo(-0.05, 12);
    expect(blank.yMax).toBeCloseTo(1.05, 12);
  });

  it("widens the y range to hold the error band and its scaled outline", () => {
    const series = [0.1, 0.2, 0.1, 0.2];
    const err = series.map(() => 0.5);
    const propagated = buildSpectrumPlotModel(plotInput(series, { errors: { err, scaled: null } }));
    expect(propagated.yMin).toBeCloseTo(-0.455, 12);
    expect(propagated.yMax).toBeCloseTo(0.755, 12);
    const scaled = buildSpectrumPlotModel(plotInput(series, { errors: { err, scaled: series.map(() => 0.65) } }));
    expect(scaled.yMin).toBeCloseTo(-0.62, 12);
    expect(scaled.yMax).toBeCloseTo(0.92, 12);
    expect(scaled.errorBand).toBe("propagated+scaled");
    expect(propagated.errorBand).toBe("propagated");
  });

  it("keeps the y range on the flux when a file's errors are not allowed to widen it", () => {
    const series = [1, 2, 1, 2];
    const errors = { err: [0.1, 1e6, 0.1, 0.1], scaled: null };
    const kept = buildSpectrumPlotModel(plotInput(series, { errors, errorsWidenRange: false }));
    expect(kept.yMin).toBeCloseTo(0.95, 12);
    expect(kept.yMax).toBeCloseTo(2.05, 12);
    expect(kept.errorBand).toBe("propagated");
    expect(kept.errors).toBe(errors);
    const widened = buildSpectrumPlotModel(plotInput(series, { errors, errorsWidenRange: true }));
    expect(widened.yMax).toBeGreaterThan(1e6);
  });

  it("carries the y title", () => {
    expect(buildSpectrumPlotModel(plotInput([1, 2], { yLabel: "Flux (Jy)" })).yLabel).toBe("Flux (Jy)");
    expect(buildSpectrumPlotModel(plotInput([1, 2], { yLabel: null })).yLabel).toBeNull();
  });

  it("keeps the range of a faint F_lambda spectrum", () => {
    const faint = XS.map((_, i) => 1e-17 * (1 + Math.exp(-((i - 20) ** 2) / 8)));
    const model = buildSpectrumPlotModel(plotInput(faint, {}, { formatted: formattedAxis(XS) }));
    expect(Math.abs(model.yMin - 9.5e-18)).toBeLessThan(1e-22);
    expect(Math.abs(model.yMax - 2.05e-17)).toBeLessThan(1e-22);
  });

  it("leaves DQ-excluded samples out of the series and of the limits", () => {
    const series = XS.map((_, i) => (i >= 10 && i <= 14 ? 0 : 5));
    const dqExcluded = XS.map((_, i) => i >= 10 && i <= 14);
    const masked = buildSpectrumPlotModel(plotInput(series, { dqExcluded }, { formatted: formattedAxis(XS) }));
    expect(masked.series[12]).toBeNaN();
    expect(masked.yMin).toBeGreaterThan(4.7);
    expect(masked.dqExcluded).toBe(dqExcluded);
    const unmasked = buildSpectrumPlotModel(plotInput(series, {}, { formatted: formattedAxis(XS) }));
    expect(unmasked.series[12]).toBe(0);
    expect(unmasked.yMin).toBeLessThan(0);
  });
});

describe("spectrumPlotMapping and spectrumYScale", () => {
  it("maps channels to pixels with the plot pads", () => {
    const model = buildSpectrumPlotModel(plotInput(XS.map(() => 1), {}, { formatted: formattedAxis(XS) }));
    expect(spectrumPlotMapping(model, 400)).toEqual({
      width: 400,
      padLeft: SPECTRUM_PLOT_PAD.left,
      padRight: 12,
      xMin: model.xMin,
      xMax: model.xMax,
      xValues: model.xValues,
      n: model.n,
    });
    const single = buildSpectrumPlotModel(plotInput([3], {}, { formatted: formattedAxis([0.6555]) }));
    expect(single.xMin).toBe(single.xMax);
    const px = axisValueToPixel(0.6555, spectrumPlotMapping(single, 400));
    expect(Number.isFinite(px)).toBe(true);
    expect(px).toBe(SPECTRUM_PLOT_PAD.left);
  });

  it("leaves room for the y title", () => {
    expect(SPECTRUM_PLOT_PAD.left).toBe(62);
    const model = buildSpectrumPlotModel(plotInput(XS.map(() => 1), {}, { formatted: formattedAxis(XS) }));
    expect(spectrumPlotMapping(model, 400).padLeft).toBe(62);
  });

  it("scales y inside the plot", () => {
    const model = { ...buildSpectrumPlotModel(plotInput([0, 1])), yMin: 0, yMax: 1 };
    const scale = spectrumYScale(model, 180);
    expect(scale.plotTop).toBe(10);
    expect(scale.plotBottom).toBe(156);
    expect(scale.plotHeight).toBe(146);
    expect(scale.toY(0)).toBe(156);
    expect(scale.toY(1)).toBe(10);
    expect(scale.toY(0.5)).toBe(83);
    const flat = spectrumYScale({ ...model, yMin: 2, yMax: 2 }, 180);
    expect(flat.toY(2)).toBe(156);
  });
});

describe("spectrumReadout", () => {
  it("prints the readout with error and unit", () => {
    const faint = XS.map(() => 1e-17);
    faint[20] = 2e-17;
    const readoutErr = XS.map(() => 5e-19);
    const onAxis = (series: number[], extra: Partial<SpectrumPlotInput>) =>
      buildSpectrumPlotModel(plotInput(series, extra, { formatted: formattedAxis(XS) }));
    expect(spectrumReadout(onAxis(faint, { readoutErr, fluxUnit: "erg/s/cm2/A" }), 20, null)).toBe(
      "ch 20 · 0.6575 μm → 2.000e-17 ± 5.000e-19 erg/s/cm2/A",
    );
    expect(spectrumReadout(onAxis(faint, { fluxUnit: "native" }), 20, null)).toBe("ch 20 · 0.6575 μm → 2.000e-17 native");
    const ones = XS.map(() => 1);
    expect(spectrumReadout(onAxis(ones, { fluxUnit: "DN" }), 20, 0)).toBe("ch 20, +20 from displayed · 0.6575 μm → 1.000 DN");
    expect(spectrumReadout(buildSpectrumPlotModel(plotInput([1, 1, 1, 1, 1], { fluxUnit: "DN" })), 3, null)).toBe("ch 3 → 1.000 DN");
    const gap = ones.slice();
    gap[20] = NaN;
    expect(spectrumReadout(onAxis(gap, { fluxUnit: "DN" }), 20, null)).toBeNull();
  });
});

function airDocument(extra: Partial<Spectrum1d> = {}): Spectrum1d {
  return {
    source: { kind: "wcs1d", path: "C:/data/ha_bess.fits", hdu: 0 },
    axis: {
      kind: "awav",
      ctype: "WAVE",
      unit: "um",
      header_unit: "Angstrom",
      header_scale: 1e-4,
      values: XS,
      crval: XS[0],
      cdelt: 1e-4,
      crpix: 1,
      rest_wavelength_um: null,
      rest_frequency_hz: null,
      specsys: null,
      velosys: null,
      notes: [],
    },
    wavelength_frame: "air",
    precision: "f64",
    flux: XS.map((_, i) => 1 + Math.exp(-((i - 20) ** 2) / 8)),
    err: XS.map(() => 0.05),
    dq: null,
    dq_table: "unknown",
    flux_unit: "erg/s/cm2/A",
    n_rows: XS.length,
    cards: {},
    notes: ["spectrum file, 1D array in HDU 0: air wavelengths"],
    ...extra,
  };
}

function viewOf(extra: Partial<SpectrumViewState>): SpectrumViewState {
  return { ...DEFAULT_SPECTRUM_VIEW, ...extra };
}

describe("documentPlot", () => {
  it("builds the plot of a document from the document and the view alone", () => {
    const doc = airDocument();
    const plot = documentPlot(doc, DEFAULT_SPECTRUM_VIEW);
    expect(plot.frameChoice).toBe("air");
    expect(plot.mode).toBe("wavelength_air");
    expect(plot.axis.kind).toBe("awav");
    expect(plot.model.xLabel).toBe("Air wavelength (μm)");
    expect(plot.model.xValues![0] === 0.6555).toBe(true);
    const asVacuum = documentPlot(doc, viewOf({ mode: "wavelength_vac" }));
    expect(asVacuum.mode).toBe("wavelength_vac");
    expect(asVacuum.model.xLabel).toBe("Vacuum wavelength (μm)");
    expect(asVacuum.model.xValues![0]).toBe(airToVacuumUm(0.6555));
    expect(asVacuum.marks.map((m) => m.id)).toContain("h_alpha");
    expect(plot.model.yLabel).toBe("Flux (erg/s/cm2/A)");
    expect(plot.model.fluxUnit).toBe("erg/s/cm2/A");
    expect(plot.model.errors!.err).toBe(doc.err);
    expect(plot.model.readoutErr).toBe(doc.err);
    expect(plot.model.color).toBe("#a855f7");
    expect(plot.model.yMax).toBeLessThan(2.1);
    expect(plot.availability.ok).toBe(true);
    expect(plot.marks.map((m) => m.id)).toContain("h_alpha");
    expect(plot.pickedMarkId).toBeNull();
  });

  it("follows the view", () => {
    const doc = airDocument();
    const vacuum = documentPlot(doc, viewOf({ frameChoice: "vacuum" }));
    expect(vacuum.frameChoice).toBe("vacuum");
    expect(vacuum.axis.kind).toBe("wave");
    expect(vacuum.model.xValues![0] === 0.6555).toBe(true);

    const noRest = documentPlot(doc, viewOf({ mode: "velocity", restUm: null }));
    expect(noRest.formatted).toBeNull();
    expect(noRest.model.xLabel).toBe("Channel");
    expect(noRest.availability.ok).toBe(false);
    expect(noRest.marks).toEqual([]);

    const velocity = documentPlot(doc, viewOf({ mode: "velocity", restUm: 0.656461 }));
    expect(velocity.mode).toBe("velocity");
    expect(velocity.model.xLabel.startsWith("Velocity, optical")).toBe(true);

    expect(documentPlot(doc, viewOf({ lineListVisible: false })).marks).toEqual([]);

    const flagged = airDocument({ dq: XS.map((_, i) => (i === 3 ? 1 : 0)), dq_table: "jwst" });
    expect(documentPlot(flagged, DEFAULT_SPECTRUM_VIEW).model.dqExcluded![3]).toBe(true);
    expect(documentPlot(flagged, viewOf({ useDq: false })).model.dqExcluded).toBeNull();

    expect(documentPlot(doc, viewOf({ restUm: 0.656461, pickedLineId: "h_alpha" })).pickedMarkId).toBe("h_alpha");
    expect(documentPlot(doc, viewOf({ restUm: 0.65, pickedLineId: "h_alpha" })).pickedMarkId).toBeNull();
  });
});

function plotView(extra: Partial<SpectrumPlotView>): SpectrumPlotView {
  return { xDomain: null, logY: false, yLimits: "full", ...extra };
}

function expectRelative(actual: number, expected: number, tolerance: number) {
  expect(Math.abs(actual - expected) / Math.abs(expected)).toBeLessThan(tolerance);
}

describe("spectrum plot view", () => {
  it("interpolates percentiles the way numpy does", () => {
    expect(percentile([1, 2, 3, 10], 1)).toBeCloseTo(1.03, 12);
    expect(percentile([1, 2, 3, 10], 99)).toBeCloseTo(9.79, 12);
    const tail = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 100];
    expect(Math.abs(percentile(tail, 1) - 0.1)).toBeLessThan(1e-12);
    expect(Math.abs(percentile(tail, 99) - 90.9)).toBeLessThan(1e-12);
    expect(percentile([5], 50)).toBe(5);
    expect(percentile([], 50)).toBeNaN();
  });

  it("zooms x to the domain and takes y from the visible samples", () => {
    const series = XS.map((_, i) => i);
    const onAxis = (view: SpectrumPlotView) => buildSpectrumPlotModel(plotInput(series, { view }, { formatted: formattedAxis(XS) }));
    const zoomed = onAxis(plotView({ xDomain: [0.65645, 0.65755] }));
    expect(zoomed.xMin).toBe(0.65645);
    expect(zoomed.xMax).toBe(0.65755);
    expect(Math.abs(zoomed.xExtent[0] - 0.6555)).toBeLessThan(1e-12);
    expect(Math.abs(zoomed.xExtent[1] - 0.6594)).toBeLessThan(1e-12);
    expect(zoomed.yMin).toBeCloseTo(9.5, 12);
    expect(zoomed.yMax).toBeCloseTo(20.5, 12);
    const wide = onAxis(plotView({ xDomain: [0.6, 0.7] }));
    expect([wide.xMin, wide.xMax]).toEqual([XS[0], XS[39]]);
    expect(wide.xExtent).toEqual([XS[0], XS[39]]);
    const flat = onAxis(plotView({ xDomain: [0.656, 0.656] }));
    expect([flat.xMin, flat.xMax]).toEqual([XS[0], XS[39]]);
    const plain = buildSpectrumPlotModel(plotInput([1, 2, 3, 4]));
    expect(plain.xExtent).toEqual([0, 3]);
    expect([plain.xMin, plain.xMax]).toEqual([0, 3]);
  });

  it("has the fallback y range when nothing is visible", () => {
    const series = XS.map((_, i) => (i >= 10 && i <= 20 ? NaN : i));
    const model = buildSpectrumPlotModel(plotInput(series, { view: plotView({ xDomain: [0.65645, 0.65755] }) }, { formatted: formattedAxis(XS) }));
    expect(model.yMin).toBeCloseTo(-0.05, 12);
    expect(model.yMax).toBeCloseTo(1.05, 12);
    expect(model.offScale).toBe(0);
  });

  it("sets robust limits from the 1st and 99th percentile and counts what falls outside", () => {
    const y = Array.from({ length: 2000 }, (_, i) => 1e-3 + 5e-5 * Math.sin(0.37 * i));
    y[100] = -42.82;
    y[1500] = 0.5;
    const robust = buildSpectrumPlotModel(plotInput(y, { view: plotView({ yLimits: "robust" }) }));
    expectRelative(robust.yMin, 0.0009450313600466141, 1e-9);
    expectRelative(robust.yMax, 0.0010549719170206313, 1e-9);
    expect(robust.offScale).toBe(2);
    expect(robust.farOutliers).toBe(2);
    const full = buildSpectrumPlotModel(plotInput(y, { view: plotView({ yLimits: "full" }) }));
    expectRelative(full.yMin, -44.986, 1e-9);
    expectRelative(full.yMax, 2.666, 1e-9);
    expect(full.offScale).toBe(0);
    expect(full.farOutliers).toBe(2);
    expect(full.logY).toBe(false);
    expect(full.logHidden).toBe(0);
  });

  it("keeps the error band out of robust limits", () => {
    const series = [1, 2, 1, 2];
    const errors = { err: series.map(() => 0.5), scaled: null };
    const full = buildSpectrumPlotModel(plotInput(series, { errors, errorsWidenRange: true, view: plotView({ yLimits: "full" }) }));
    expect(full.yMin).toBeCloseTo(0.4, 12);
    expect(full.yMax).toBeCloseTo(2.6, 12);
    const robust = buildSpectrumPlotModel(plotInput(series, { errors, errorsWidenRange: true, view: plotView({ yLimits: "robust" }) }));
    expect(robust.yMin).toBeCloseTo(0.95, 12);
    expect(robust.yMax).toBeCloseTo(2.05, 12);
  });

  it("uses a log axis only when a visible sample is above zero", () => {
    const logModel = buildSpectrumPlotModel(plotInput([-1, 0, 0.01, 0.1, 1, 10], { view: plotView({ logY: true }) }));
    expect(logModel.logY).toBe(true);
    expect(logModel.logHidden).toBe(2);
    expect(logModel.offScale).toBe(0);
    expectRelative(logModel.yMin, 0.00707945784384138, 1e-12);
    expectRelative(logModel.yMax, 14.12537544622754, 1e-12);
    const negative = buildSpectrumPlotModel(plotInput([-1, 0], { view: plotView({ logY: true }) }));
    expect(negative.logY).toBe(false);
    expect(negative.logHidden).toBe(0);
    expect(negative.yMin).toBeCloseTo(-1.05, 12);
    expect(negative.yMax).toBeCloseTo(0.05, 12);
    const flat = buildSpectrumPlotModel(plotInput([5, 5], { view: plotView({ logY: true }) }));
    expect(flat.logY).toBe(true);
    expectRelative(flat.yMin, 5 / Math.sqrt(10), 1e-12);
    expectRelative(flat.yMax, 5 * Math.sqrt(10), 1e-12);
    expect(buildSpectrumPlotModel(plotInput([-1, 0, 1])).logY).toBe(false);
  });

  it("scales and ticks a log axis", () => {
    const model = buildSpectrumPlotModel(plotInput([-1, 0, 0.01, 0.1, 1, 10], { view: plotView({ logY: true }) }));
    const scale = spectrumYScale(model, 180);
    expect(scale.toY(model.yMax)).toBeCloseTo(10, 9);
    expect(scale.toY(model.yMin)).toBeCloseTo(156, 9);
    const expected = 10 + (146 * (Math.log10(model.yMax) - Math.log10(0.1))) / (Math.log10(model.yMax) - Math.log10(model.yMin));
    expect(scale.toY(0.1)).toBeCloseTo(expected, 9);
    expect(scale.toY(0)).toBeGreaterThan(156);
    expect(scale.toY(-1)).toBeGreaterThan(156);
    const values = logTicks(model.yMin, model.yMax);
    expect(scale.ticks.map((t) => t.value)).toEqual(values);
    expect(scale.ticks.map((t) => t.label)).toEqual(values.map(formatLogTickLabel));
    expect(scale.ticks.map((t) => t.label)).toEqual(expect.arrayContaining(["0.01", "0.1", "1", "10"]));

    const linear = spectrumYScale({ ...buildSpectrumPlotModel(plotInput([0, 1])), yMin: 0, yMax: 1 }, 180);
    expect(linear.ticks.map((t) => t.value)).toEqual([1, 0.75, 0.5, 0.25, 0]);
    expect(linear.ticks.map((t) => t.label)).toEqual(["1.00", "0.75", "0.50", "0.25", "0"]);
  });

  it("passes the view of a document into its plot", () => {
    const doc = airDocument();
    const plain = documentPlot(doc, DEFAULT_SPECTRUM_VIEW).model;
    const [a, b] = [plain.xValues![10], plain.xValues![30]];
    const plot = documentPlot(doc, viewOf({ xDomain: [a, b], logY: true, yLimits: "robust" }));
    expect(plot.model.xMin).toBe(a);
    expect(plot.model.xMax).toBe(b);
    expect(plot.model.logY).toBe(true);
    const sorted = doc.flux.slice(10, 31).sort((p, q) => p - q);
    const lo = Math.log10(percentile(sorted, 1));
    const hi = Math.log10(percentile(sorted, 99));
    expectRelative(plot.model.yMin, 10 ** (lo - 0.05 * (hi - lo)), 1e-12);
    expectRelative(plot.model.yMax, 10 ** (hi + 0.05 * (hi - lo)), 1e-12);

    const cube = spectrumPlotParts({
      axis: null,
      mode: "wavelength_vac",
      view: viewOf({ xDomain: [10, 30], logY: true, yLimits: "robust" }),
      correction: "none",
      correctionResult: null,
      shiftKms: null,
      series: doc.flux,
      dqExcluded: null,
      errors: null,
      errorsWidenRange: true,
      readoutErr: null,
      fluxUnit: "DN",
      color: "#a855f7",
      fallbackWavelengths: null,
      fallbackUnit: null,
    }).model;
    expect([cube.xMin, cube.xMax]).toEqual([10, 30]);
    expect(cube.logY).toBe(true);
    expectRelative(cube.yMin, plot.model.yMin, 1e-12);
    expectRelative(cube.yMax, plot.model.yMax, 1e-12);
  });
});

function expectAllRelative(actual: readonly number[] | null, expected: readonly number[], tolerance: number) {
  expect(actual).toHaveLength(expected.length);
  expected.forEach((value, i) => expectRelative(actual![i], value, tolerance));
}

function velocityDocument(): Spectrum1d {
  const base = airDocument();
  return airDocument({
    axis: { ...base.axis, kind: "vopt", ctype: "VOPT", unit: "km/s", header_unit: "km/s", header_scale: 1, values: XS.map((_, i) => -200 + 10 * i) },
    wavelength_frame: "not_wavelength",
    flux_unit: "Jy",
  });
}

describe("spectrum display units", () => {
  it("shows a wavelength axis in the unit asked for", () => {
    const values = [0.6555, 0.6556, 0.6557];
    const onAxis = (unit: SpectrumAxisInput["wavelengthUnit"], mode: SpectrumAxisInput["mode"] = "wavelength_vac", label = "Vacuum wavelength (μm)") =>
      buildSpectrumPlotModel(plotInput([1, 2, 1], {}, { formatted: formattedAxis(values, label), mode, wavelengthUnit: unit }));
    const nm = onAxis("nm");
    expectAllRelative(nm.xValues, [655.5, 655.6, 655.7], 1e-12);
    expect(nm.xLabel).toBe("Vacuum wavelength (nm)");
    expect(nm.xUnit).toBe("nm");
    expect(nm.xScale).toBe(1000);
    expect(nm.wavelengthUnit).toBe("nm");
    expectRelative(nm.xMin, 655.5, 1e-12);
    expectRelative(nm.xMax, 655.7, 1e-12);
    const angstrom = onAxis("A");
    expect(angstrom.xLabel).toBe("Vacuum wavelength (Å)");
    expect(angstrom.xScale).toBe(10000);
    expectRelative(angstrom.xValues![0], 6555, 1e-12);
    expect(onAxis("A", "wavelength_air", "Air wavelength (μm)").xLabel).toBe("Air wavelength (Å)");
    for (const unit of ["um", undefined] as const) {
      const micron = onAxis(unit);
      expect(micron.xValues).toEqual(values);
      expect(micron.xLabel).toBe("Vacuum wavelength (μm)");
      expect(micron.xScale).toBe(1);
      expect(micron.wavelengthUnit).toBe("um");
    }
    const single = buildSpectrumPlotModel(plotInput([3], {}, { formatted: formattedAxis([0.6555]), wavelengthUnit: "nm" }));
    expectAllRelative(single.xValues, [655.5], 1e-12);
    expect(single.xMin).toBe(single.xMax);
  });

  it("keeps every other axis in its own unit", () => {
    const cases: Partial<SpectrumAxisInput>[] = [
      { mode: "frequency", formatted: formattedAxis([457.3, 457.2, 457.1], "Frequency (GHz)", "GHz") },
      { mode: "velocity", formatted: formattedAxis([-100, 0, 100], "Velocity, optical (km/s)", "km/s") },
      { mode: "wavelength_vac", formatted: formattedAxis([-100, 0, 100], "Velocity (km/s)", "km/s") },
      { fallbackWavelengths: [5000, 5001, 5002], fallbackUnit: "ANGSTROM" },
      {},
    ];
    for (const axis of cases) {
      const plain = buildSpectrumPlotModel(plotInput([1, 2, 1], {}, axis));
      const asked = buildSpectrumPlotModel(plotInput([1, 2, 1], {}, { ...axis, wavelengthUnit: "nm" }));
      expect(asked.xScale).toBe(1);
      expect(asked.wavelengthUnit).toBeNull();
      expect(plain.wavelengthUnit).toBeNull();
      expect(asked.xValues).toEqual(plain.xValues);
      expect(asked.xLabel).toBe(plain.xLabel);
      expect(asked.xUnit).toBe(plain.xUnit);
    }
  });

  it("places rest-line marks on the scaled axis", () => {
    const plot = documentPlot(airDocument(), viewOf({ mode: "wavelength_vac", wavelengthUnit: "nm" }));
    expect(plot.model.xScale).toBe(1000);
    expectRelative(plot.model.xValues![0], airToVacuumUm(0.6555) * 1000, 1e-12);
    const halpha = plot.marks.find((m) => m.id === "h_alpha");
    expect(halpha?.value).toBeCloseTo(0.656461, 12);
    const drawn = halpha!.value * plot.model.xScale;
    expect(drawn).toBeGreaterThanOrEqual(plot.model.xMin);
    expect(drawn).toBeLessThanOrEqual(plot.model.xMax);
  });

  it("shows the flux of a document in the unit asked for", () => {
    const doc = airDocument();
    const vac = formatAxis(documentAxis(doc.axis, "air"), "wavelength_vac", null, "optical")!.values;
    const micro = documentPlot(doc, viewOf({ mode: "wavelength_vac", fluxUnit: "uJy" })).model;
    const expected = fluxDisplay({ flux: doc.flux, err: doc.err, vacuumUm: vac, fluxUnit: "erg/s/cm2/A" }, "uJy");
    expect(micro.series).toEqual(expected.values);
    expect(micro.errors).toEqual({ err: expected.errors, scaled: null });
    expect(micro.readoutErr).toEqual(expected.errors);
    expect(micro.fluxUnit).toBe("uJy");
    expect(micro.yLabel).toBe("Flux (uJy)");
    expect(micro.fluxState).toEqual({ choice: "uJy", scale: null, invertY: false, note: null });
    expectRelative(micro.series[20], ((2 * vac[20] ** 2) / 2.99792458e-13) * 1e6, 1e-12);
    const si = documentPlot(doc, viewOf({ mode: "wavelength_vac", fluxUnit: "flam_si" })).model;
    expectRelative(si.series[20], 20, 1e-12);
    expect(si.fluxState.scale).toBe(10);
    expect(si.yLabel).toBe("Flux (W m-2 um-1)");
  });

  it("inverts the y axis for AB magnitudes and uses no log axis", () => {
    const doc = airDocument({ flux_unit: "Jy" });
    const ab = documentPlot(doc, viewOf({ mode: "wavelength_vac", fluxUnit: "ab", logY: true })).model;
    expect(ab.fluxState.invertY).toBe(true);
    expect(ab.yLabel).toBe("AB magnitude");
    expect(ab.logY).toBe(false);
    expect(ab.yMin).toBeLessThan(ab.yMax);
    const scale = spectrumYScale(ab, 180);
    expect(scale.toY(ab.yMin)).toBe(10);
    expect(scale.toY(ab.yMax)).toBe(156);
    const native = documentPlot(doc, viewOf({ mode: "wavelength_vac", fluxUnit: "native" })).model;
    expect(spectrumYScale(native, 180).toY(native.yMax)).toBe(10);

    const base = airDocument();
    const dark = airDocument({ axis: { ...base.axis, values: XS.slice(0, 2) }, flux: [0, -1], err: [0.05, 0.05], n_rows: 2, flux_unit: "Jy" });
    const blank = documentPlot(dark, viewOf({ mode: "wavelength_vac", fluxUnit: "ab" })).model;
    expect(blank.fluxState.invertY).toBe(true);
    expect(blank.series.every((v) => !Number.isFinite(v))).toBe(true);
    expect(blank.yMin).toBeCloseTo(-0.05, 12);
    expect(blank.yMax).toBeCloseTo(1.05, 12);
  });

  it("keeps the file unit and says why when the unit asked for cannot be shown", () => {
    const counts = airDocument({ flux_unit: "DN/s" });
    const kept = documentPlot(counts, viewOf({ mode: "wavelength_vac", fluxUnit: "Jy" })).model;
    expect(kept.series).toEqual(counts.flux);
    expect(kept.fluxUnit).toBe("DN/s");
    expect(kept.fluxState).toEqual({ choice: "native", scale: 1, invertY: false, note: "Jy is not available for flux in DN/s" });

    const velocity = velocityDocument();
    const cgs = documentPlot(velocity, viewOf({ mode: "wavelength_vac", fluxUnit: "flam_cgs" })).model;
    expect(cgs.fluxState.note).toBe("erg s-1 cm-2 Angstrom-1 is not available for flux in Jy without a wavelength axis");
    expect(cgs.series).toEqual(velocity.flux);
    const milli = documentPlot(velocity, viewOf({ mode: "wavelength_vac", fluxUnit: "mJy" })).model;
    expect(milli.fluxState.scale).toBe(1000);
    expectRelative(milli.series[20], 2000, 1e-12);
  });

  it("never converts a spectrum without a file unit to convert from", () => {
    const series = XS.map((_, i) => 1 + i);
    const model = spectrumPlotParts({
      axis: null,
      mode: "wavelength_vac",
      view: viewOf({ mode: "wavelength_vac", fluxUnit: "mJy" }),
      correction: "none",
      correctionResult: null,
      shiftKms: null,
      series,
      dqExcluded: null,
      errors: null,
      errorsWidenRange: true,
      readoutErr: null,
      fluxUnit: "DN",
      color: "#a855f7",
      fallbackWavelengths: null,
      fallbackUnit: null,
    }).model;
    expect(model.series).toEqual(series);
    expect(model.fluxState).toEqual(NATIVE_FLUX_STATE);
    expect(model.fluxUnit).toBe("DN");
  });
});

const BINNED_NOTE = "binned by 4: 10 bins; mean of the unmasked samples, errors assume independent channels";

function binnedModel(extra: Partial<SpectrumViewState> = {}, doc: Spectrum1d = airDocument()) {
  return documentPlot(doc, viewOf({ mode: "wavelength_vac", binFactor: 4, ...extra })).model;
}

describe("spectrum binning on the plot", () => {
  it("bins a document and keeps its channels", () => {
    const doc = airDocument();
    const model = binnedModel({}, doc);
    const bins = model.bins!;
    expect(bins.factor).toBe(4);
    expectRelative(bins.y[4], 1.4872538282230479, 1e-12);
    expectRelative(bins.y[5], 1.7034200074138945, 1e-12);
    expect(bins.err).toHaveLength(10);
    bins.err!.forEach((e) => expectRelative(e, 0.025, 1e-12));
    expect([bins.first[5], bins.last[5], bins.nUsed[5]]).toEqual([20, 23, 4]);
    const xs = model.xValues!;
    expectRelative(bins.x[0], (xs[0] + xs[1] + xs[2] + xs[3]) / 4, 1e-12);
    expect(bins.note).toBe(BINNED_NOTE);
    expect(model.n).toBe(40);
    expect(model.series).toEqual(doc.flux);
    expect(model.xValues).toHaveLength(40);
  });

  it("takes the y limits from the bins, not from the channels", () => {
    const series = XS.map((_, i) => (i === 7 ? 105 : 5));
    const b = binSpectrum(series, null, null, 8);
    expect(b.flux).toEqual([17.5, 5, 5, 5, 5]);
    const bins = { factor: b.factor, first: b.first, last: b.last, nUsed: b.nUsed, y: b.flux, err: b.err, note: binningNote(b) };
    const binned = buildSpectrumPlotModel(plotInput(series, { bins }));
    expect(binned.xValues).toBeNull();
    expectRelative(binned.yMin, 4.375, 1e-12);
    expectRelative(binned.yMax, 18.125, 1e-12);
    expect(buildSpectrumPlotModel(plotInput(series)).yMax).toBeGreaterThan(100);
  });

  it("leaves masked samples out of the bins", () => {
    const flagged = airDocument({ dq: XS.map((_, i) => (i >= 20 && i <= 23 ? 1 : 0)), dq_table: "jwst" });
    const masked = binnedModel({ useDq: true }, flagged).bins!;
    expect(masked.nUsed[5]).toBe(0);
    expect(masked.y[5]).toBeNaN();
    expect(binnedModel({ useDq: false }, flagged).bins!.nUsed[5]).toBe(4);
  });

  it("converts the bins at their own wavelengths", () => {
    const doc = airDocument();
    const vac = formatAxis(documentAxis(doc.axis, "air"), "wavelength_vac", null, "optical")!.values;
    const model = binnedModel({ fluxUnit: "uJy" }, doc);
    const b = binSpectrum(doc.flux, doc.err, null, 4);
    const expected = fluxDisplay({ flux: b.flux, err: b.err, vacuumUm: binAxis(vac, 4), fluxUnit: "erg/s/cm2/A" }, "uJy");
    expect(model.bins!.y).toEqual(expected.values);
    expect(model.bins!.err).toEqual(expected.errors);
    const channels = fluxDisplay({ flux: doc.flux, err: doc.err, vacuumUm: vac, fluxUnit: "erg/s/cm2/A" }, "uJy");
    expect(model.series).toEqual(channels.values);
    expect(model.fluxUnit).toBe("uJy");
  });

  it("maps the plotted samples and the channels apart", () => {
    const model = binnedModel();
    const plotted = spectrumPlottedMapping(model, 400);
    expect(plotted.n).toBe(10);
    expect(plotted.xValues).toBe(model.bins!.x);
    expect(spectrumPlotMapping(model, 400).n).toBe(40);
    expect(plottedCount(model)).toBe(10);
    const plain = documentPlot(airDocument(), viewOf({ mode: "wavelength_vac" })).model;
    expect(spectrumPlottedMapping(plain, 400)).toEqual(spectrumPlotMapping(plain, 400));
    expect(plottedCount(plain)).toBe(plain.n);
  });

  it("reads out a bin with the channels it covers", () => {
    const model = binnedModel();
    const x5 = model.bins!.x[5];
    expect(spectrumReadout(model, 5, null)).toBe(
      `bin 5 (ch 20–23) · ${formatAxisValue(x5, "μm", channelWidthAt(model.bins!.x, 5))} μm → 1.703 ± 0.02500 erg/s/cm2/A`,
    );
    expect(spectrumReadout(model, 5, 0)).toBe(spectrumReadout(model, 5, null));
    const thirds = binnedModel({ binFactor: 3 });
    const last = spectrumReadout(thirds, 13, null)!;
    expect(last.startsWith("bin 13 (ch 39) · ")).toBe(true);
    expect(last.endsWith("→ 1.000 ± 0.05000 erg/s/cm2/A")).toBe(true);
    const flagged = airDocument({ dq: XS.map((_, i) => (i >= 20 && i <= 23 ? 1 : 0)), dq_table: "jwst" });
    expect(spectrumReadout(binnedModel({ useDq: true }, flagged), 5, null)).toBeNull();
    const channelAxis = binnedModel({ mode: "velocity", restUm: null });
    expect(channelAxis.xValues).toBeNull();
    expect(spectrumReadout(channelAxis, 2, null)).toBe("bin 2 (ch 8–11) → 1.000 ± 0.02500 erg/s/cm2/A");
  });

  it("does nothing for a factor of one", () => {
    expect(binnedModel({ binFactor: 1 }).bins).toBeNull();
    expect(binnedModel({ binFactor: 0 }).bins).toBeNull();
    expect(binnedModel({ binFactor: 2.7 }).bins!.factor).toBe(2);
    const whole = binnedModel({ binFactor: 500 }).bins!;
    expect(whole.factor).toBe(40);
    expect(whole.y).toHaveLength(1);
  });
});

const G_AXIS = Array.from({ length: 200 }, (_, i) => 3.6 + i * 0.005);

function gDoc(values: number[] = G_AXIS): Spectrum1d {
  const base = airDocument();
  return airDocument({
    axis: { ...base.axis, kind: "wave", ctype: "WAVE", unit: "um", header_unit: "um", header_scale: 1, values, crval: values[0], cdelt: values[1] - values[0] },
    wavelength_frame: "vacuum",
    flux: values.map(() => 1),
    err: null,
    flux_unit: "Jy",
    n_rows: values.length,
  });
}

function gPlot(extra: Partial<SpectrumViewState>, doc: Spectrum1d = gDoc()) {
  return documentPlot(doc, viewOf({ mode: "wavelength_vac", ...extra }));
}

describe("band strips on the spectrum plot", () => {
  it("places the bands of the chosen species on the axis", () => {
    const co2 = gPlot({ bandSpecies: ["CO2"] }).bands;
    expect(co2.map((b) => b.id)).toEqual(["co2_4p3"]);
    expect(co2[0].lo).toBeCloseTo(4.15, 9);
    expect(co2[0].hi).toBeCloseTo(4.5, 9);
    const withGap = gPlot({ bandSpecies: ["CO2", "G395H gap"] }).bands;
    expect(withGap.map((b) => b.id)).toEqual(["co2_4p3", "nirspec_g395h_gap"]);
    expect(withGap[1].lo).toBeCloseTo(3.72, 9);
    expect(withGap[1].hi).toBeCloseTo(3.82, 9);
    const coPlot = gPlot({ bandSpecies: ["CO"] });
    expect(coPlot.bands.map((b) => b.id)).toEqual(["co_4p7"]);
    expect(coPlot.bands[0].lo).toBeCloseTo(4.3, 9);
    expect(coPlot.bands[0].hi).toBe(coPlot.model.xMax);
    expect(coPlot.bands[0].hi).toBeCloseTo(4.595, 9);
    expect(gPlot({ bandSpecies: ["H2O", "SO2"] }).bands).toEqual([]);
    expect(gPlot({ bandSpecies: [] }).bands).toEqual([]);
  });

  it("shifts molecular bands with the redshift and leaves the detector gap where it is", () => {
    const bands = gPlot({ redshift: 0.01, bandSpecies: ["CO2", "G395H gap"] }).bands;
    expect(bands.map((b) => b.id)).toEqual(["co2_4p3", "nirspec_g395h_gap"]);
    expect(bands[0].lo).toBeCloseTo(4.15 * 1.01, 9);
    expect(bands[0].hi).toBeCloseTo(4.5 * 1.01, 9);
    expect(bands[1].lo).toBeCloseTo(3.72, 9);
    expect(bands[1].hi).toBeCloseTo(3.82, 9);
  });

  it("keeps the bands when the rest lines are hidden, and keeps them in micrometres when the axis is not", () => {
    const hidden = gPlot({ lineListVisible: false, bandSpecies: ["CO2"] });
    expect(hidden.bands).toHaveLength(1);
    expect(hidden.marks).toEqual([]);
    const nm = gPlot({ wavelengthUnit: "nm", bandSpecies: ["CO2"] });
    expect(nm.model.xScale).toBe(1000);
    expect(nm.bands).toHaveLength(1);
    expect(nm.bands[0].lo).toBeCloseTo(4.15, 9);
    const noRest = gPlot({ mode: "velocity", restUm: null, bandSpecies: ["CO2"] });
    expect(noRest.availability.ok).toBe(false);
    expect(noRest.bands).toEqual([]);
  });

  it("turns a band into the channels it covers", () => {
    const model = gPlot({ bandSpecies: ["CO2"] }).model;
    expect(bandChannelRange({ lo: 4.15, hi: 4.5 }, model)).toEqual({ z0: 110, z1: 180 });
    const nm = gPlot({ wavelengthUnit: "nm", bandSpecies: ["CO2"] }).model;
    expect(bandChannelRange({ lo: 4.15, hi: 4.5 }, nm)).toEqual({ z0: 110, z1: 180 });
    const reversed = gPlot({ bandSpecies: ["CO2"] }, gDoc([...G_AXIS].reverse())).model;
    expect(bandChannelRange({ lo: 4.15, hi: 4.5 }, reversed)).toEqual({ z0: 19, z1: 89 });
    const empty = buildSpectrumPlotModel(plotInput([]));
    expect(bandChannelRange({ lo: 4.15, hi: 4.5 }, empty)).toBeNull();
  });

  it("recognises a range that is a band", () => {
    const plot = gPlot({ bandSpecies: ["CO2"] });
    const [co2] = plot.bands;
    expect(rangeBand({ z0: 110, z1: 180 }, plot.bands, plot.model)).toBe(co2);
    expect(rangeBand({ z0: 110, z1: 181 }, plot.bands, plot.model)).toBeNull();
    expect(rangeBand(null, plot.bands, plot.model)).toBeNull();
    const twin = { ...co2, id: "twin" };
    expect(rangeBand({ z0: 110, z1: 180 }, [co2, twin], plot.model)).toBe(co2);
    expect(rangeBand({ z0: 110, z1: 180 }, [twin, co2], plot.model)).toBe(twin);
  });
});
