import { describe, expect, it } from "vitest";
import {
  BMJD_OFFSET,
  EXCLUDABLE_FLAGS,
  FLAG_CENTROID_JUMP,
  FLAG_COMP_SATURATED,
  FLAG_LOW_CONFIDENCE_DRIFT,
  FLAG_MEASUREMENT_ERROR,
  FLAG_SATURATED,
  FLAG_SKIPPED,
  TIME_SCALE_NAMES,
  convertBarycentric,
  docTimeLabel,
  frameGroupKey,
  lightCurveDocFromFrames,
  lightCurveDocFromTable,
  periodogramEntry,
  quickLookEntry,
  rawFluxDocFromFrames,
  tableGroupKey,
  timeOffset,
  timeScaleLabel,
  type LightCurveTableInput,
  type LightCurveTableMapping,
} from "../lightCurveDoc";
import { MAG_ERR_FACTOR, timeAxisLabel } from "../differentialPhotometry";
import { QUICK_LOOK_LABEL } from "../lightCurveAnalysis";
import { periodStep } from "../lightCurveInputs";
import { measurementLogCsv } from "../measurementLog";
import type { PeriodogramResult } from "../../shared/types/periodogram";
import type { StarPhotometry } from "../../services/analysis";
import type { TimeSeriesFrame, TimeSeriesResult } from "../../shared/types/analysis";
import type { FrameGeometry } from "../../shared/types/geometry";
import type { LightCurveDoc, LightCurveQuickLook } from "../../shared/types/lightCurve";

function star(x: number, y: number, flux: number, err: number, extra: Partial<StarPhotometry> = {}): StarPhotometry {
  return {
    x,
    y,
    peak: 100,
    net_flux: flux,
    flux_err: err,
    mag_inst: -2.5 * Math.log10(flux),
    snr: flux / err,
    fwhm: 3,
    aperture_radius: 5,
    aperture_pixels: 78,
    aperture_area: 78.5,
    bg_mean: 10,
    bg_sigma: 1,
    bg_pixels: 300,
    saturated: false,
    saturation_source: "none",
    n_masked: 0,
    n_saturated: 0,
    err_used: false,
    aperture_correction: null,
    flux_total: null,
    plateau_radius: null,
    flux_jy: null,
    flux_err_jy: null,
    mag_ab: null,
    mag_ab_err: null,
    mag_ab_total: null,
    st_mag: null,
    sky_inner: 10,
    sky_outer: 15,
    growth_curve: [],
    ee50_radius: null,
    ee80_radius: null,
    n_nonfinite: 0,
    ee_in_aperture: null,
    warnings: [],
    ...extra,
  };
}

function frame(index: number, targets: (StarPhotometry | null)[], extra: Partial<TimeSeriesFrame> = {}): TimeSeriesFrame {
  return {
    index,
    path: `/run/f${index}.fits`,
    file_name: `f${index}.fits`,
    jd_mid: 2460310.5 + index / 1440,
    time_source: "DATE-OBS + EXPTIME/2 (60 s)",
    exptime: 60,
    filter: "V",
    airmass: 1.2,
    geometry: null,
    offset: index === 0 ? null : { dx: 0, dy: 0, confidence: 20, registered: true },
    photcal_label: null,
    targets,
    errors: targets.map(() => null),
    skipped: null,
    ...extra,
  };
}

function geometry(extra: Partial<FrameGeometry> = {}): FrameGeometry {
  return {
    jd_utc: 2460310.5,
    jd_tt: 2460310.5008,
    jd_tdb: 2460310.5008,
    bjd_tdb: 2460310.5041234,
    hjd_utc: 2460310.5033211,
    bjd_source: "computed",
    lst_deg: 100,
    hour_angle_deg: 10,
    altitude_deg: 65.4321,
    azimuth_deg: 200,
    airmass_computed: 1.09876,
    airmass_formula: "Kasten & Young 1989",
    parallactic_angle_deg: -23.456,
    sun_altitude_deg: -30,
    moon_altitude_deg: 5,
    moon_illumination: 0.5,
    moon_separation_deg: 80,
    time_scale_notes: [],
    ...extra,
  };
}

function result(frames: TimeSeriesFrame[]): TimeSeriesResult {
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
    warnings: [],
    area_mixed: false,
    geometry_target: null,
    geometry_notes: [],
    elapsed_ms: 5,
  };
}

