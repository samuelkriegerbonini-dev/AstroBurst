import { describe, expect, it, vi } from "vitest";
import {
  EMPTY_LOG,
  MAX_LOG_ENTRIES,
  MeasurementLogCore,
  batchPhotometryEntry,
  crossMatchEntry,
  dqHandlingOf,
  entrySummary,
  fileNameOf,
  filterEntries,
  formatLogValue,
  imageLabelOf,
  lineCutEntry,
  lineEntry,
  lineUnitNote,
  measurementLogCsv,
  measurementLogCsvFileName,
  medianOf,
  photometryEntry,
  pixelEntry,
  profileLogReady,
  pvEntry,
  radialProfileEntry,
  regionLogReady,
  regionStatsEntries,
  sbProfileEntry,
  skySeparationEntry,
  spectrumCompareEntry,
  spectrumExportEntry,
  spectrumUnitOf,
  statisticsEntry,
  statisticsLogReady,
  timeSeriesEntry,
  type MeasurementLogDraft,
  type MeasurementLogEntry,
  type MeasurementProvenance,
  type RegionStatsMeasured,
  type StatisticsLogInput,
} from "../measurementLog";
import type { BatchPhotometryResult, PhotCal, PhotometryMeasurement, StarPhotometry } from "../../services/analysis";
import type { PixelTableResult, TimeSeriesFrame, TimeSeriesResult } from "../../shared/types/analysis";
import type { SkySeparationResult } from "../../shared/types/astrometry";
import type { CrossMatchResult } from "../../shared/types/catalog";
import type { ChannelStatisticsBody } from "../../shared/types/statistics";
import type { LineMeasurement, SpectralAxisInfo } from "../../shared/types/spectral";
import type {
  LineCut,
  RadialProfile,
  Region,
  RegionCalibrated,
  RegionShape,
  RegionStats,
  RegionStatsEntry,
  SbProfile,
} from "../../shared/types/regions";
import type { PvDiagramResult, PvRun } from "../../shared/types/pv";
import type { SpectrumExportInput } from "../spectrumExport";
import type { ComparisonCsvInput, ComparisonEntry } from "../spectrumCompare";

const CLOCK = "2026-09-25T12:00:00.000Z";
const PROV: MeasurementProvenance = { file: "x.fits", image: "original" };
const CIRCLE: RegionShape = { shape: "circle", x: 10, y: 20, r: 4 };
const ANNULUS: RegionShape = { shape: "annulus", x: 10, y: 20, r_inner: 8, r_outer: 12 };
const LINE: Extract<RegionShape, { shape: "line" }> = { shape: "line", x1: 1, y1: 2, x2: 4, y2: 6 };

function makeStore(): { store: MeasurementLogCore; listener: ReturnType<typeof vi.fn> } {
  let n = 0;
  const store = new MeasurementLogCore(
    () => new Date(CLOCK),
    () => `id${++n}`,
  );
  const listener = vi.fn();
  store.subscribe(listener);
  return { store, listener };
}

function draft(extra: Partial<MeasurementLogDraft> = {}): MeasurementLogDraft {
  return {
    kind: "photometry",
    file: "x.fits",
    image: "original",
    dq: "off",
    unit: null,
    source: "s",
    params: {},
    values: {},
    notes: [],
    ...extra,
  };
}

function star(extra: Partial<StarPhotometry> = {}): StarPhotometry {
  return {
    x: 10,
    y: 20,
    peak: 500,
    net_flux: 1234.5,
    flux_err: 12.3,
    mag_inst: -7.73,
    snr: 100.4,
    fwhm: 3.2,
    aperture_radius: 4.8,
    aperture_pixels: 72,
    aperture_area: 72.4,
    bg_mean: 10,
    bg_sigma: 1.5,
    bg_pixels: 300,
    saturated: false,
    saturation_source: "none",
    n_masked: 2,
    n_saturated: 0,
    err_used: true,
    aperture_correction: null,
    flux_total: null,
    plateau_radius: null,
    flux_jy: 2.5e-6,
    flux_err_jy: 1e-8,
    mag_ab: 22.9,
    mag_ab_err: 0.01,
    mag_ab_total: null,
    st_mag: null,
    sky_inner: 9.6,
    sky_outer: 14.4,
    growth_curve: [],
    ee50_radius: null,
    ee80_radius: null,
    ...extra,
  };
}

const PHOTCAL: PhotCal = {
  convention: { kind: "jwst_mjy_sr", pixar_sr: 1e-13, pixar_a2: null, derived_pixar: false },
  bunit: "MJy/sr",
  notes: [],
  warnings: [],
  label: "JWST MJy/sr",
};

function photometry(extra: Partial<PhotometryMeasurement> = {}): PhotometryMeasurement {
  return { photometry: star(), sky: { ra: 150.1, dec: 2.2 }, gaia: null, photcal: PHOTCAL, warnings: ["w1"], masked: true, elapsed_ms: 3, ...extra };
}

function region(id: string, overrides: Partial<Region> = {}): Region {
  return {
    id,
    shape: CIRCLE,
    props: { color: null, width: null, text: null, dash: null, include: true },
    backgroundId: null,
    ...overrides,
  };
}

function stats(overrides: Partial<RegionStats> = {}): RegionStats {
  return {
    count: 49,
    n_nan: 0,
    n_excluded: 2,
    n_padding: 0,
    area: 50.27,
    bounds: { x0: 6, y0: 16, x1: 14, y1: 24 },
    clipped: false,
    sum: 196,
    mean: 4,
    median: 4,
    mad: 0,
    sigma: 0,
    std: 0.5,
    min: 3,
    max: 5,
    clipped_mean: 4,
    clipped_median: 4,
    clipped_sigma: 0,
    n_rejected: 0,
    background: null,
    net_sum: null,
    net_snr: null,
    sum_err: null,
    weighted_mean: null,
    calibrated: null,
    ...overrides,
  };
}

function calibrated(overrides: Partial<RegionCalibrated> = {}): RegionCalibrated {
  return {
    flux_source: "sum",
    flux_native: 196,
    flux_err_native: null,
    flux_jy: 3.92e-6,
    flux_err_jy: null,
    mag_ab: 22.417,
    mag_ab_err: null,
    st_mag: null,
    area_arcsec2: 0.126,
    geometric_area_arcsec2: 0.126,
    sb_mag_arcsec2: 20.17,
    ra: 150.5,
    dec: 2.25,
    pa_sky_deg: 12.5,
    ...overrides,
  };
}

