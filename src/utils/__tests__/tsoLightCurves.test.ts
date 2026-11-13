import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { DEFAULT_LIGHT_CURVE_VIEW, type LightCurveViewState } from "../../shared/types/lightCurve";
import type {
  SeriesKey,
  TsoCurve,
  TsoCurveMode,
  TsoLightCurves,
  TsoLimitsOrigin,
  TsoTimeScale,
} from "../../shared/types/tso";
import { QUICK_LOOK_LABEL } from "../lightCurveAnalysis";
import {
  BINS_OVERLAY,
  DEFAULT_REJECT_SIGMA,
  INDEX_TIME_LABEL,
  MAX_TSO_BINS,
  PIXEL_LOSS_FLAG,
  PIXEL_LOSS_RATIO,
  WHITE_DOC_ID,
  bandText,
  binDocId,
  binsFromEdgesText,
  depthSpectrum,
  equalWidthBins,
  lightCurveDocsFromTso,
  pixelLossFlags,
  tsoGroupKey,
  tsoTimeName,
  windowPixelRatio,
  type DepthRow,
} from "../tsoLightCurves";

interface WhiteFixture {
  series: SeriesKey;
  mode: TsoCurveMode;
  limits_um: [number, number];
  limits_origin: TsoLimitsOrigin;
  time_scale: TsoTimeScale;
  flux_unit: string;
  int_num: number[];
  time: number[];
  mjd_utc: number[];
  flux: number[];
  flux_err: number[];
  n_used: number[];
  npixels_mean: number[];
}

interface BinsFluxFixture {
  edges_um: number[];
  flux: number[][];
}

interface BinsErrFixture {
  edges_um: number[];
  flux_err: number[][];
}

const NO_WINDOW_NOTE = "set a transit window in the light-curve view to get depths";
const DEPTH_RULE_NOTE = "depth D = 1 - F_in / F_out per bin; the error is the larger of the scatter and the formal estimate";
const GOLDEN_TOLERANCE = 1e-10;

function curve(flux: (number | null)[], extra: Partial<TsoCurve> = {}): TsoCurve {
  return {
    band_um: [1, 2],
    limits_origin: "open",
    flux,
    flux_err: flux.map(() => 0.5),
    n_used: flux.map(() => 5),
    npixels_mean: flux.map(() => 7),
    ...extra,
  };
}

function result(extra: Partial<TsoLightCurves> = {}): TsoLightCurves {
  const time = [60000.1, 60000.2, 60000.3, 60000.4];
  return {
    set_id: "abc",
    label: "NRS1",
    series: { detector: "NRS1", sporder: 0 },
    mode: "pipeline",
    time_scale: "bmjd_tdb",
    time,
    mjd_utc: time.map((t) => t - 0.002),
    int_num: [1, 2, 3, 4],
    row_index: [0, 1, 2, 3],
    flux_unit: "Jy",
    white: curve([10, 11, 12, 13]),
    bins: [],
    dropped: { nan_time: 0, repeated_time: 0 },
    rejected_elements: [],
    notes: [],
    warnings: [],
    ...extra,
  };
}

function transitCurves(whiteExtra: Partial<TsoCurve> = {}, binExtra: Partial<TsoCurve> = {}): TsoLightCurves {
  const time = [1, 2, 3, 4, 5, 6, 7, 8];
  const flux_err = time.map(() => 0.1);
  return result({
    time_scale: "index",
    time,
    mjd_utc: time.map(() => null),
    int_num: time,
    row_index: time.map((_, i) => i),
    white: curve([10, 10, 10, 5, 5, 10, 10, 10], { flux_err, ...whiteExtra }),
    bins: [curve([4, 4, 4, 3, 3, 4, 4, 4], { band_um: [1, 1.5], limits_origin: "user", flux_err, ...binExtra })],
  });
}

function fixture<T>(name: string): T {
  const path = fileURLToPath(new URL(`../../../src-tauri/tests/fixtures/tso/${name}`, import.meta.url));
  return JSON.parse(readFileSync(path, "utf8")) as T;
}