const T = star(10, 10, 500, 5);
const C1 = star(40, 40, 1000, 10);
const C2 = star(70, 70, 1000, 10);

const DIFF_MAG = 1.505149978319906;
const DIFF_ERR = 0.013297055068698485;

function plainSeries(extra: Partial<TimeSeriesFrame> = {}): TimeSeriesResult {
  return result([0, 1, 2].map((i) => frame(i, [T, C1, C2], extra)));
}

function flaggedSeries(): TimeSeriesResult {
  return result([
    frame(0, [T, C1, C2]),
    frame(1, [null, null, null], { skipped: "dimensions 10 x 10 differ from the reference 20 x 20" }),
    frame(2, [star(10, 10, 500, 5, { saturated: true }), C1, C2]),
    frame(3, [T, star(40, 40, 1000, 10, { saturated: true }), C2]),
    frame(4, [T, C1, C2], { offset: { dx: 37.2, dy: -51.9, confidence: 1.1, registered: false } }),
    frame(5, [T, null, C2], { errors: [null, "lies outside the 256 x 256 image", null] }),
  ]);
}

const TABLE: LightCurveTableInput = {
  columns: [
    { name: "bjd_tdb", unit: "d", values: [2460310.5, null, 2460310.52, 2460310.53] },
    { name: "diff_mag", unit: "mag", values: [1.5, 1.6, null, 1.7] },
    { name: "diff_err", unit: null, values: [0.01, 0.01, 0.01, null] },
    { name: "file", unit: null, values: null },
  ],
};

const MAPPING: LightCurveTableMapping = {
  path: "C:/d/run_lightcurve.csv",
  timeColumn: "bjd_tdb",
  timeScale: "bjd_tdb",
  valueColumn: "diff_mag",
  quantity: "diff_mag",
  errColumn: "diff_err",
};

const QUICK_LOOK: LightCurveQuickLook = {
  docId: "target",
  label: "target T",
  source: { kind: "frames", reference: "f0.fits", target: "T", comps: ["C1"] },
  quantity: "diff_mag",
  unit: "mag",
  timeScale: "bjd_tdb",
  timeName: "BJD_TDB",
  nPoints: 120,
  window: { start: 2460310.61, end: 2460310.65 },
  marginDays: 0.005,
  baselineKind: "linear",
  depth: 0.0123,
  errScatter: 0.0004,
  errFormal: 0.0003,
  err: 0.0004,
  nIn: 40,
  nOut: 70,
  scatterPpm: 950,
  formalPpm: 700,
  scatterRatio: 1.357,
  binned: null,
  nExcluded: 2,
  nClipped: 1,
  notes: ["2 of 40 in-transit points are flagged: saturated"],
};