function lineMeasurement(extra: Partial<LineMeasurement> = {}): LineMeasurement {
  return {
    z0: 10,
    z1: 30,
    n_channels: 21,
    axis_unit: "um",
    flux_unit: "Jy/beam x um",
    continuum_level: 0.5,
    continuum_slope: 0.01,
    continuum_sigma: 0.02,
    continuum_reference: 1.0,
    continuum_linear: true,
    continuum_channels: 8,
    continuum_windows: [[2, 8], [32, 38]],
    flux: 1.5,
    flux_err: 0.1,
    equivalent_width: -0.002,
    centroid: 1.0012,
    sigma: 0.0005,
    fwhm: 0.0012,
    peak: 3.2,
    peak_channel: 20,
    snr: 25,
    velocity: { centroid_kms: 12.5, sigma_kms: 150, fwhm_kms: 353.2, rest_um: 0.6565, convention: "optical", shift_applied_kms: -7.25, axis_frame: "barycentric" },
    fit: { amplitude: 3.1, centre: 1.0011, sigma: 0.00049, amplitude_err: 0.1, centre_err: 0.00001, sigma_err: 0.00002, chi2: 12.3, dof: 15, iterations: 6, converged: true },
    notes: ["line note"],
    elapsed_ms: 2,
    source: { kind: "pixel", x: 3, y: 4 },
    ...extra,
  };
}

function frame(index: number, targets: (StarPhotometry | null)[], extra: Partial<TimeSeriesFrame> = {}): TimeSeriesFrame {
  return {
    index,
    path: `/run/f${index}.fits`,
    file_name: `f${index}.fits`,
    jd_mid: 2460000.5 + index,
    time_source: "DATE-OBS + EXPTIME/2 (UTC)",
    exptime: 60,
    filter: "V",
    airmass: 1.2,
    geometry: null,
    offset: null,
    photcal_label: null,
    targets,
    errors: targets.map(() => null),
    skipped: null,
    ...extra,
  };
}

function timeSeries(frames: TimeSeriesFrame[]): TimeSeriesResult {
  return {
    reference_path: "/run/f0.fits",
    targets: [
      { x: 10, y: 10, label: "T", role: "target" },
      { x: 40, y: 40, label: "C1", role: "comp" },
      { x: 70, y: 70, label: "C2", role: "comp" },
    ],
    frames,
    n_frames: frames.length,
    n_skipped: frames.filter((f) => f.skipped !== null).length,
    warnings: ["ts warning"],
    geometry_target: null,
    geometry_notes: [],
    elapsed_ms: 5,
  };
}

function analyticSeries(): TimeSeriesResult {
  const comps = () => [star({ net_flux: 50, flux_err: 1 }), star({ net_flux: 50, flux_err: 1 })];
  return timeSeries([
    frame(0, [star({ net_flux: 100, flux_err: 1 }), ...comps()]),
    frame(1, [star({ net_flux: 100, flux_err: 1 }), ...comps()]),
    frame(2, [star({ net_flux: 110, flux_err: 1 }), ...comps()]),
  ]);
}

function pixelResult(extra: Partial<PixelTableResult> = {}): PixelTableResult {
  return {
    x: 12,
    y: 34,
    size: 3,
    x0: 11,
    y0: 33,
    values: [
      [1, 2, 3],
      [4, 5.5, 6],
      [7, 8, 9],
    ],
    err: [
      [0.1, 0.1, 0.1],
      [0.1, 0.25, 0.1],
      [0.1, 0.1, 0.1],
    ],
    dq: [
      [0, 0, 0],
      [0, 4, 0],
      [0, 0, 0],
    ],
    dq_names: [
      [null, null, null],
      [null, "SATURATED", null],
      [null, null, null],
    ],
    dq_table: "JWST",
    unit: "MJy/sr",
    stats: { min: 1, max: 9, mean: 5.06, median: 5.5, n_finite: 9, n_nan: 0 },
    err_stats: null,
    elapsed_ms: 1,
    ...extra,
  };
}

function channelBody(mean: number): ChannelStatisticsBody {
  return {
    statistics: {
      count: 100,
      total: 100,
      fraction: 1,
      mean,
      median: mean,
      avg_dev: 0.1,
      mad: 0.1,
      bwmv_sqrt: 0.12,
      min: 0,
      max: 10,
      sum: mean * 100,
      variance: 0.04,
      std_dev: 0.2,
      nan_count: 0,
      padding: 0,
      excluded: 3,
    },
    noise: { sigma: 0.05, fraction: 0.9, iterations: 3, method: "k-sigma MRS" },
    noise_note: null,
    data_min: 0,
    data_max: 10,
  };
}

function statisticsInput(extra: Partial<StatisticsLogInput> = {}): StatisticsLogInput {
  return {
    channels: [{ label: "K", body: channelBody(2.5) }],
    unit: "ADU",
    masked: true,
    dqExcluded: 17,
    elapsedMs: 4,
    regionKind: null,
    composite: false,
    noise: true,
    excludeDq: true,
    region: null,
    ...extra,
  };
}

function crossMatch(): CrossMatchResult {
  return {
    matches: [],
    sources: [],
    astrometry: { median_d_ra_arcsec: 0.05, median_d_dec_arcsec: -0.02, rms_arcsec: 0.12, n: 40 },
    zero_point: { zp: 25.31, zp_err: 0.02, colour_coeff: null, rms: 0.05, n_used: 38, n_rejected: 2, n_without_colour: 0, band: "G", colour_term_used: false },
    photcal_present: false,
    epoch_year: 2024.3,
    n_detected: 60,
    n_catalog: 120,
    band: "G",
    match_radius_arcsec: 2,
    elapsed_ms: 9,
    warnings: ["xm warning"],
  };
}

function waveAxis(values: number[]): SpectralAxisInfo {
  return {
    kind: "wave",
    ctype: "WAVE",
    unit: "um",
    header_unit: "um",
    header_scale: 1,
    values,
    crval: values[0],
    cdelt: values.length > 1 ? values[1] - values[0] : 0,
    crpix: 1,
    rest_wavelength_um: null,
    rest_frequency_hz: null,
    specsys: "TOPOCENT",
    velosys: null,
    notes: [],
  };
}

function exportInput(): SpectrumExportInput {
  return {
    fileName: "x.fits#hdu=1",
    axis: waveAxis([1.0, 1.001]),
    mode: "wavelength_vac",
    restUm: null,
    convention: "optical",
    correction: "none",
    correctionResult: null,
    source: { kind: "pixel", x: 3, y: 4 },
    regionId: null,
    regionText: null,
    backgroundId: null,
    view: "sum",
    bunit: "Jy/beam",
    values: [1, 2],
    region: null,
    pixelFluxJy: null,
    pixelFluxJyError: null,
    sky: { ra: 10, dec: -5 },
    exportedAtUtc: CLOCK,
  };
}

function comparisonEntry(id: string, label: string): ComparisonEntry {
  return { id, kind: "region", label, color: "#fff", source: { kind: "region", shape: CIRCLE, background: null }, regionText: null, backgroundId: null, region: null, pixel: null, error: null };
}

function comparisonInput(): ComparisonCsvInput {
  return {
    fileName: "x.fits",
    view: "mean",
    mode: "offset",
    windows: null,
    axis: { values: [1.0, 1.001], label: "Wavelength", unit: "um", header: "wavelength_um" },
    axisKnown: true,
    vacuum: { values: [1.0, 1.001], restUm: null, restOrigin: null, reason: null },
    channelCount: 2,
    entries: [comparisonEntry("a", "A"), comparisonEntry("b", "B")],
    result: {
      series: [],
      plotted: [
        { id: "a", label: "A", unit: "Jy/beam", factor: 1, offset: 0 },
        { id: "b", label: "B", unit: "Jy/beam", factor: 1, offset: 2.5 },
      ],
      omitted: [],
      yLabel: "flux",
      step: 2.5,
    },
    specsys: "TOPOCENT",
    axisMode: "wavelength_vac",
    correction: "none",
    correctionResult: null,
    exportedAtUtc: CLOCK,
  };
}