function e1(): TsoLightCurves {
  const white = fixture<WhiteFixture>("tso_quicklook_e1_white.json");
  const binsFlux = fixture<BinsFluxFixture>("tso_quicklook_e1_bins_flux.json");
  const binsErr = fixture<BinsErrFixture>("tso_quicklook_e1_bins_err.json");
  return {
    set_id: "e1",
    label: white.series.detector,
    series: white.series,
    mode: white.mode,
    time_scale: white.time_scale,
    time: white.time,
    mjd_utc: white.mjd_utc,
    int_num: white.int_num,
    row_index: white.time.map((_, i) => i),
    flux_unit: white.flux_unit,
    white: {
      band_um: white.limits_um,
      limits_origin: white.limits_origin,
      flux: white.flux,
      flux_err: white.flux_err,
      n_used: white.n_used,
      npixels_mean: white.npixels_mean,
    },
    bins: binsFlux.flux.map((flux, k) => ({
      band_um: [binsFlux.edges_um[k], binsFlux.edges_um[k + 1]],
      limits_origin: "user",
      flux,
      flux_err: binsErr.flux_err[k],
      n_used: flux.map(() => 1),
      npixels_mean: flux.map(() => null),
    })),
    dropped: { nan_time: 0, repeated_time: 0 },
    rejected_elements: [],
    notes: [],
    warnings: [],
  };
}

function expectRelative(measured: number | null, expected: number): void {
  expect(measured).not.toBeNull();
  expect(Math.abs((measured as number) - expected)).toBeLessThanOrEqual(GOLDEN_TOLERANCE * Math.abs(expected));
}