describe("light-curve documents from an image series", () => {
  it("a frame series becomes a differential-magnitude document on the header time axis", () => {
    const res = plainSeries();
    const doc = lightCurveDocFromFrames(res, 0, [1, 2], "jd");
    expect(doc.id).toBe("target");
    expect(doc.label).toBe("target T");
    expect(doc.quantity).toBe("diff_mag");
    expect(doc.unit).toBe("mag");
    expect(doc.timeScale).toBe("jd_header");
    expect(doc.time).toEqual([2460310.5, 2460310.5 + 1 / 1440, 2460310.5 + 2 / 1440]);
    expect(doc.value).toHaveLength(3);
    expect(doc.err).toHaveLength(3);
    doc.value.forEach((v) => expect(v).toBeCloseTo(DIFF_MAG, 12));
    doc.err.forEach((e) => expect(e).toBeCloseTo(DIFF_ERR, 12));
    expect(doc.pointRef).toEqual([0, 1, 2]);
    expect(doc.flags).toEqual([[], [], []]);
    expect(doc.magErrFactor).toBe(1.0857);
    expect(doc.magErrFactor).toBe(MAG_ERR_FACTOR);
    expect(doc.source).toEqual({ kind: "frames", reference: "f0.fits", target: "T", comps: ["C1", "C2"] });
    expect(doc.notes).toEqual([
      "differential magnitude: T minus the sum of 2 comparison stars (C1, C2)",
      "time: DATE-OBS + EXPTIME/2 (60 s)",
    ]);
    expect("overlay" in doc).toBe(false);
    const single = lightCurveDocFromFrames(res, 0, [0, 1], "jd", { id: "t1", label: "T against C1", overlay: "ensembles" });
    expect(single.id).toBe("t1");
    expect(single.label).toBe("T against C1");
    expect(single.overlay).toBe("ensembles");
    expect(single.source).toEqual({ kind: "frames", reference: "f0.fits", target: "T", comps: ["C1"] });
    expect(single.notes[0]).toBe("differential magnitude: T minus the sum of 1 comparison star (C1)");
    expect(frameGroupKey(res.reference_path)).toBe("frames:/run/f0.fits");
  });

  it("the axis label of a frame document is the one timeAxisLabel prints", () => {
    const res = plainSeries();
    const doc = lightCurveDocFromFrames(res, 0, [1, 2], "jd");
    expect(timeOffset(doc)).toBe(2460310);
    expect(docTimeLabel(doc, timeOffset(doc))).toBe(timeAxisLabel("jd", 2460310, res.frames));
    expect(docTimeLabel(doc, timeOffset(doc))).toBe("JD (header time, not barycentric) - 2460310");

    const withBjd = plainSeries({ geometry: geometry() });
    const bjd = lightCurveDocFromFrames(withBjd, 0, [1, 2], "bjd");
    expect(bjd.timeScale).toBe("bjd_tdb");
    expect(bjd.time[0]).toBe(2460310.5041234);
    expect(bjd.timeLabel).toBe("BJD_TDB");
    expect(docTimeLabel(bjd, timeOffset(bjd))).toBe(timeAxisLabel("bjd", 2460310, withBjd.frames));
    expect(docTimeLabel(bjd, timeOffset(bjd))).toBe("BJD_TDB - 2460310");

    const index = lightCurveDocFromFrames(res, 0, [1, 2], "index");
    expect(index.timeScale).toBe("index");
    expect(index.time).toEqual([0, 1, 2]);
    expect(timeOffset(index)).toBe(0);
    expect(index.timeLabel).toBe("frame index");
    expect(docTimeLabel(index, timeOffset(index))).toBe("frame index");
  });

  it("a header time that is barycentric keeps the wording of wave 0b", () => {
    const res = plainSeries({ geometry: geometry({ bjd_source: "header", hjd_utc: null }) });
    const doc = lightCurveDocFromFrames(res, 0, [1, 2], "jd");
    expect(doc.timeScale).toBe("jd_header");
    expect(doc.timeLabel).toBe("JD (header time, barycentric)");
    expect(docTimeLabel(doc, 2460310)).toBe(timeAxisLabel("jd", 2460310, res.frames));
    expect(docTimeLabel(doc, 2460310)).toBe("JD (header time, barycentric) - 2460310");

    const jwst = plainSeries({ geometry: geometry({ bjd_source: "TDB-MID", hjd_utc: null }) });
    const other = lightCurveDocFromFrames(jwst, 0, [1, 2], "jd");
    expect(docTimeLabel(other, 2460310)).toBe("JD (header time, not barycentric) - 2460310");
  });

  it("the BJD axis falls back to the header time when one frame has no BJD", () => {
    const res = result([
      frame(0, [T, C1, C2], { geometry: geometry() }),
      frame(1, [T, C1, C2], { geometry: geometry({ bjd_tdb: null }) }),
      frame(2, [T, C1, C2], { geometry: geometry() }),
    ]);
    const doc = lightCurveDocFromFrames(res, 0, [1, 2], "bjd");
    expect(doc.timeScale).toBe("jd_header");
    expect(doc.time[1]).toBe(2460310.5 + 1 / 1440);
    expect(rawFluxDocFromFrames(res, 1, "bjd").timeScale).toBe("jd_header");
  });

  it("frame flags name what the observer must check", () => {
    const res = flaggedSeries();
    const doc = lightCurveDocFromFrames(res, 0, [1, 2], "jd");
    expect(doc.flags).toEqual([[], ["skipped"], ["saturated"], ["comp saturated"], ["low-confidence drift"], ["measurement error"]]);
    expect(doc.value[0]).toBeCloseTo(DIFF_MAG, 12);
    expect(doc.value[1]).toBeNull();
    expect(doc.err[1]).toBeNull();
    expect(doc.value[5]).toBeNull();
    expect(doc.pointRef).toEqual([0, 1, 2, 3, 4, 5]);

    const withoutC1 = lightCurveDocFromFrames(res, 0, [2], "jd");
    expect(withoutC1.flags[3]).toEqual([]);
    expect(withoutC1.flags[5]).toEqual([]);
    expect(withoutC1.source).toEqual({ kind: "frames", reference: "f0.fits", target: "T", comps: ["C2"] });
    expect(lightCurveDocFromFrames(res, 0, [0, 1, 2], "jd").flags[2]).toEqual(["saturated"]);

    const everything = result([
      frame(0, [T, C1, C2]),
      frame(1, [star(10, 10, 500, 5, { saturated: true }), star(40, 40, 1000, 10, { saturated: true }), null], {
        offset: { dx: 3, dy: 4, confidence: 1.1, registered: false },
        errors: [null, null, "lies outside the 256 x 256 image"],
      }),
    ]);
    expect(lightCurveDocFromFrames(everything, 0, [0, 1, 2], "jd").flags[1]).toEqual([
      FLAG_SATURATED,
      FLAG_COMP_SATURATED,
      FLAG_LOW_CONFIDENCE_DRIFT,
      FLAG_MEASUREMENT_ERROR,
    ]);
    expect([FLAG_SKIPPED, FLAG_SATURATED, FLAG_COMP_SATURATED, FLAG_CENTROID_JUMP, FLAG_LOW_CONFIDENCE_DRIFT, FLAG_MEASUREMENT_ERROR]).toEqual([
      "skipped",
      "saturated",
      "comp saturated",
      "centroid jump",
      "low-confidence drift",
      "measurement error",
    ]);
    expect(EXCLUDABLE_FLAGS).toEqual(["saturated", "comp saturated", "centroid jump"]);
  });

  it("a centroid jump is flagged on the frame where the star moved", () => {
    const res = result([frame(0, [T, C1, C2]), frame(1, [T, C1, C2]), frame(2, [star(15, 10, 500, 5), C1, C2])]);
    const doc = lightCurveDocFromFrames(res, 0, [1, 2], "jd");
    expect(doc.flags[2]).toContain("centroid jump");
    expect(doc.flags[0]).not.toContain("centroid jump");
    expect(doc.flags[1]).not.toContain("centroid jump");
    expect(rawFluxDocFromFrames(res, 1, "jd").flags).toEqual([[], [], ["centroid jump"]]);
  });

  it("timeOffset ignores skipped frames and is zero for an index axis", () => {
    const res = result([
      frame(0, [null, null, null], { skipped: "failed to load", jd_mid: 2460309.9 }),
      frame(1, [T, C1, C2], { jd_mid: 2460310.5007 }),
      frame(2, [T, C1, C2], { jd_mid: 2460310.5014 }),
    ]);
    const doc = lightCurveDocFromFrames(res, 0, [1, 2], "jd");
    expect(doc.time).toEqual([2460309.9, 2460310.5007, 2460310.5014]);
    expect(timeOffset(doc)).toBe(2460310);
    expect(timeOffset(lightCurveDocFromFrames(res, 0, [1, 2], "index"))).toBe(0);
    expect(timeOffset({ timeScale: "index", time: [7, 8], flags: [[], []] })).toBe(0);
    expect(timeOffset({ timeScale: "bjd_tdb", time: [NaN, 2460311.25], flags: [[], []] })).toBe(2460311);
    expect(timeOffset({ timeScale: "bjd_tdb", time: [NaN], flags: [[]] })).toBe(0);
    expect(timeOffset({ timeScale: "bmjd_tdb", time: [], flags: [] })).toBe(0);
  });

  it("a raw flux document carries the net flux of one star", () => {
    const res = plainSeries();
    const doc = rawFluxDocFromFrames(res, 1, "jd", { overlay: "raw comps" });
    expect(doc.id).toBe("raw:1");
    expect(doc.label).toBe("C1");
    expect(doc.quantity).toBe("flux");
    expect(doc.unit).toBeNull();
    expect(doc.timeScale).toBe("jd_header");
    expect(doc.timeLabel).toBe("JD (header time, not barycentric)");
    expect(doc.time).toEqual([2460310.5, 2460310.5 + 1 / 1440, 2460310.5 + 2 / 1440]);
    expect(doc.value).toEqual([1000, 1000, 1000]);
    expect(doc.err).toEqual([10, 10, 10]);
    expect(doc.pointRef).toEqual([0, 1, 2]);
    expect(doc.flags).toEqual([[], [], []]);
    expect(doc.overlay).toBe("raw comps");
    expect(doc.source).toEqual({ kind: "frames", reference: "f0.fits", target: "C1", comps: [] });
    expect(doc.notes).toEqual(["net aperture flux of C1 in the image unit summed over the aperture"]);
    expect("magErrFactor" in doc).toBe(false);
    expect("overlay" in rawFluxDocFromFrames(res, 1, "jd")).toBe(false);

    const flagged = rawFluxDocFromFrames(flaggedSeries(), 1, "jd", { id: "c1", label: "comparison C1" });
    expect(flagged.id).toBe("c1");
    expect(flagged.label).toBe("comparison C1");
    expect(flagged.flags).toEqual([[], ["skipped"], [], ["saturated"], ["low-confidence drift"], []]);
    expect(flagged.value).toEqual([1000, null, 1000, 1000, 1000, null]);
    expect(flagged.err).toEqual([10, null, 10, 10, 10, null]);
    const nonFinite = rawFluxDocFromFrames(result([frame(0, [T, star(40, 40, NaN, Infinity), C2])]), 1, "jd");
    expect(nonFinite.value).toEqual([null]);
    expect(nonFinite.err).toEqual([null]);
  });
});