function pvRun(resultExtra: Partial<PvDiagramResult> = {}): PvRun {
  return {
    params: {
      filePath: "C:\\d\\x.fits#hdu=1",
      regionId: "l1",
      line: { x0: 2, y0: 16, x1: 29, y1: 16 },
      stepPx: 1,
      widthPx: 3,
      z0: 0,
      z1: 39,
      mode: "velocity",
      restUm: null,
      convention: "optical",
      correction: "barycentric",
      velocityShiftKms: -7.25,
    },
    result: {
      fits_path: "C:/out/pv.fits",
      png_path: "C:/out/pv.png",
      dimensions: [3, 40],
      offsets: [-0.36, 0, 0.36],
      offset_unit: "arcsec",
      offset_step: 0.36,
      spectral_values: [],
      spectral_unit: "km/s",
      spectral_mode: "velocity",
      spectral_shift_kms: -7.25,
      spectral_rest_um: 1.02,
      spectral_convention: "optical",
      ridge: [null, 3, 1],
      ridge_channel: [null, 20, 18],
      peak_channel: [null, 20, 18],
      peak_value: [null, 1.5, 1.2],
      peak_spectral: [null, -50, -60],
      xs: [],
      ys: [],
      line: { x0: 2, y0: 16, x1: 29, y1: 16 },
      step_px: 1,
      width_px: 3,
      z0: 0,
      z1: 39,
      n_across: 3,
      summary: {
        n_offsets: 3,
        n_channels: 40,
        n_across: 3,
        n_off_image: 0,
        offset_step: 0.36,
        offset_unit: "arcsec",
        slit_length_px: 27,
        slit_length: 9.72,
        slit_pa_deg: 90,
        pixel_scale_arcsec: 0.36,
        ridge_gradient: null,
        ridge_gradient_unit: "km/s/arcsec",
        ridge_span: null,
        n_ridge_valid: 2,
        peak_value: 1.5,
        peak_offset: 0,
        peak_spectral: -50,
        bunit: "Jy/beam",
      },
      notes: ["n1"],
      elapsed_ms: 12,
      ...resultExtra,
    },
  };
}

describe("MeasurementLogCore", () => {
  it("append stamps id and an ISO 8601 UTC timestamp and notifies once", () => {
    const { store, listener } = makeStore();
    const entry = store.append(draft({ params: { a: 1 } }));
    expect(entry.id).toBe("id1");
    expect(entry.timestamp_utc).toBe("2026-09-25T12:00:00.000Z");
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.getSnapshot()).toEqual([entry]);
  });

  it("appendAll notifies once for many drafts and not at all for none", () => {
    const { store, listener } = makeStore();
    const entries = store.appendAll([draft(), draft(), draft()]);
    expect(entries.map((e) => e.id)).toEqual(["id1", "id2", "id3"]);
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.appendAll([])).toEqual([]);
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.getSnapshot()).toHaveLength(3);
  });

  it("getSnapshot returns the same array until the next mutation and EMPTY_LOG when empty", () => {
    const { store } = makeStore();
    expect(store.getSnapshot()).toBe(EMPTY_LOG);
    expect(EMPTY_LOG).toEqual([]);
    store.append(draft());
    const first = store.getSnapshot();
    expect(store.getSnapshot()).toBe(first);
    store.append(draft());
    expect(store.getSnapshot()).not.toBe(first);
    expect(first).toHaveLength(1);
  });

  it("clear drops every entry and does not notify when already empty", () => {
    const { store, listener } = makeStore();
    store.clear();
    expect(listener).toHaveBeenCalledTimes(0);
    store.appendAll([draft(), draft()]);
    store.clear();
    expect(listener).toHaveBeenCalledTimes(2);
    expect(store.getSnapshot()).toBe(EMPTY_LOG);
    store.clear();
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("the log keeps the newest MAX_LOG_ENTRIES entries", () => {
    const { store } = makeStore();
    for (let i = 0; i < MAX_LOG_ENTRIES + 1; i += 1) store.append(draft());
    const snapshot = store.getSnapshot();
    expect(snapshot).toHaveLength(MAX_LOG_ENTRIES);
    expect(snapshot[0].id).toBe("id2");
    expect(snapshot[snapshot.length - 1].id).toBe(`id${MAX_LOG_ENTRIES + 1}`);
  });

  it("entries are frozen and detached from the draft records", () => {
    const { store } = makeStore();
    const params: Record<string, number> = { a: 1 };
    const notes = ["n"];
    const entry = store.append(draft({ params, notes }));
    params.a = 2;
    notes.push("m");
    expect(entry.params.a).toBe(1);
    expect(entry.notes).toEqual(["n"]);
    expect(Object.isFrozen(entry)).toBe(true);
    expect(Object.isFrozen(entry.params)).toBe(true);
  });
});

