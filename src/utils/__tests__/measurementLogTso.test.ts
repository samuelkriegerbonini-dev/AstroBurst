import { describe, expect, it } from "vitest";
import type { TsoCurve, TsoLightCurves } from "../../shared/types/tso";
import { tsoDepthSpectrumEntries, tsoLightCurveEntry } from "../measurementLogTso";
import type { DepthSpectrum } from "../tsoLightCurves";

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
  return {
    set_id: "abc",
    label: "NRS1",
    series: { detector: "NRS1", sporder: 0 },
    mode: "pipeline",
    time_scale: "bmjd_tdb",
    time: [60000.1, 60000.2, 60000.3, 60000.4],
    mjd_utc: [60000.098, 60000.198, 60000.298, 60000.398],
    int_num: [1, 2, 3, 4],
    row_index: [0, 1, 2, 3],
    flux_unit: "Jy",
    white: curve([10, 11, 12, 13]),
    bins: [curve([1, 2, 3, 4], { band_um: [1, 1.5], limits_origin: "user" })],
    dropped: { nan_time: 0, repeated_time: 0 },
    rejected_elements: [],
    notes: ["n1"],
    warnings: ["w1"],
    ...extra,
  };
}

const FILES = [
  { file_name: "a-seg001_nrs1_x1dints.fits", sha256: "aa", level: 2 },
  { file_name: "a-seg002_nrs1_x1dints.fits", sha256: "bb", level: 2 },
];
const FILE_NAMES = ["a-seg001_nrs1_x1dints.fits", "a-seg002_nrs1_x1dints.fits"];

function spectrum(extra: Partial<DepthSpectrum> = {}): DepthSpectrum {
  return {
    white: {
      docId: "white",
      label: "white light 1 to 2 um",
      band: [1, 2],
      centre: 1.5,
      halfWidth: 0.5,
      depth: 0.5,
      errScatter: 0.01,
      errFormal: 0.002,
      err: 0.01,
      nIn: 2,
      nOut: 6,
      pixelRatio: 0.43,
      pixelLoss: true,
    },
    rows: [
      {
        docId: "bin:0",
        label: "1 to 1.5 um",
        band: [1, 1.5],
        centre: 1.25,
        halfWidth: 0.25,
        depth: 0.25,
        errScatter: null,
        errFormal: 0.02,
        err: 0.02,
        nIn: 2,
        nOut: 6,
        pixelRatio: null,
        pixelLoss: false,
      },
    ],
    window: { start: 60000.2, end: 60000.3 },
    marginDays: 0.01,
    notes: [],
    warnings: ["w2"],
    ...extra,
  };
}