describe("time scales of a light-curve document", () => {
  it("timeScaleLabel names every scale and drops a zero offset", () => {
    expect(timeScaleLabel("bmjd_tdb", 60061)).toBe("BMJD (TDB) - 60061");
    expect(timeScaleLabel("bjd_tdb", 2460310)).toBe("BJD_TDB - 2460310");
    expect(timeScaleLabel("hjd_utc", 2460310)).toBe("HJD (UTC) - 2460310");
    expect(timeScaleLabel("jd_utc", 2460310)).toBe("JD (UTC) - 2460310");
    expect(timeScaleLabel("jd_header", 2460310)).toBe("JD (header time, not barycentric) - 2460310");
    expect(timeScaleLabel("mjd_utc", 60061)).toBe("MJD (UTC) - 60061");
    expect(timeScaleLabel("index", 5)).toBe("frame index");
    expect(timeScaleLabel("bjd_tdb", 0)).toBe("BJD_TDB");
    expect(Object.keys(TIME_SCALE_NAMES)).toHaveLength(7);
    expect(docTimeLabel({ timeScale: "index", timeLabel: "integration number" }, 0)).toBe("integration number");
    expect(docTimeLabel({ timeScale: "index", timeLabel: "integration number" }, 5)).toBe("integration number");
    expect(docTimeLabel({ timeScale: "bmjd_tdb" }, 60061)).toBe("BMJD (TDB) - 60061");
  });

  it("convertBarycentric adds or removes 2400000.5 and nothing else", () => {
    const doc: LightCurveDoc = {
      id: "white",
      label: "white light",
      quantity: "flux",
      unit: "Jy",
      timeScale: "bmjd_tdb",
      time: [60061.5, 60061.75],
      value: [0.164, null],
      err: [0.0001, null],
      pointRef: [1, 2],
      flags: [[], ["pixel loss"]],
      source: { kind: "tso", paths: ["a_x1dints.fits"], detector: "NRS1", sporder: null, band_um: null, mode: "masked" },
      notes: ["a note"],
    };
    expect(BMJD_OFFSET).toBe(2400000.5);
    const bjd = convertBarycentric(doc, "bjd_tdb");
    expect(bjd).toEqual({ ...doc, timeScale: "bjd_tdb", time: [2460062, 2460062.25] });
    expect(bjd.value).toBe(doc.value);
    expect(bjd.err).toBe(doc.err);
    expect(bjd.pointRef).toBe(doc.pointRef);
    expect(bjd.flags).toBe(doc.flags);
    expect(doc.time).toEqual([60061.5, 60061.75]);
    expect(convertBarycentric(bjd, "bmjd_tdb")).toEqual(doc);
    expect(convertBarycentric(doc, "bmjd_tdb")).toBe(doc);
    expect(convertBarycentric(bjd, "bjd_tdb")).toBe(bjd);
    const utc: LightCurveDoc = { ...doc, timeScale: "mjd_utc" };
    expect(convertBarycentric(utc, "bjd_tdb")).toBe(utc);
    expect(convertBarycentric(utc, "bmjd_tdb")).toBe(utc);
  });
});