describe("helpers", () => {
  it("dqHandlingOf maps the requested flag and the masked report to the five words", () => {
    expect(dqHandlingOf(true, true)).toBe("excluded");
    expect(dqHandlingOf(true, false)).toBe("requested");
    expect(dqHandlingOf(true, null)).toBe("requested");
    expect(dqHandlingOf(false, true)).toBe("off");
    expect(dqHandlingOf(false, false)).toBe("off");
    expect(dqHandlingOf(false, null)).toBe("off");
  });

  it("imageLabelOf falls back to original when nothing is processed", () => {
    expect(imageLabelOf(null)).toBe("original");
    expect(imageLabelOf({ text: "Stretch", title: "t", tone: "processed" })).toBe("Stretch");
  });

  it("fileNameOf strips the plane fragment and handles both separators", () => {
    expect(fileNameOf("C:\\a\\b.fits#hdu=2")).toBe("b.fits");
    expect(fileNameOf("/x/y/c.fits#array=SCI")).toBe("c.fits");
    expect(fileNameOf("d.fits")).toBe("d.fits");
    expect(fileNameOf("C:/obs/Night #3/m31.fits")).toBe("m31.fits");
    expect(fileNameOf("C:/obs/M31#2.fits#hdu=1")).toBe("M31#2.fits");
    expect(fileNameOf("/x/M31#2.fits#array=SCI")).toBe("M31#2.fits");
    expect(fileNameOf("/x/e.fits#notaplane")).toBe("e.fits#notaplane");
    expect(fileNameOf(null)).toBeNull();
  });

  it("measurementLogCsv pins the fixed header and unions p_ and v_ columns in first-seen order", () => {
    const { store } = makeStore();
    store.append(draft({ params: { a: 1 }, values: { x: 2 }, notes: ["n1", "n2"] }));
    store.append(draft({ kind: "pixel", dq: "reported", unit: "ADU", params: { b: "q" }, values: { y: 3 } }));
    const csv = measurementLogCsv(store.getSnapshot());
    const lines = csv.split("\r\n");
    expect(lines[0]).toBe("timestamp_utc,kind,file,image,dq,unit,source,notes,p_a,p_b,v_x,v_y");
    expect(lines[1]).toBe(`${CLOCK},photometry,x.fits,original,off,,s,n1 | n2,1,,2,`);
    expect(lines[2]).toBe(`${CLOCK},pixel,x.fits,original,reported,ADU,s,,,q,,3`);
    expect(lines[3]).toBe("");
    expect(csv.endsWith("\r\n")).toBe(true);
    expect(csv.includes("\n")).toBe(true);
    expect(csv.replace(/\r\n/g, "").includes("\n")).toBe(false);
  });

  it("measurementLogCsv quotes the JSON source and a note with a comma", () => {
    const { store } = makeStore();
    store.append(draft({ source: '{"kind":"pixel","x":1,"y":2}', notes: ["a, b"] }));
    const row = measurementLogCsv(store.getSnapshot()).split("\r\n")[1];
    expect(row).toBe(`${CLOCK},photometry,x.fits,original,off,,"{""kind"":""pixel"",""x"":1,""y"":2}","a, b"`);
  });

  it("measurementLogCsvFileName strips the hdu fragment and extension and has a session default", () => {
    expect(measurementLogCsvFileName("C:\\a\\x.fits#hdu=1")).toBe("x_measurements.csv");
    expect(measurementLogCsvFileName("/y/z.fits.gz")).toBe("z_measurements.csv");
    expect(measurementLogCsvFileName(null)).toBe("astroburst_measurements.csv");
  });

  it("filterEntries by kind and by file, with one file identity for a plane-selected cube", () => {
    const { store } = makeStore();
    store.append(photometryEntry(PROV, photometry(), { gaiaMatch: true, excludeDq: true }));
    store.append(lineEntry("C:\\d\\x.fits#hdu=1", lineMeasurement(), "gaussian"));
    store.append(pixelEntry({ file: "y.fits", image: "original" }, pixelResult()));
    const entries = store.getSnapshot();
    expect(entries.map((e) => e.file)).toEqual(["x.fits", "x.fits", "y.fits"]);
    expect(filterEntries(entries, "all", "x.fits").map((e) => e.kind)).toEqual(["photometry", "line"]);
    expect(filterEntries(entries, "all", null)).toHaveLength(3);
    expect(filterEntries(entries, "line", null).map((e) => e.kind)).toEqual(["line"]);
    expect(filterEntries(entries, "line", "y.fits")).toHaveLength(0);
  });

  it("regionLogReady requires the same regions array reference and identical clip and DQ settings", () => {
    const regions = [region("r1")];
    const measured: RegionStatsMeasured = { regions, excludeDq: true, sigma: 3, maxiters: 5, masked: true, dqExcluded: 4, elapsedMs: 2 };
    expect(regionLogReady(measured, { regions, excludeDq: true, sigma: 3, maxiters: 5 })).toBe(true);
    expect(regionLogReady(measured, { regions: [...regions], excludeDq: true, sigma: 3, maxiters: 5 })).toBe(false);
    expect(regionLogReady(measured, { regions, excludeDq: true, sigma: 2.5, maxiters: 5 })).toBe(false);
    expect(regionLogReady(measured, { regions, excludeDq: false, sigma: 3, maxiters: 5 })).toBe(false);
    expect(regionLogReady(measured, { regions, excludeDq: true, sigma: 3, maxiters: null })).toBe(false);
    expect(regionLogReady(null, { regions, excludeDq: true, sigma: 3, maxiters: 5 })).toBe(false);
  });

  it("profileLogReady and statisticsLogReady compare the request the result was measured with", () => {
    expect(profileLogReady({ requestKey: "k", excludeDq: true }, "k", true)).toBe(true);
    expect(profileLogReady({ requestKey: "k", excludeDq: true }, "other", true)).toBe(false);
    expect(profileLogReady({ requestKey: "k", excludeDq: true }, "k", false)).toBe(false);
    expect(profileLogReady(null, "k", true)).toBe(false);
    expect(profileLogReady({ requestKey: "k", excludeDq: true }, null, true)).toBe(false);

    const single = statisticsInput({ region: CIRCLE, excludeDq: true, noise: false });
    expect(statisticsLogReady(single, { composite: false, noise: false, excludeDq: true, region: CIRCLE })).toBe(true);
    expect(statisticsLogReady(single, { composite: false, noise: false, excludeDq: true, region: { ...CIRCLE } })).toBe(false);
    expect(statisticsLogReady(single, { composite: false, noise: false, excludeDq: false, region: CIRCLE })).toBe(false);
    expect(statisticsLogReady(single, { composite: false, noise: true, excludeDq: true, region: CIRCLE })).toBe(false);
    expect(statisticsLogReady(null, { composite: false, noise: false, excludeDq: true, region: CIRCLE })).toBe(false);

    const composite = statisticsInput({ composite: true, region: null, excludeDq: true, noise: false });
    expect(statisticsLogReady(composite, { composite: true, noise: false, excludeDq: false, region: null })).toBe(true);
    expect(statisticsLogReady(composite, { composite: false, noise: false, excludeDq: true, region: null })).toBe(false);
    expect(statisticsLogReady(composite, { composite: true, noise: true, excludeDq: true, region: null })).toBe(false);
  });
});