describe("measurementLogTso", () => {
  it("logs a computed set of curves with its inputs and limits", () => {
    const entry = tsoLightCurveEntry(result(), FILES, "C:/out/x.csv");
    expect(entry).toEqual({
      kind: "tso_light_curve",
      file: "a-seg001_nrs1_x1dints.fits",
      image: "4 integrations",
      dq: "not_applied",
      unit: "Jy",
      source: JSON.stringify({ kind: "x1dints", files: FILE_NAMES, detector: "NRS1", sporder: 0 }),
      params: {
        series: "NRS1",
        mode: "pipeline",
        time_scale: "BMJD (TDB)",
        band_lo_um: 1,
        band_hi_um: 2,
        limits_origin: "open",
        n_bins: 1,
        bins: "1-1.5",
        n_files: 2,
        source_level: "2",
        set_id: "abc",
        source_sha256: "aa; bb",
        n_rejected_elements: 0,
        saved_path: "C:/out/x.csv",
      },
      values: {
        n_integrations: 4,
        dropped_nan_time: 0,
        dropped_repeated_time: 0,
        white_flux_median: 11.5,
        white_flux_err_median: 0.5,
        white_n_used_min: 5,
        white_n_used_max: 5,
        time_first: 60000.1,
        time_last: 60000.4,
      },
      notes: [
        "white_flux_median and white_flux_err_median are sums over the band in Jy; time_first and time_last in days, BMJD (TDB)",
        "w1",
        "n1",
      ],
    });
    expect(Object.keys(entry.params)).toEqual([
      "series",
      "mode",
      "time_scale",
      "band_lo_um",
      "band_hi_um",
      "limits_origin",
      "n_bins",
      "bins",
      "n_files",
      "source_level",
      "set_id",
      "source_sha256",
      "n_rejected_elements",
      "saved_path",
    ]);
    expect(Object.keys(entry.values)).toEqual([
      "n_integrations",
      "dropped_nan_time",
      "dropped_repeated_time",
      "white_flux_median",
      "white_flux_err_median",
      "white_n_used_min",
      "white_n_used_max",
      "time_first",
      "time_last",
    ]);

    const twoBins = tsoLightCurveEntry(
      result({
        bins: [curve([1, 2, 3, 4], { band_um: [1, 1.5] }), curve([1, 2, 3, 4], { band_um: [1.5, 2] })],
        white: curve([10, 11, 12, 13], { n_used: [3, 5, 4, 6] }),
        rejected_elements: [{ index: 3, wavelength_um: 1.2, reason: "r" }],
      }),
      [...FILES, { file_name: "b.fits", sha256: "cc", level: 3 }],
      null,
    );
    expect(twoBins.params.bins).toBe("1-1.5; 1.5-2");
    expect(twoBins.params.n_bins).toBe(2);
    expect(twoBins.params.source_level).toBe("2; 3");
    expect(twoBins.params.source_sha256).toBe("aa; bb; cc");
    expect(twoBins.params.n_rejected_elements).toBe(1);
    expect(twoBins.params.saved_path).toBeNull();
    expect(twoBins.values.white_n_used_min).toBe(3);
    expect(twoBins.values.white_n_used_max).toBe(6);
  });

  it("marks the masked mode, an unknown unit and an empty curve", () => {
    const entry = tsoLightCurveEntry(
      result({
        mode: "masked",
        flux_unit: "unknown",
        time_scale: "index",
        time: [],
        mjd_utc: [],
        int_num: [],
        row_index: [],
        white: curve([], { flux_err: [], n_used: [], npixels_mean: [] }),
        bins: [],
      }),
      [],
      null,
    );
    expect(entry.dq).toBe("excluded");
    expect(entry.unit).toBeNull();
    expect(entry.file).toBeNull();
    expect(entry.image).toBe("0 integrations");
    expect(entry.params.bins).toBeNull();
    expect(entry.params.source_sha256).toBe("");
    expect(entry.values.white_flux_median).toBeNull();
    expect(entry.values.white_n_used_min).toBeNull();
    expect(entry.values.white_n_used_max).toBeNull();
    expect(entry.values.time_first).toBeNull();
    expect(entry.values.time_last).toBeNull();
    expect(entry.notes[0]).toBe(
      "white_flux_median and white_flux_err_median are sums over the band in the flux unit of the file; time_first and time_last are integration numbers",
    );
  });

  it("logs one row per band of a depth spectrum, the white light first", () => {
    const entries = tsoDepthSpectrumEntries(spectrum(), result(), { baselineKind: "mean" }, FILES, null);
    expect(entries).toHaveLength(2);
    expect(entries[0]).toEqual({
      kind: "tso_depth_spectrum",
      file: "a-seg001_nrs1_x1dints.fits",
      image: "2 in, 6 out",
      dq: "not_applied",
      unit: "fraction",
      source: JSON.stringify({ kind: "x1dints", files: FILE_NAMES, detector: "NRS1", sporder: 0, band_um: [1, 2] }),
      params: {
        series: "NRS1",
        mode: "pipeline",
        band: "white",
        band_lo_um: 1,
        band_hi_um: 2,
        window_start: 60000.2,
        window_end: 60000.3,
        time_scale: "BMJD (TDB)",
        baseline_margin_days: 0.01,
        baseline_kind: "mean",
        set_id: "abc",
        source_sha256: "aa; bb",
        saved_path: null,
      },
      values: {
        depth: 0.5,
        depth_err: 0.01,
        depth_err_scatter: 0.01,
        depth_err_formal: 0.002,
        n_in: 2,
        n_out: 6,
        npixels_ratio: 0.43,
      },
      notes: [
        "quick look, no limb darkening or systematics model",
        "depth D = 1 - F_in / F_out; depth_err is the larger of the scatter and the formal estimate",
        "band edges in um; window in days, BMJD (TDB)",
        "mean NPIXELS inside the window is 0.43 of the baseline: the extraction lost pixels during the event, so this depth is biased high",
      ],
    });
    expect(Object.keys(entries[0].params)).toEqual([
      "series",
      "mode",
      "band",
      "band_lo_um",
      "band_hi_um",
      "window_start",
      "window_end",
      "time_scale",
      "baseline_margin_days",
      "baseline_kind",
      "set_id",
      "source_sha256",
      "saved_path",
    ]);
    expect(Object.keys(entries[0].values)).toEqual([
      "depth",
      "depth_err",
      "depth_err_scatter",
      "depth_err_formal",
      "n_in",
      "n_out",
      "npixels_ratio",
    ]);
    expect(entries[1].params.band).toBe("1 to 1.5 um");
    expect(entries[1].values.depth_err_scatter).toBeNull();
    expect(entries[1].values.npixels_ratio).toBeNull();
    expect(entries[1].notes).toHaveLength(3);
    expect(entries[1].source).toBe(
      JSON.stringify({ kind: "x1dints", files: FILE_NAMES, detector: "NRS1", sporder: 0, band_um: [1, 1.5] }),
    );

    const masked = tsoDepthSpectrumEntries(spectrum(), result({ mode: "masked", time_scale: "index" }), { baselineKind: "linear" }, FILES, "C:/out/d.csv");
    expect(masked[0].dq).toBe("excluded");
    expect(masked[0].params.baseline_kind).toBe("linear");
    expect(masked[0].params.saved_path).toBe("C:/out/d.csv");
    expect(masked[0].notes[2]).toBe("band edges in um; window in integration numbers");

    const noDepth = spectrum();
    const allNull = {
      ...noDepth,
      white: noDepth.white === null ? null : { ...noDepth.white, depth: null },
      rows: noDepth.rows.map((row) => ({ ...row, depth: null })),
    };
    expect(tsoDepthSpectrumEntries(allNull, result(), { baselineKind: "mean" }, FILES, null)).toEqual([]);
    expect(tsoDepthSpectrumEntries(spectrum({ window: null }), result(), { baselineKind: "mean" }, FILES, null)).toEqual([]);
  });
});