describe("light-curve documents from a saved table", () => {
  it("a table becomes a document through an explicit column mapping", () => {
    const doc = lightCurveDocFromTable(TABLE, MAPPING);
    expect(doc).toEqual({
      id: "diff_mag",
      label: "diff_mag (run_lightcurve.csv)",
      quantity: "diff_mag",
      unit: "mag",
      timeScale: "bjd_tdb",
      time: [2460310.5, 2460310.53],
      value: [1.5, 1.7],
      err: [0.01, null],
      pointRef: [0, 3],
      flags: [[], []],
      source: { kind: "table", path: "C:/d/run_lightcurve.csv", time_column: "bjd_tdb", value_column: "diff_mag" },
      notes: ["2 of 4 rows kept (2 without a finite time or value)", "errors from column diff_err"],
    });
    expect(tableGroupKey(MAPPING.path)).toBe("table:C:/d/run_lightcurve.csv");

    const complete: LightCurveTableInput = {
      columns: [
        { name: "t", unit: null, values: [1, 2, NaN] },
        { name: "f", unit: null, values: [0.5, 0.25, 0.125] },
      ],
    };
    const bare = lightCurveDocFromTable(complete, {
      path: "curve.ecsv",
      timeColumn: "t",
      timeScale: "index",
      valueColumn: "f",
      quantity: "rel_flux",
      label: "my curve",
    });
    expect(bare.label).toBe("my curve");
    expect(bare.unit).toBeNull();
    expect(bare.time).toEqual([1, 2]);
    expect(bare.err).toEqual([null, null]);
    expect(bare.notes).toEqual(["2 of 3 rows kept (1 without a finite time or value)"]);
    const whole = lightCurveDocFromTable({ columns: [complete.columns[1], { name: "t", unit: null, values: [1, 2, 3] }] }, {
      path: "curve.ecsv",
      timeColumn: "t",
      timeScale: "index",
      valueColumn: "f",
      quantity: "rel_flux",
      errColumn: null,
    });
    expect(whole.notes).toEqual([]);
    expect(whole.pointRef).toEqual([0, 1, 2]);
  });

  it("a table mapping that names a missing or a text column is refused with the column named", () => {
    expect(() => lightCurveDocFromTable(TABLE, { ...MAPPING, timeColumn: "mjd" })).toThrow(
      new Error("column 'mjd' is not in the table (columns: bjd_tdb, diff_mag, diff_err, file)"),
    );
    expect(() => lightCurveDocFromTable(TABLE, { ...MAPPING, valueColumn: "file" })).toThrow(new Error("column 'file' holds text, not numbers"));
    expect(() => lightCurveDocFromTable(TABLE, { ...MAPPING, errColumn: "sigma" })).toThrow(
      new Error("column 'sigma' is not in the table (columns: bjd_tdb, diff_mag, diff_err, file)"),
    );
    const found = lightCurveDocFromTable(TABLE, { ...MAPPING, timeColumn: "BJD_TDB" });
    expect(found.time).toEqual([2460310.5, 2460310.53]);
    const twins: LightCurveTableInput = {
      columns: [
        { name: "FLUX", unit: "Jy", values: [9, 9] },
        { name: "flux", unit: "mJy", values: [1, 2] },
        { name: "time", unit: "d", values: [0, 1] },
      ],
    };
    const exact = lightCurveDocFromTable(twins, { path: "a.csv", timeColumn: "time", timeScale: "mjd_utc", valueColumn: "flux", quantity: "flux" });
    expect(exact.value).toEqual([1, 2]);
    expect(exact.unit).toBe("mJy");
  });
});