describe("builders", () => {
  it("photometryEntry records requested and effective apertures, the DQ report, the sky position and never invents a gain or a FWHM", () => {
    const res = photometry();
    const entry = photometryEntry(PROV, res, { gaiaMatch: true, excludeDq: true });
    expect(entry.kind).toBe("photometry");
    expect(entry.file).toBe("x.fits");
    expect(entry.image).toBe("original");
    expect(entry.params.aperture_radius_px).toBe("auto");
    expect(entry.params.annulus_inner_px).toBe("auto");
    expect(entry.params.annulus_outer_px).toBe("auto");
    expect(entry.params.aperture_effective_px).toBe(res.photometry.aperture_radius);
    expect(entry.params.sky_inner_effective_px).toBe(9.6);
    expect(entry.params.sky_outer_effective_px).toBe(14.4);
    expect(entry.params.gain).toBe("none");
    expect(entry.params.gaia_match).toBe(true);
    expect(entry.params.photcal).toBe("JWST MJy/sr");
    expect(entry.dq).toBe("excluded");
    expect(entry.unit).toBe("MJy/sr");
    expect(entry.source).toBe('{"kind":"pixel","x":10,"y":20}');
    expect(entry.values.ra).toBe(150.1);
    expect(entry.values.dec).toBe(2.2);
    expect(entry.values.net_flux).toBe(1234.5);
    expect(entry.values.fwhm).toBe(3.2);
    expect(entry.values.n_masked).toBe(2);
    expect(entry.values.saturated).toBe(false);
    expect(entry.values.gaia_gmag).toBeNull();
    expect(entry.notes).toEqual(["w1"]);

    const sent = photometryEntry(PROV, photometry({ masked: false, sky: null }), { apertureRadius: 5, annulusInner: 8, annulusOuter: 12, gain: 2.5, gaiaMatch: false, excludeDq: true });
    expect(sent.params.aperture_radius_px).toBe(5);
    expect(sent.params.annulus_inner_px).toBe(8);
    expect(sent.params.annulus_outer_px).toBe(12);
    expect(sent.params.gain).toBe(2.5);
    expect(sent.dq).toBe("requested");
    expect(sent.values.ra).toBeNull();
    expect(photometryEntry(PROV, photometry({ masked: false }), { gaiaMatch: true, excludeDq: false }).dq).toBe("off");

    const failed = photometryEntry(PROV, photometry({ photometry: star({ fwhm: 0, snr: null as unknown as number }), photcal: null }), { gaiaMatch: true, excludeDq: false });
    expect(failed.values.fwhm).toBeNull();
    expect(failed.values.snr).toBeNull();
    expect(failed.unit).toBeNull();
    expect(failed.params.photcal).toBeNull();

    const batch = batchPhotometryEntry(PROV, { rows: [], photcal: null, warnings: [], masked: false, n_measured: 0, n_failed: 0, elapsed_ms: 1 }, { excludeDq: false, from: "stars" });
    expect(batch.params.gain).toBe("none");
    const series = timeSeriesEntry(analyticSeries(), { apertureRadius: 5, excludeDq: false, trackDrift: true });
    expect(series.params.gain).toBe("none");
  });

  it("batchPhotometryEntry summarises a run with medians over finite measured values and counts failures", () => {
    const rows: BatchPhotometryResult["rows"] = [
      { index: 0, photometry: star({ net_flux: 1, snr: 10, fwhm: 2, saturated: true }), sky: null, error: null },
      { index: 1, photometry: star({ net_flux: 3, snr: null as unknown as number, fwhm: 0 }), sky: null, error: null },
      { index: 2, photometry: star({ net_flux: 5, snr: 30, fwhm: 4 }), sky: null, error: null },
      { index: 3, photometry: null, sky: null, error: "off image" },
    ];
    const res: BatchPhotometryResult = { rows, photcal: PHOTCAL, warnings: ["bw"], masked: true, n_measured: 3, n_failed: 1, elapsed_ms: 7 };
    const entry = batchPhotometryEntry(PROV, res, { apertureRadius: 4, excludeDq: true, from: "regions" });
    expect(entry.kind).toBe("photometry_batch");
    expect(entry.dq).toBe("excluded");
    expect(entry.unit).toBe("MJy/sr");
    expect(entry.source).toBe('{"kind":"points","n":4,"from":"regions"}');
    expect(entry.params.aperture_radius_px).toBe(4);
    expect(entry.params.photcal).toBe("JWST MJy/sr");
    expect(entry.values.n_points).toBe(4);
    expect(entry.values.n_measured).toBe(3);
    expect(entry.values.n_failed).toBe(1);
    expect(entry.values.n_saturated).toBe(1);
    expect(entry.values.median_net_flux).toBe(3);
    expect(entry.values.median_snr).toBe(20);
    expect(entry.values.median_fwhm).toBe(3);
    expect(entry.notes).toEqual(["bw"]);

    const sentinelOnly = batchPhotometryEntry(PROV, { ...res, rows: [{ index: 0, photometry: star({ fwhm: 0 }), sky: null, error: null }], n_measured: 1, n_failed: 0 }, { excludeDq: false, from: "pasted" });
    expect(sentinelOnly.values.median_fwhm).toBeNull();
    expect(medianOf([])).toBeNull();
    expect(medianOf([null, NaN])).toBeNull();
    expect(medianOf([4, 1, 3, 2])).toBe(2.5);
  });

  it("regionStatsEntries makes one row per measured region with the calibrated block and an error note for a missing entry", () => {
    const bg = region("bg", { shape: ANNULUS });
    const r1 = region("r1", { backgroundId: "bg", props: { color: null, width: null, text: "core", dash: null, include: true } });
    const r3 = region("r3", { shape: { shape: "box", x: 1, y: 2, width: 3, height: 4, angle: 0 } });
    const r4 = region("r4");
    const measured: RegionStatsMeasured = { regions: [r1, bg, r3, r4], excludeDq: true, sigma: null, maxiters: null, masked: false, dqExcluded: 9, elapsedMs: 3 };
    const table = new Map<string, RegionStatsEntry>([
      ["r1", { id: "r1", stats: stats({ calibrated: calibrated(), sky: { ra: 150.5, dec: 2.25, pa_sky_deg: 12.5, area_arcsec2: 0.126, geometric_area_arcsec2: 0.126 } }), error: null }],
      ["bg", { id: "bg", stats: stats({ count: 200, sum: 800 }), error: null }],
      ["r4", { id: "r4", stats: null, error: "off image" }],
    ]);
    const rows = regionStatsEntries(PROV, measured, table, PHOTCAL);
    expect(rows).toHaveLength(4);
    expect(rows.every((r) => r.kind === "region_stats" && r.dq === "requested" && r.unit === "MJy/sr")).toBe(true);
    expect(rows[0].source).toBe(JSON.stringify({ kind: "region", label: "core", shape: CIRCLE, background: ANNULUS }));
    expect(rows[0].params).toEqual({ sigma: 3, maxiters: 5, dq_excluded_px: 9, photcal: "JWST MJy/sr" });
    expect(rows[0].values.count).toBe(49);
    expect(rows[0].values.n_excluded).toBe(2);
    expect(rows[0].values.area_px).toBe(50.27);
    expect(rows[0].values.flux_jy).toBe(3.92e-6);
    expect(rows[0].values.mag_ab).toBe(22.417);
    expect(rows[0].values.flux_source).toBe("sum");
    expect(rows[0].values.ra).toBe(150.5);
    expect(rows[0].values.pa_sky_deg).toBe(12.5);
    expect(rows[0].values.sb_mag_arcsec2).toBe(20.17);
    expect(rows[0].notes).toEqual([]);
    expect(rows[1].source).toBe(JSON.stringify({ kind: "region", label: null, shape: ANNULUS, background: null }));
    expect(rows[1].values.count).toBe(200);
    expect(rows[1].values.flux_jy).toBeNull();
    expect(rows[2].values).toEqual({});
    expect(rows[2].notes).toEqual(["no statistics"]);
    expect(rows[3].values).toEqual({});
    expect(rows[3].notes).toEqual(["off image"]);
    const clipped = regionStatsEntries(PROV, { ...measured, sigma: 2.5, maxiters: 10, masked: true }, table, null);
    expect(clipped[0].params.sigma).toBe(2.5);
    expect(clipped[0].params.maxiters).toBe(10);
    expect(clipped[0].dq).toBe("excluded");
    expect(clipped[0].unit).toBeNull();
  });

  it("lineEntry copies the echoed z range, rest wavelength and velocities, has no DQ and states the unit of every value", () => {
    const entry = lineEntry("C:\\d\\x.fits#hdu=1", lineMeasurement(), "gaussian");
    expect(entry.kind).toBe("line");
    expect(entry.file).toBe("x.fits");
    expect(entry.image).toBe("loaded cube");
    expect(entry.dq).toBe("not_applied");
    expect(entry.unit).toBe("Jy/beam x um");
    expect(entry.source).toBe('{"kind":"pixel","x":3,"y":4}');
    expect(entry.params.z0).toBe(10);
    expect(entry.params.z1).toBe(30);
    expect(entry.params.continuum_windows).toBe("[[2,8],[32,38]]");
    expect(entry.params.model).toBe("gaussian");
    expect(entry.params.rest_um).toBe(0.6565);
    expect(entry.params.convention).toBe("optical");
    expect(entry.params.shift_applied_kms).toBe(-7.25);
    expect(entry.params.axis_frame).toBe("barycentric");
    expect(entry.params.spectrum_unit).toBe("Jy/beam");
    expect(entry.values.flux).toBe(1.5);
    expect(entry.values.fwhm_kms).toBe(353.2);
    expect(entry.values.fit_centre).toBe(1.0011);
    expect(entry.values.fit_converged).toBe(true);
    expect(entry.values.peak_channel).toBe(20);
    const last = entry.notes[entry.notes.length - 1];
    expect(entry.notes[0]).toBe("line note");
    expect(last).toBe(lineUnitNote("Jy/beam x um", "um"));
    expect(last).toContain("peak, continuum_level, continuum_sigma, fit_amplitude in the spectrum unit (Jy/beam)");
    expect(last).toContain("centroid, sigma, fwhm, equivalent_width, fit_centre, fit_sigma, fit_centre_err, fit_sigma_err in um");
    expect(spectrumUnitOf("MJy/sr x pix x um", "um")).toBe("MJy/sr x pix");
    expect(spectrumUnitOf("Jy", "um")).toBeNull();

    const bare = lineEntry("/a/y.fits", lineMeasurement({ velocity: null, fit: null, source: { kind: "region", shape: CIRCLE, background: null } }), "none");
    expect(bare.params.rest_um).toBeNull();
    expect(bare.params.axis_frame).toBeNull();
    expect(bare.values.fit_amplitude).toBeNull();
    expect(bare.values.fit_converged).toBeNull();
    expect(bare.source).toBe(JSON.stringify({ kind: "region", shape: CIRCLE, background: null }));
  });

  it("timeSeriesEntry computes the target rms from the differential light curve and names the time scale", () => {
    const res = analyticSeries();
    const entry = timeSeriesEntry(res, { apertureRadius: 5, annulusInner: 8, annulusOuter: 12, gain: 1.5, excludeDq: true, trackDrift: false });
    expect(entry.kind).toBe("time_series");
    expect(entry.file).toBe("f0.fits");
    expect(entry.image).toBe("3 frames");
    expect(entry.dq).toBe("requested");
    expect(entry.unit).toBe("mag");
    expect(JSON.parse(entry.source)).toEqual({ kind: "frames", n: 3, reference: "f0.fits", targets: res.targets });
    expect(entry.params.n_frames).toBe(3);
    expect(entry.params.n_skipped).toBe(0);
    expect(entry.params.aperture_radius_px).toBe(5);
    expect(entry.params.annulus_inner_px).toBe(8);
    expect(entry.params.annulus_outer_px).toBe(12);
    expect(entry.params.gain).toBe(1.5);
    expect(entry.params.track_drift).toBe(false);
    expect(entry.params.target_label).toBe("T");
    expect(entry.params.n_comp).toBe(2);
    expect(entry.params.n_check).toBe(0);
    expect(entry.params.first_frame).toBe("f0.fits");
    expect(entry.params.last_frame).toBe("f2.fits");
    expect(entry.values.n_used).toBe(3);
    expect(entry.values.target_rms_mag).toBeCloseTo((2.5 * Math.log10(1.1) * Math.SQRT2) / 3, 9);
    expect(entry.values.check_rms_mag).toBe(0);
    expect(entry.values.check_label).toBe("C1");
    expect(entry.values.jd_mid_first).toBe(2460000.5);
    expect(entry.values.jd_mid_last).toBe(2460002.5);
    expect(entry.values.time_span_days).toBe(2);
    expect(entry.values.time_source).toBe("DATE-OBS + EXPTIME/2 (UTC)");
    expect(entry.values.bjd_tdb_first).toBeNull();
    expect(entry.values.bjd_tdb_last).toBeNull();
    expect(entry.notes).toEqual(["ts warning"]);

    const incomplete = timeSeries(res.frames.map((f, i) => (i === 1 ? { ...f, jd_mid: null } : f)));
    const partial = timeSeriesEntry(incomplete, { apertureRadius: 5, excludeDq: false, trackDrift: true });
    expect(partial.dq).toBe("off");
    expect(partial.values.jd_mid_first).toBeNull();
    expect(partial.values.jd_mid_last).toBeNull();
    expect(partial.values.time_span_days).toBeNull();
    expect(partial.values.time_source).toBeNull();
    expect(partial.params.first_frame).toBe("f0.fits");
    expect(partial.params.last_frame).toBe("f2.fits");
  });

  it("sbProfileEntry, radialProfileEntry and lineCutEntry echo the payload and the measured request, never live state", () => {
    const reg = region("r1", { backgroundId: "missing", props: { color: null, width: null, text: "r1", dash: null, include: true } });
    const data: RadialProfile = {
      x: 10.5,
      y: 20.5,
      max_radius: 30,
      background: { median: 1.5, sigma: 0.2, count: 400 },
      bins: [
        { r: 0, count: 4, mean: 10, median: 10, std: 1, cumulative_sum: 40 },
        { r: 1, count: 12, mean: 5, median: 5, std: 1, cumulative_sum: 100 },
      ],
      masked: true,
      elapsed_ms: 2,
    };
    const radial = radialProfileEntry(PROV, data, true, { x: 10, y: 20, maxRadius: 30, background: [40, 60] }, reg);
    expect(radial.kind).toBe("radial_profile");
    expect(radial.dq).toBe("excluded");
    expect(radial.unit).toBeNull();
    expect(radial.params.x).toBe(10.5);
    expect(radial.params.max_radius_px).toBe(30);
    expect(radial.params.background_inner_px).toBe(40);
    expect(radial.params.background_outer_px).toBe(60);
    expect(radial.values.n_bins).toBe(2);
    expect(radial.values.background_median).toBe(1.5);
    expect(radial.values.background_count).toBe(400);
    expect(radial.values.cumulative_sum_last).toBe(100);
    expect(JSON.parse(radial.source)).toEqual({ kind: "region", label: "r1", shape: CIRCLE, background: null });
    const noBg = radialProfileEntry(PROV, { ...data, background: null, bins: [] }, false, { x: 10, y: 20, maxRadius: 30, background: null }, reg);
    expect(noBg.params.background_inner_px).toBeNull();
    expect(noBg.params.background_outer_px).toBeNull();
    expect(noBg.values.background_median).toBeNull();
    expect(noBg.values.cumulative_sum_last).toBeNull();
    expect(noBg.dq).toBe("off");

    const sb: SbProfile = {
      x: 50,
      y: 50,
      sma_max: 3,
      ellipticity: 0.4,
      angle_deg: 30,
      bin_width: 2,
      background: { median: 0.7, sigma: 0.1, count: 50 },
      bins: [{ sma_inner: 0, sma_outer: 1, sma: 0.5, count: 4, cumulative_count: 4, mean: 100, median: 90, std: 4, cumulative_sum: 50, sma_arcsec: 0.025, mu_ab: 20.05, mu_err: 0.01, mag_ab_cumulative: 15 }],
      r50_px: 0.5,
      r80_px: 1.5,
      r90_px: 2,
      petrosian_radius_px: 2.4,
      pixel_scale_arcsec: 0.05,
      pixel_area_arcsec2: 0.0025,
      sky_pa_deg: 120,
      photcal: PHOTCAL,
      calibration_warnings: ["cw"],
      total_mag_ab: 14.5,
      notes: ["sb note"],
      masked: false,
      elapsed_ms: 3,
    };
    const sbEntry = sbProfileEntry(PROV, sb, false, { shape: CIRCLE, background: ANNULUS, binWidth: 2 }, reg);
    expect(sbEntry.kind).toBe("sb_profile");
    expect(sbEntry.dq).toBe("off");
    expect(sbEntry.unit).toBe("MJy/sr");
    expect(JSON.parse(sbEntry.source)).toEqual({ kind: "region", label: "r1", shape: CIRCLE, background: ANNULUS });
    expect(sbEntry.params).toEqual({ x: 50, y: 50, sma_max_px: 3, ellipticity: 0.4, angle_deg: 30, bin_width_px: 2, photcal: "JWST MJy/sr" });
    expect(sbEntry.values.r50_px).toBe(0.5);
    expect(sbEntry.values.petrosian_radius_px).toBe(2.4);
    expect(sbEntry.values.total_mag_ab).toBe(14.5);
    expect(sbEntry.values.mu_0_mag_arcsec2).toBe(20.05);
    expect(sbEntry.values.sky_pa_deg).toBe(120);
    expect(sbEntry.values.n_bins).toBe(1);
    expect(sbEntry.values.background_median).toBe(0.7);
    expect(sbEntry.notes).toEqual(["sb note", "cw"]);
    const sbNoBg = sbProfileEntry(PROV, { ...sb, background: null, photcal: null }, true, { shape: CIRCLE, background: null, binWidth: 2 }, reg);
    expect(JSON.parse(sbNoBg.source).background).toBeNull();
    expect(sbNoBg.values.background_sigma).toBeNull();
    expect(sbNoBg.unit).toBeNull();
    expect(sbNoBg.dq).toBe("requested");

    const cut: LineCut = { x1: 1, y1: 2, x2: 4, y2: 6, length: 5, n_samples: 6, distance: [0, 1, 2, 3, 4, 5], xs: [], ys: [], values: [null, 2, NaN, 9, 4, null], masked: false, elapsed_ms: 1 };
    const cutEntry = lineCutEntry(PROV, cut, true, region("l", { shape: LINE }));
    expect(cutEntry.kind).toBe("line_cut");
    expect(cutEntry.dq).toBe("requested");
    expect(cutEntry.params).toEqual({ x1: 1, y1: 2, x2: 4, y2: 6 });
    expect(cutEntry.values).toEqual({ length_px: 5, n_samples: 6, value_min: 2, value_max: 9 });
    expect(cutEntry.unit).toBeNull();
    const emptyCut = lineCutEntry(PROV, { ...cut, values: [null, null] }, false, region("l", { shape: LINE }));
    expect(emptyCut.values.value_min).toBeNull();
    expect(emptyCut.values.value_max).toBeNull();
  });

  it("skySeparationEntry logs arcsec with the position angle east of north", () => {
    const sky: SkySeparationResult = { a_sky: [150.1, 2.2], b_sky: [150.2, 2.3], separation_deg: 0.01, separation_arcmin: 0.6, separation_arcsec: 36, position_angle_deg: 45, pixel_length: 5, pixel_scale_arcsec: 7.2 };
    const entry = skySeparationEntry(PROV, LINE, sky);
    expect(entry.kind).toBe("sky_separation");
    expect(entry.dq).toBe("not_applied");
    expect(entry.unit).toBe("arcsec");
    expect(JSON.parse(entry.source)).toEqual({ kind: "region", label: null, shape: LINE, background: null });
    expect(entry.params).toEqual({ x1: 1, y1: 2, x2: 4, y2: 6 });
    expect(entry.values).toEqual({ a_ra: 150.1, a_dec: 2.2, b_ra: 150.2, b_dec: 2.3, separation_arcsec: 36, separation_arcmin: 0.6, separation_deg: 0.01, position_angle_deg: 45, pixel_length: 5, pixel_scale_arcsec: 7.2 });
  });

  it("pixelEntry logs the centre cell with its DQ names and dq reported", () => {
    const entry = pixelEntry(PROV, pixelResult());
    expect(entry.kind).toBe("pixel");
    expect(entry.dq).toBe("reported");
    expect(entry.unit).toBe("MJy/sr");
    expect(entry.source).toBe('{"kind":"pixel","x":12,"y":34}');
    expect(entry.params).toEqual({ size: 3, dq_table: "JWST" });
    expect(entry.values).toEqual({ x: 12, y: 34, value: 5.5, err: 0.25, dq_value: 4, dq_names: "SATURATED", window_min: 1, window_max: 9, window_mean: 5.06, window_median: 5.5, n_finite: 9, n_nan: 0 });
    const bare = pixelEntry(PROV, pixelResult({ err: null, dq: null, dq_names: null, dq_table: null, unit: null }));
    expect(bare.values.err).toBeNull();
    expect(bare.values.dq_value).toBeNull();
    expect(bare.values.dq_names).toBeNull();
    expect(bare.params.dq_table).toBeNull();
    expect(bare.unit).toBeNull();
  });

  it("statisticsEntry prefixes channel keys only for composite results and marks composite as not_applied", () => {
    const composite = statisticsEntry({ file: "x.fits", image: "composite" }, statisticsInput({
      channels: [
        { label: "R", body: channelBody(1) },
        { label: "G", body: channelBody(2) },
        { label: "B", body: channelBody(3) },
      ],
      unit: null,
      masked: false,
      dqExcluded: null,
      composite: true,
      region: null,
      excludeDq: true,
    }));
    expect(composite.kind).toBe("statistics");
    expect(composite.image).toBe("composite");
    expect(composite.dq).toBe("not_applied");
    expect(composite.source).toBe('{"kind":"image"}');
    expect(composite.unit).toBeNull();
    expect(composite.params).toEqual({ noise: true, region_kind: null, dq_excluded_px: null });
    expect(composite.values.r_mean).toBe(1);
    expect(composite.values.g_mean).toBe(2);
    expect(composite.values.b_mean).toBe(3);
    expect(composite.values.r_noise_sigma).toBe(0.05);
    expect(composite.values.mean).toBeUndefined();

    const single = statisticsEntry(PROV, statisticsInput({ region: CIRCLE, regionKind: "circle" }));
    expect(single.dq).toBe("excluded");
    expect(single.unit).toBe("ADU");
    expect(single.source).toBe(JSON.stringify({ kind: "region", label: null, shape: CIRCLE, background: null }));
    expect(single.params).toEqual({ noise: true, region_kind: "circle", dq_excluded_px: 17 });
    expect(single.values.mean).toBe(2.5);
    expect(single.values.std_dev).toBe(0.2);
    expect(single.values.excluded).toBe(3);
    expect(single.values.noise_fraction).toBe(0.9);
    const noNoise = statisticsEntry(PROV, statisticsInput({ masked: false, channels: [{ label: "K", body: { ...channelBody(2), noise: null } }] }));
    expect(noNoise.dq).toBe("requested");
    expect(noNoise.values.noise_sigma).toBeNull();
  });

  it("crossMatchEntry logs the zero point in mag and the astrometric rms in arcsec", () => {
    const entry = crossMatchEntry("C:\\d\\x.fits", crossMatch(), { sigma: 5, maxStars: 500, colourTerm: true });
    expect(entry.kind).toBe("catalog_crossmatch");
    expect(entry.file).toBe("x.fits");
    expect(entry.image).toBe("loaded file");
    expect(entry.dq).toBe("not_applied");
    expect(entry.unit).toBe("mag");
    expect(entry.source).toBe('{"kind":"file"}');
    expect(entry.params).toEqual({ band: "G", match_radius_arcsec: 2, colour_term: true, detection_sigma: 5, max_stars: 500, epoch_year: 2024.3 });
    expect(entry.values.zp).toBe(25.31);
    expect(entry.values.zp_err).toBe(0.02);
    expect(entry.values.zp_rms).toBe(0.05);
    expect(entry.values.colour_coeff).toBeNull();
    expect(entry.values.astrometry_rms_arcsec).toBe(0.12);
    expect(entry.values.astrometry_n).toBe(40);
    expect(entry.values.n_matches).toBe(0);
    expect(entry.values.n_detected).toBe(60);
    expect(entry.values.photcal_present).toBe(false);
    expect(entry.notes).toEqual(["xm warning"]);
    const empty = crossMatchEntry("/a/y.fits", { ...crossMatch(), astrometry: null, zero_point: null }, { sigma: 5, maxStars: 500, colourTerm: false });
    expect(empty.values.zp).toBeNull();
    expect(empty.values.astrometry_rms_arcsec).toBeNull();
  });

  it("spectrumExportEntry, spectrumCompareEntry and pvEntry build rows from the wave-1 records", () => {
    const exported = spectrumExportEntry("C:\\d\\x.fits#hdu=1", exportInput(), null);
    expect(exported.kind).toBe("spectrum_export");
    expect(exported.file).toBe("x.fits");
    expect(exported.image).toBe("loaded cube");
    expect(exported.dq).toBe("not_applied");
    expect(exported.source).toBe('{"kind":"pixel","x":3,"y":4}');
    expect(exported.params.saved_path).toBeNull();
    expect(exported.params.n_channels).toBe(2);
    expect(exported.params.flux_unit).toBe("Jy/beam");
    expect(exported.params.view).toBe("sum");
    expect(exported.unit).toBe("Jy/beam");
    expect(exported.values.flux_max).toBe(2);
    expect(exported.values.ra_deg).toBe(10);
    expect(exported.values.flux_jy_max).toBeNull();
    expect(spectrumExportEntry("C:\\d\\x.fits#hdu=1", exportInput(), "C:/out/x.csv").params.saved_path).toBe("C:/out/x.csv");

    const compared = spectrumCompareEntry("C:\\d\\x.fits#hdu=1", comparisonInput(), "C:/out/cmp.csv");
    expect(compared.kind).toBe("spectrum_compare");
    expect(compared.file).toBe("x.fits");
    expect(compared.dq).toBe("not_applied");
    expect(compared.source).toBe('{"kind":"file"}');
    expect(compared.params).toEqual({ normalisation: "offset", offset_step: 2.5, view: "mean", saved_path: "C:/out/cmp.csv" });
    expect(compared.values).toEqual({ n_series: 2 });
    expect(compared.unit).toBeNull();
    const peak = spectrumCompareEntry("/a/y.fits", { ...comparisonInput(), mode: "peak" }, "/out/c.csv");
    expect(peak.params.offset_step).toBeNull();

    const pv = pvEntry(pvRun());
    expect(pv.kind).toBe("pv");
    expect(pv.file).toBe("x.fits");
    expect(pv.image).toBe("loaded cube");
    expect(pv.dq).toBe("not_applied");
    expect(pv.unit).toBe("km/s");
    expect(pv.source).toBe('{"kind":"region","label":null,"shape":{"shape":"line","x1":2,"y1":16,"x2":29,"y2":16},"background":null}');
    expect(pv.params.step_px).toBe(1);
    expect(pv.params.width_px).toBe(3);
    expect(pv.params.z0).toBe(0);
    expect(pv.params.z1).toBe(39);
    expect(pv.params.mode).toBe("velocity");
    expect(pv.params.convention).toBe("optical");
    expect(pv.params.velocity_shift_kms).toBe(-7.25);
    expect(pv.params.correction).toBe("barycentric");
    expect(pv.params.rest_um).toBe(1.02);
    expect(pv.params.offset_unit).toBe("arcsec");
    expect(pv.params.spectral_unit).toBe("km/s");
    expect(pv.params.fits_path).toBe("C:/out/pv.fits");
    expect(pv.values).toEqual({ n_offsets: 3, n_channels: 40, ridge_min: 1, ridge_max: 3 });
    expect(pv.notes).toEqual(["n1"]);
    const flat = pvEntry(pvRun({ ridge: [null, null] }));
    expect(flat.values.ridge_min).toBeNull();
    expect(flat.values.ridge_max).toBeNull();
  });

  it("formatLogValue and entrySummary render numbers, strings, booleans and null", () => {
    expect(formatLogValue(0.04878213)).toBe("0.0487821");
    expect(formatLogValue(2.5)).toBe("2.5");
    expect(formatLogValue(7)).toBe("7");
    expect(formatLogValue(1234567.89)).toBe("1234570");
    expect(formatLogValue("auto")).toBe("auto");
    expect(formatLogValue(true)).toBe("true");
    expect(formatLogValue(false)).toBe("false");
    expect(formatLogValue(null)).toBe("--");
    const entry: MeasurementLogEntry = {
      id: "i",
      timestamp_utc: CLOCK,
      kind: "photometry",
      file: null,
      image: "original",
      dq: "off",
      unit: null,
      source: "s",
      params: {},
      values: { a: 1, b: "x", c: null, d: 2.5 },
      notes: [],
    };
    expect(entrySummary(entry)).toBe("a=1, b=x, c=--");
    expect(entrySummary(entry, 4)).toBe("a=1, b=x, c=--, d=2.5");
    expect(entrySummary({ ...entry, values: {} })).toBe("");
  });
});