describe("tsoLightCurves", () => {
  it("names the group, the bin documents, a band and a time scale", () => {
    expect(tsoGroupKey("4b385da81d5c70b8", { detector: "NRS1", sporder: 0 })).toBe("tso:4b385da81d5c70b8:NRS1:0");
    expect(tsoGroupKey("4b385da81d5c70b8", { detector: "NRS1", sporder: null })).toBe("tso:4b385da81d5c70b8:NRS1:");
    expect(tsoGroupKey("4b385da81d5c70b8", { detector: "NRS1", sporder: -1 })).toBe("tso:4b385da81d5c70b8:NRS1:-1");
    expect(binDocId(3)).toBe("bin:3");
    expect(bandText([0.6, 1])).toBe("0.6 to 1 um");
    expect(bandText([0.5530469695008425, 5.370352185327354])).toBe("0.553 to 5.37 um");
    expect(bandText([2.47, 3.94])).toBe("2.47 to 3.94 um");
    expect(tsoTimeName("bmjd_tdb")).toBe("BMJD (TDB)");
    expect(tsoTimeName("mjd_utc")).toBe("MJD (UTC)");
    expect(tsoTimeName("index")).toBe("integration number");
    expect([PIXEL_LOSS_FLAG, PIXEL_LOSS_RATIO, WHITE_DOC_ID, BINS_OVERLAY, INDEX_TIME_LABEL]).toEqual([
      "pixel loss",
      0.9,
      "white",
      "bins",
      "integration number",
    ]);
    expect([MAX_TSO_BINS, DEFAULT_REJECT_SIGMA]).toEqual([512, 100]);
  });

  it("turns the white light and the bins into documents of one group", () => {
    const curves = result({
      bins: [
        curve([1, 2, 3, 4], { band_um: [1, 1.5], limits_origin: "user" }),
        curve([5, 6, 7, 8], { band_um: [1.5, 2], limits_origin: "user" }),
      ],
    });
    const paths = ["C:/d/a_x1dints.fits"];
    const docs = lightCurveDocsFromTso(curves, paths);
    expect(docs.map((d) => d.id)).toEqual(["white", "bin:0", "bin:1"]);
    expect(docs.map((d) => d.label)).toEqual(["white light 1 to 2 um", "1 to 1.5 um", "1.5 to 2 um"]);
    expect(docs.map((d) => d.overlay)).toEqual([undefined, "bins", "bins"]);
    const [white, first, second] = docs;
    expect("overlay" in white).toBe(false);
    expect(white.quantity).toBe("flux");
    expect(white.unit).toBe("Jy");
    expect(white.timeScale).toBe("bmjd_tdb");
    expect("timeLabel" in white).toBe(false);
    expect(white.time).toEqual(curves.time);
    expect(white.time).not.toBe(curves.time);
    expect(white.value).toEqual([10, 11, 12, 13]);
    expect(white.err).toEqual([0.5, 0.5, 0.5, 0.5]);
    expect(white.pointRef).toEqual([1, 2, 3, 4]);
    expect(white.pointRef).not.toBe(curves.int_num);
    expect(white.flags).toEqual([[], [], [], []]);
    expect(white.source).toEqual({
      kind: "tso",
      paths: ["C:/d/a_x1dints.fits"],
      detector: "NRS1",
      sporder: 0,
      band_um: [1, 2],
      mode: "pipeline",
    });
    expect(white.source.kind === "tso" && white.source.paths).not.toBe(paths);
    expect(white.notes).toEqual(["pipeline mode: sum of FLUX over 1 to 2 um (open limits)", "5 elements summed per integration"]);
    expect(first.notes).toEqual([
      "pipeline mode: sum of FLUX over 1 to 1.5 um, upper edge excluded",
      "5 elements summed per integration",
    ]);
    expect(first.source.kind === "tso" && first.source.band_um).toEqual([1, 1.5]);
    expect(first.source.kind === "tso" && first.source.band_um).not.toBe(curves.bins[0].band_um);
    expect(first.value).toEqual([1, 2, 3, 4]);
    expect(first.time).toEqual(curves.time);
    expect(first.time).not.toBe(white.time);
    expect(second.value).toEqual([5, 6, 7, 8]);
    expect(second.source.kind === "tso" && second.source.band_um).toEqual([1.5, 2]);
    const masked = lightCurveDocsFromTso(result({ mode: "masked", white: curve([1, 2, 3, 4], { limits_origin: "file log" }) }), []);
    expect(masked[0].notes[0]).toBe("masked mode: sum of FLUX over 1 to 2 um (file log limits)");
    expect(masked[0].source.kind === "tso" && masked[0].source.mode).toBe("masked");
  });

  it("leaves integrations without elements empty and keeps only usable errors", () => {
    const [white] = lightCurveDocsFromTso(
      result({ white: curve([10, 0, null, 13], { n_used: [5, 0, 0, 3], flux_err: [0.5, 0, null, 0] }) }),
      [],
    );
    expect(white.value).toEqual([10, null, null, 13]);
    expect(white.err).toEqual([0.5, null, null, null]);
    expect(white.notes).toContain("0 to 5 elements summed per integration");
    expect(white.notes).toContain("2 integration(s) with no element summed are left empty");
    const [gaps] = lightCurveDocsFromTso(
      result({ white: curve([NaN, 11, Infinity, 13], { flux_err: [0.5, Infinity, 0.5, NaN] }) }),
      [],
    );
    expect(gaps.value).toEqual([null, 11, null, 13]);
    expect(gaps.err).toEqual([null, null, null, null]);
    expect(gaps.notes).toEqual(["pipeline mode: sum of FLUX over 1 to 2 um (open limits)", "5 elements summed per integration"]);
    const [empty] = lightCurveDocsFromTso(result({ time: [], mjd_utc: [], int_num: [], row_index: [], white: curve([]) }), []);
    expect([empty.time, empty.value, empty.err, empty.pointRef, empty.flags]).toEqual([[], [], [], [], []]);
    expect(empty.notes).toEqual(["pipeline mode: sum of FLUX over 1 to 2 um (open limits)"]);
  });

  it("names the index scale and an unknown unit", () => {
    const [white] = lightCurveDocsFromTso(result({ time_scale: "index", time: [1, 2, 3, 4], flux_unit: "unknown" }), []);
    expect(white.timeScale).toBe("index");
    expect(white.timeLabel).toBe("integration number");
    expect(white.unit).toBeNull();
  });

  it("flags the integrations whose mean NPIXELS falls below 0.9 of the median", () => {
    expect(pixelLossFlags([7, 7, 3, 7, 6.5, null])).toEqual([false, false, true, false, false, false]);
    expect(pixelLossFlags([null, null])).toEqual([false, false]);
    expect(pixelLossFlags([])).toEqual([]);
    expect(pixelLossFlags([10, 9, 8.99, NaN])).toEqual([false, false, false, false]);
    expect(pixelLossFlags([10, 10, 9, 8.5])).toEqual([false, false, false, true]);
    expect(pixelLossFlags([10, 10, 10, 9])).toEqual([false, false, false, false]);
    const [white] = lightCurveDocsFromTso(result({ white: curve([10, 11, 12, 13], { npixels_mean: [7, 7, 3, 7] }) }), []);
    expect(white.flags).toEqual([[], [], ["pixel loss"], []]);
    expect(white.notes).toContain("1 integration(s) flagged pixel loss: mean NPIXELS below 0.9 of its median");
    expect(white.value).toEqual([10, 11, 12, 13]);
  });

  it("builds equal bins whose last edge is the upper limit", () => {
    const bins = equalWidthBins(0.6, 5.3, 5);
    expect(bins).toHaveLength(5);
    expect(bins[0][0]).toBe(0.6);
    expect(bins[4][1]).toBe(5.3);
    bins.forEach(([lo, hi], k) => {
      expect(hi - lo).toBeCloseTo(0.94, 12);
      if (k > 0) expect(lo).toBe(bins[k - 1][1]);
    });
    expect(equalWidthBins(1, 2, 1)).toEqual([[1, 2]]);
    expect(equalWidthBins(2, 1, 3)).toEqual([]);
    expect(equalWidthBins(1, 1, 3)).toEqual([]);
    expect(equalWidthBins(1, 2, 0)).toEqual([]);
    expect(equalWidthBins(1, 2, 2.5)).toEqual([]);
    expect(equalWidthBins(NaN, 2, 2)).toEqual([]);
    expect(equalWidthBins(1, Infinity, 2)).toEqual([]);
    expect(equalWidthBins(1, 2, 513)).toEqual([]);
    const most = equalWidthBins(1, 2, 512);
    expect(most).toHaveLength(512);
    expect(most[511][1]).toBe(2);
    const uneven = equalWidthBins(2.47, 3.94, 19);
    expect(2.47 + 19 * ((3.94 - 2.47) / 19)).not.toBe(3.94);
    expect(uneven[18][1]).toBe(3.94);
    expect(uneven[18][0]).toBe(uneven[17][1]);
  });

  it("reads typed edges as contiguous bins and says what is wrong", () => {
    expect(binsFromEdgesText("0.6, 1.0, 2.0, 3.0, 4.0, 5.3")).toEqual({
      ok: true,
      bins: [
        [0.6, 1],
        [1, 2],
        [2, 3],
        [3, 4],
        [4, 5.3],
      ],
    });
    expect(binsFromEdgesText("0.6 1;2\n3")).toEqual({
      ok: true,
      bins: [
        [0.6, 1],
        [1, 2],
        [2, 3],
      ],
    });
    expect(binsFromEdgesText("  ")).toEqual({ ok: true, bins: [] });
    expect(binsFromEdgesText(", 1, 2,")).toEqual({ ok: true, bins: [[1, 2]] });
    expect(binsFromEdgesText("1")).toEqual({ ok: false, error: "two or more edges make a bin" });
    expect(binsFromEdgesText("1, x")).toEqual({ ok: false, error: "edge 'x' is not a number" });
    expect(binsFromEdgesText("1, Infinity")).toEqual({ ok: false, error: "edge 'Infinity' is not a number" });
    expect(binsFromEdgesText("1, 2, 2")).toEqual({ ok: false, error: "edges must increase: 2 is not above 2" });
    expect(binsFromEdgesText("3, 1")).toEqual({ ok: false, error: "edges must increase: 1 is not above 3" });
    expect(binsFromEdgesText("0,6 1,0")).toEqual({ ok: false, error: "edges must increase: 1 is not above 6" });
    expect(binsFromEdgesText("1.0 1.00")).toEqual({ ok: false, error: "edges must increase: 1.00 is not above 1.0" });
    const edges = (count: number) => Array.from({ length: count }, (_, k) => String(k + 1)).join(" ");
    expect(binsFromEdgesText(edges(514))).toEqual({ ok: false, error: "at most 512 bins" });
    const most = binsFromEdgesText(edges(513));
    expect(most.ok && most.bins.length).toBe(512);
  });

  it("measures the pixel ratio between the window and the baseline", () => {
    const time = [1, 2, 3, 4, 5, 6];
    const npixels = [7, 7, 3, 4, 7, 7];
    expect(windowPixelRatio(time, npixels, { start: 3, end: 4 }, 0)).toBe(3.5 / 7);
    expect(windowPixelRatio(time, npixels, { start: 3, end: 4 }, 1.5)).toBe(3.5 / 7);
    expect(windowPixelRatio(time, [7, 14, 3, 4, 14, 7], { start: 3, end: 4 }, 0)).toBe(3.5 / 10.5);
    expect(windowPixelRatio(time, [7, 14, 3, 4, 14, 7], { start: 3, end: 4 }, 1)).toBe(3.5 / 10.5);
    expect(windowPixelRatio(time, [7, 14, 3, 4, 14, 7], { start: 3, end: 4 }, 1.5)).toBe(3.5 / 7);
    expect(windowPixelRatio(time, [7, NaN, 3, null, 14, 7], { start: 3, end: 4 }, 0)).toBe(3 / (28 / 3));
    expect(windowPixelRatio(time, time.map(() => null), { start: 3, end: 4 }, 0)).toBeNull();
    expect(windowPixelRatio(time, npixels, { start: 3.2, end: 3.8 }, 0)).toBeNull();
    expect(windowPixelRatio(time, npixels, { start: 0, end: 9 }, 0)).toBeNull();
    expect(windowPixelRatio(time, npixels, { start: 3, end: 4 }, 5)).toBeNull();
    expect(windowPixelRatio(time, [0, 0, 3, 4, 0, 0], { start: 3, end: 4 }, 0)).toBeNull();
  });

  it("gives one depth per bin with the errors of the light-curve model", () => {
    const curves = transitCurves();
    const docs = lightCurveDocsFromTso(curves, []);
    const view: LightCurveViewState = { ...DEFAULT_LIGHT_CURVE_VIEW, window: { start: 4, end: 5 } };
    const spectrum = depthSpectrum(docs, view, curves);
    const formal = Math.sqrt((Math.sqrt(2 * 0.01) / 2 / 4) ** 2 + 0.75 ** 2 * (Math.sqrt(6 * 0.01) / 6 / 4) ** 2);
    expect(spectrum.white?.docId).toBe("white");
    expect(spectrum.white?.label).toBe("white light 1 to 2 um");
    expect(spectrum.white?.band).toEqual([1, 2]);
    expect(spectrum.white?.depth).toBeCloseTo(0.5, 12);
    expect(spectrum.rows).toHaveLength(1);
    const row = spectrum.rows[0];
    expect(row.docId).toBe("bin:0");
    expect(row.label).toBe("1 to 1.5 um");
    expect(row.band).toEqual([1, 1.5]);
    expect(row.band).not.toBe(curves.bins[0].band_um);
    expect(row.depth).toBeCloseTo(0.25, 12);
    expect(row.centre).toBe(1.25);
    expect(row.halfWidth).toBe(0.25);
    expect(row.nIn).toBe(2);
    expect(row.nOut).toBe(6);
    expect(row.errFormal).toBeCloseTo(formal, 10);
    expect(row.errScatter).toBe(0);
    expect(row.err).toBe(row.errFormal);
    expect(row.pixelRatio).toBe(1);
    expect(row.pixelLoss).toBe(false);
    expect(spectrum.window).toBe(view.window);
    expect(spectrum.marginDays).toBe(0);
    expect(spectrum.notes).toEqual([QUICK_LOOK_LABEL, DEPTH_RULE_NOTE]);
    expect(spectrum.warnings).toEqual([]);

    const withMargin = depthSpectrum(docs, { ...view, marginDays: 1.5 }, curves);
    expect(withMargin.marginDays).toBe(1.5);
    expect(withMargin.rows[0].nOut).toBe(4);
    expect(depthSpectrum(docs, { ...view, marginDays: NaN }, curves).marginDays).toBe(0);
    expect(depthSpectrum(docs, { ...view, marginDays: -1 }, curves).rows[0].nOut).toBe(6);
    const endless = depthSpectrum(docs, { ...view, marginDays: Infinity }, curves);
    expect(endless.marginDays).toBe(0);
    expect(endless.rows[0].pixelRatio).toBe(1);
    const onePoint = depthSpectrum(docs, { ...view, window: { start: 4, end: 4 } }, curves);
    expect(onePoint.window).toEqual({ start: 4, end: 4 });
    expect(onePoint.rows[0].nIn).toBe(1);

    const unset = depthSpectrum(docs, { ...view, window: null }, curves);
    const emptyRow = { depth: null, errScatter: null, errFormal: null, err: null, nIn: 0, nOut: 0, pixelRatio: null, pixelLoss: false };
    expect(unset.window).toBeNull();
    expect(unset.white).toMatchObject(emptyRow);
    expect(unset.rows).toHaveLength(1);
    expect(unset.rows[0]).toMatchObject({ ...emptyRow, docId: "bin:0", centre: 1.25, halfWidth: 0.25 });
    expect(unset.notes).toEqual([QUICK_LOOK_LABEL, DEPTH_RULE_NOTE, NO_WINDOW_NOTE]);
    expect(unset.warnings).toEqual([]);
    const reversed = depthSpectrum(docs, { ...view, window: { start: 5, end: 4 } }, curves);
    expect(reversed.window).toBeNull();
    expect(reversed.rows[0].depth).toBeNull();
    expect(reversed.notes).toContain(NO_WINDOW_NOTE);
    expect(depthSpectrum(docs, { ...view, window: { start: 4, end: Infinity } }, curves).window).toBeNull();

    const strangers = [{ ...docs[0], id: "whtlt:x" }, ...docs, { ...docs[1], id: "bin:5" }, { ...docs[1], id: "bin:" }];
    const skipped = depthSpectrum(strangers, view, curves);
    expect(skipped.white?.docId).toBe("white");
    expect(skipped.rows.map((r) => r.docId)).toEqual(["bin:0"]);
    const binsOnly = depthSpectrum([docs[1]], view, curves);
    expect(binsOnly.white).toBeNull();
    expect(binsOnly.rows).toHaveLength(1);
    expect(depthSpectrum([], view, curves)).toEqual({
      white: null,
      rows: [],
      window: view.window,
      marginDays: 0,
      notes: [QUICK_LOOK_LABEL, DEPTH_RULE_NOTE],
      warnings: [],
    });
  });

  it("warns when the window lost pixels", () => {
    const curves = transitCurves({ npixels_mean: [7, 7, 7, 3, 3, 7, 7, 7] }, { npixels_mean: [7, 7, 7, 6.5, 6.5, 7, 7, 7] });
    const docs = lightCurveDocsFromTso(curves, []);
    const view: LightCurveViewState = { ...DEFAULT_LIGHT_CURVE_VIEW, window: { start: 4, end: 5 } };
    const spectrum = depthSpectrum(docs, view, curves);
    expect(spectrum.white?.pixelRatio).toBe(3 / 7);
    expect(spectrum.white?.pixelLoss).toBe(true);
    expect(spectrum.white?.depth).toBeCloseTo(0.5, 12);
    expect(spectrum.white?.nIn).toBe(2);
    expect(spectrum.rows[0].pixelRatio).toBe(6.5 / 7);
    expect(spectrum.rows[0].pixelLoss).toBe(false);
    expect(spectrum.warnings).toEqual([
      "mean NPIXELS inside the window is below 0.9 of the baseline in 1 of 2 curves (lowest ratio 0.429): the extraction lost pixels during the event, so these depths are biased high",
    ]);
    const both = transitCurves({ npixels_mean: [7, 7, 7, 3, 3, 7, 7, 7] }, { npixels_mean: [7, 7, 7, 1.75, 1.75, 7, 7, 7] });
    expect(depthSpectrum(lightCurveDocsFromTso(both, []), view, both).warnings).toEqual([
      "mean NPIXELS inside the window is below 0.9 of the baseline in 2 of 2 curves (lowest ratio 0.25): the extraction lost pixels during the event, so these depths are biased high",
    ]);
    expect(depthSpectrum(docs, { ...view, window: null }, curves).warnings).toEqual([]);
  });

  it("reproduces the quick look of the real transit file, a pin of the file and not of the planet", () => {
    const curves = e1();
    expect(curves.time).toHaveLength(240);
    const docs = lightCurveDocsFromTso(curves, ["E1"]);
    const view: LightCurveViewState = {
      ...DEFAULT_LIGHT_CURVE_VIEW,
      window: { start: curves.time[182], end: curves.time[187] },
      marginDays: 0.0044,
    };
    const spectrum = depthSpectrum(docs, view, curves);
    const golden: [string, number, number, number][] = [
      ["white light 0.553 to 5.37 um", 0.9503918297804987, 0.006329466289158166, 0.00027501456081958594],
      ["0.6 to 1 um", 0.9673703022163279, 0.010016353251191705, 0.0007353683290367279],
      ["1 to 2 um", 0.983291587599211, 0.0048012563432253096, 0.0004283885822162704],
      ["2 to 3 um", 0.93598598141012, 0.0071442668865198, 0.0009430400711400589],
      ["3 to 4 um", 0.9694971412962083, 0.0030302124497207017, 0.00015469537502730567],
      ["4 to 5.3 um", 0.8731296866418552, 0.00809113864280491, 0.00041927623623093054],
    ];
    expect(spectrum.white).not.toBeNull();
    const measured = [spectrum.white as DepthRow, ...spectrum.rows];
    expect(measured.map((row) => row.label)).toEqual(golden.map(([label]) => label));
    golden.forEach(([, depth, errScatter, errFormal], k) => {
      const row = measured[k];
      expectRelative(row.depth, depth);
      expectRelative(row.errScatter, errScatter);
      expectRelative(row.errFormal, errFormal);
      expect(row.nIn).toBe(6);
      expect(row.nOut).toBe(210);
      expect(row.err).toBe(row.errScatter);
    });
    expect(spectrum.white?.pixelRatio).toBeCloseTo(0.43858712034718045, 10);
    expect(spectrum.white?.pixelLoss).toBe(true);
    expect(spectrum.rows.map((row) => row.pixelRatio)).toEqual([null, null, null, null, null]);
    expect(spectrum.rows.map((row) => row.pixelLoss)).toEqual([false, false, false, false, false]);
    expect(spectrum.warnings).toEqual([
      "mean NPIXELS inside the window is below 0.9 of the baseline in 1 of 1 curves (lowest ratio 0.439): the extraction lost pixels during the event, so these depths are biased high",
    ]);
    expect(spectrum.marginDays).toBe(0.0044);
    const white = docs[0];
    const flagged = white.pointRef.filter((_, i) => white.flags[i].includes("pixel loss"));
    expect(flagged).toEqual([181, 182, 183, 184, 185, 186, 187, 188, 189, 190]);
    expect(white.label).toBe("white light 0.553 to 5.37 um");
    expect(white.notes).toContain("10 integration(s) flagged pixel loss: mean NPIXELS below 0.9 of its median");
    expect(docs.slice(1).every((doc) => doc.flags.every((words) => words.length === 0))).toBe(true);
  });
});