describe("the quick-look log entry", () => {
  it("the quick-look entry records the depth, its two errors and the windows", () => {
    const entry = quickLookEntry(QUICK_LOOK);
    expect(entry.kind).toBe("time_series");
    expect(entry.file).toBe("f0.fits");
    expect(entry.image).toBe("120 points");
    expect(entry.dq).toBe("not_applied");
    expect(entry.unit).toBe("fraction");
    expect(Object.keys(entry.params)).toEqual([
      "series",
      "quantity",
      "time_scale",
      "window_start",
      "window_end",
      "baseline_margin_days",
      "baseline_kind",
      "binned",
      "n_excluded",
      "n_clipped",
    ]);
    expect(entry.params).toEqual({
      series: "target T",
      quantity: "diff_mag",
      time_scale: "BJD_TDB",
      window_start: 2460310.61,
      window_end: 2460310.65,
      baseline_margin_days: 0.005,
      baseline_kind: "linear",
      binned: null,
      n_excluded: 2,
      n_clipped: 1,
    });
    expect(Object.keys(entry.values)).toEqual([
      "box_depth",
      "box_depth_err",
      "box_depth_err_scatter",
      "box_depth_err_formal",
      "n_in",
      "n_out",
      "oot_scatter_ppm",
      "oot_formal_ppm",
      "oot_scatter_ratio",
    ]);
    expect(entry.values).toEqual({
      box_depth: 0.0123,
      box_depth_err: 0.0004,
      box_depth_err_scatter: 0.0004,
      box_depth_err_formal: 0.0003,
      n_in: 40,
      n_out: 70,
      oot_scatter_ppm: 950,
      oot_formal_ppm: 700,
      oot_scatter_ratio: 1.357,
    });
    expect(entry.notes).toEqual([
      QUICK_LOOK_LABEL,
      "depth D = 1 - F_in / F_out; the error is the larger of the scatter and the formal estimate",
      "times in days, BJD_TDB",
      "2 of 40 in-transit points are flagged: saturated",
    ]);
    expect(JSON.parse(entry.source)).toEqual({ kind: "light_curve", doc: "target T", from: QUICK_LOOK.source });
    const formalLarger = quickLookEntry({ ...QUICK_LOOK, errFormal: 0.0009, err: 0.0009 }).values;
    expect(formalLarger.box_depth_err).toBe(0.0009);
    expect(formalLarger.box_depth_err_scatter).toBe(0.0004);
    expect(formalLarger.box_depth_err_formal).toBe(0.0009);

    const fromTable = quickLookEntry({
      ...QUICK_LOOK,
      source: { kind: "table", path: "C:/d/run_lightcurve.csv", time_column: "bjd_tdb", value_column: "diff_mag" },
      binned: "5 points",
    });
    expect(fromTable.file).toBe("run_lightcurve.csv");
    expect(fromTable.params.binned).toBe("5 points");
    const tso = { kind: "tso", detector: "NRS1", sporder: null, band_um: null, mode: "masked" } as const;
    expect(quickLookEntry({ ...QUICK_LOOK, source: { ...tso, paths: ["C:/d/jw_x1dints.fits", "C:/d/b.fits"] } }).file).toBe("jw_x1dints.fits");
    expect(quickLookEntry({ ...QUICK_LOOK, source: { ...tso, paths: [] } }).file).toBe("");
  });

  it("the quick-look entry passes through the measurement log as any other entry", () => {
    const entry = { ...quickLookEntry(QUICK_LOOK), id: "m-1", timestamp_utc: "2026-11-13T05:00:00Z" };
    const [header, row] = measurementLogCsv([entry]).split(/\r?\n/);
    const columns = header.split(",");
    expect(columns).toContain("p_window_start");
    expect(columns).toContain("v_box_depth");
    expect(columns.slice(-9)).toEqual(Object.keys(entry.values).map((key) => `v_${key}`));
    expect(row.endsWith(",2,1,0.0123,0.0004,0.0004,0.0003,40,70,950,700,1.357")).toBe(true);
    expect(row).toContain(QUICK_LOOK_LABEL);
  });
});

const PERIODOGRAM: PeriodogramResult = {
  frequency: [0.3315762781484169, 0.3453919564046009, 0.359207634660785],
  power: [0.61, 0.8675526073476112, 0.58],
  best_frequency: 0.3453919564046009,
  best_period: 2.895261402175141,
  best_power: 0.8675526073476112,
  n_points: 40,
  n_dropped: 0,
  n_frequencies: 500,
  baseline_days: 14.476307010875704,
  frequency_step: 0.013815678256184037,
  normalization: "standard",
  weighted: true,
  false_alarm_probability: 6.64610294915559e-14,
  fap_method: "baluev",
  notes: ["note a", "note b"],
  elapsed_ms: 3,
};

const PERIOD_DOC: LightCurveDoc = {
  id: "target",
  label: "target T",
  quantity: "diff_mag",
  unit: "mag",
  timeScale: "bjd_tdb",
  time: [2460310.5, 2460310.6],
  value: [1.5, 1.6],
  err: [0.01, 0.01],
  pointRef: [0, 1],
  flags: [[], []],
  source: { kind: "frames", reference: "f0.fits", target: "T", comps: ["C1"] },
  notes: [],
};

describe("the period-search log entry", () => {
  it("the period-search entry records the peak, its false-alarm probability and the grid", () => {
    const fold = { periodDays: 2.895261402175141, t0: 2460310.6 };
    const entry = periodogramEntry({
      doc: PERIOD_DOC,
      valueName: "normalised flux",
      result: PERIODOGRAM,
      minFrequency: null,
      maxFrequency: 2,
      fold,
    });
    expect(entry.kind).toBe("periodogram");
    expect(entry.file).toBe("f0.fits");
    expect(entry.image).toBe("40 points");
    expect(entry.dq).toBe("not_applied");
    expect(entry.unit).toBe("d");
    expect(JSON.parse(entry.source)).toEqual({ kind: "light_curve", doc: "target T", from: PERIOD_DOC.source });
    expect(Object.keys(entry.params)).toEqual([
      "series",
      "quantity",
      "time_scale",
      "min_frequency_per_day",
      "max_frequency_per_day",
      "weighted",
      "normalization",
      "fap_method",
      "fold_period_days",
      "fold_epoch",
    ]);
    expect(entry.params).toEqual({
      series: "target T",
      quantity: "normalised flux",
      time_scale: "BJD_TDB",
      min_frequency_per_day: null,
      max_frequency_per_day: 2,
      weighted: true,
      normalization: "standard",
      fap_method: "baluev",
      fold_period_days: 2.895261402175141,
      fold_epoch: 2460310.6,
    });
    expect(Object.keys(entry.values)).toEqual([
      "best_period_days",
      "best_frequency_per_day",
      "best_power",
      "false_alarm_probability",
      "period_step_days",
      "n_points",
      "n_dropped",
      "n_frequencies",
      "frequency_step_per_day",
      "baseline_days",
    ]);
    expect(entry.values).toEqual({
      best_period_days: 2.895261402175141,
      best_frequency_per_day: 0.3453919564046009,
      best_power: 0.8675526073476112,
      false_alarm_probability: 6.64610294915559e-14,
      period_step_days: periodStep(2.895261402175141, 0.013815678256184037),
      n_points: 40,
      n_dropped: 0,
      n_frequencies: 500,
      frequency_step_per_day: 0.013815678256184037,
      baseline_days: 14.476307010875704,
    });
    expect(entry.notes).toEqual(["note a", "note b", "periods in days, frequencies in cycles per day"]);

    const unfolded = periodogramEntry({
      doc: { ...PERIOD_DOC, timeLabel: "BJD (TDB) from the header", source: { kind: "table", path: "C:/d/run.csv", time_column: "t", value_column: "m" } },
      valueName: "diff mag (1 comp)",
      result: { ...PERIODOGRAM, false_alarm_probability: null, weighted: false, n_points: 38, n_dropped: 2 },
      minFrequency: 0.1,
      maxFrequency: null,
      fold: null,
    });
    expect(unfolded.file).toBe("run.csv");
    expect(unfolded.image).toBe("38 points");
    expect(unfolded.values.n_points).toBe(38);
    expect(unfolded.values.n_dropped).toBe(2);
    expect(unfolded.params.fold_period_days).toBeNull();
    expect(unfolded.params.fold_epoch).toBeNull();
    expect(unfolded.params.min_frequency_per_day).toBe(0.1);
    expect(unfolded.params.max_frequency_per_day).toBeNull();
    expect(unfolded.params.weighted).toBe(false);
    expect(unfolded.params.quantity).toBe("diff mag (1 comp)");
    expect(unfolded.params.time_scale).toBe("BJD (TDB) from the header");
    expect(unfolded.values.false_alarm_probability).toBeNull();
    const tso = { kind: "tso", detector: "NRS1", sporder: null, band_um: null, mode: "masked" } as const;
    const fromTso = periodogramEntry({
      doc: { ...PERIOD_DOC, source: { ...tso, paths: ["C:/d/jw_x1dints.fits", "C:/d/b.fits"] } },
      valueName: "flux",
      result: PERIODOGRAM,
      minFrequency: null,
      maxFrequency: null,
      fold: null,
    });
    expect(fromTso.file).toBe("jw_x1dints.fits");
  });
});
