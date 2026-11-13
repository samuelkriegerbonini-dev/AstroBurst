import { describe, expect, it } from "vitest";
import type { TsoCurve, TsoLightCurves } from "../../shared/types/tso";
import { tsoCurvesCsv, tsoCurvesCsvFileName, tsoDepthSpectrumCsv, tsoDepthSpectrumCsvFileName } from "../tsoExports";
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
const CONTEXT = { software: "AstroBurst v0.8.0", exportedUtc: "2026-11-12T10:00:00Z", files: FILES };

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

function lines(text: string): string[] {
  return text.split("\r\n");
}

describe("tsoExports", () => {
  it("prints the band of a bin with its open upper edge and the white light with the origin of its limits", () => {
    const binLines = lines(tsoCurvesCsv(result({ bins: [curve([1, 2, 3, 4], { band_um: [0.6, 1] })] }), CONTEXT));
    expect(binLines).toContain("# bin1_band_um: 0.6 to 1, upper edge excluded");
    const logLines = lines(
      tsoCurvesCsv(result({ white: curve([10, 11, 12, 13], { band_um: [2.47, 3.94], limits_origin: "file log" }) }), CONTEXT),
    );
    expect(logLines).toContain("# white_band_um: 2.47 to 3.94 (file log)");
    const openLines = lines(
      tsoCurvesCsv(result({ white: curve([10, 11, 12, 13], { band_um: [0.5530469695008425, 5.370352185327354] }) }), CONTEXT),
    );
    expect(openLines).toContain("# white_band_um: 0.5530469695008425 to 5.370352185327354 (open)");
  });

  it("writes the curves with provenance first, then units, then one row per integration", () => {
    const text = tsoCurvesCsv(result(), CONTEXT);
    const all = lines(text);
    const head = [
      "# software: AstroBurst v0.8.0",
      "# exported_utc: 2026-11-12T10:00:00Z",
      "# file: a-seg001_nrs1_x1dints.fits; a-seg002_nrs1_x1dints.fits",
      "# product: tso_light_curves",
      "# series: NRS1",
      "# detector: NRS1",
      "# sporder: 0",
      "# mode: pipeline",
      "# time_scale: bmjd_tdb",
      "# flux_unit: Jy",
      "# set_id: abc",
      "# source_level: 2",
      "# source_sha256: aa; bb",
      "# white_band_um: 1 to 2 (open)",
      "# bin1_band_um: 1 to 1.5, upper edge excluded",
      "# dropped_nan_time: 0",
      "# dropped_repeated_time: 0",
      "# rejected_elements: none",
      "# warning: w1",
      "# unit time: d, BMJD (TDB)",
      "# unit mjd_utc: d, MJD (UTC)",
      "# unit white_flux: Jy",
      "# unit white_flux_err: Jy",
      "# unit bin1_flux: Jy",
      "# unit bin1_flux_err: Jy",
    ];
    expect(all.slice(0, head.length)).toEqual(head);
    expect(all[head.length]).toBe(
      "int_num,row_index,time,mjd_utc,white_flux,white_flux_err,white_n_used,white_npixels_mean,bin1_flux,bin1_flux_err,bin1_n_used,bin1_npixels_mean",
    );
    expect(all[head.length + 1]).toBe("1,0,60000.1,60000.098,10,0.5,5,7,1,0.5,5,7");
    expect(all.slice(head.length + 1, -1)).toHaveLength(4);
    expect(all[head.length + 4]).toBe("4,3,60000.4,60000.398,13,0.5,5,7,4,0.5,5,7");
    expect(text.endsWith("\r\n")).toBe(true);
    expect(all[all.length - 1]).toBe("");
  });

  it("leaves missing numbers empty and lists rejected elements and a missing order", () => {
    const text = tsoCurvesCsv(
      result({
        series: { detector: "NIS", sporder: null },
        time_scale: "index",
        white: curve([10, null, 12, 13], { npixels_mean: [7, null, 7, 7] }),
        rejected_elements: [{ index: 230, wavelength_um: 3.0178602767154326, reason: "r" }],
      }),
      CONTEXT,
    );
    const all = lines(text);
    expect(all).toContain("# sporder: none");
    expect(all).toContain("# rejected_elements: 230 at 3.0178602767154326 um");
    expect(all).toContain("# unit time: integration number");
    const header = all.findIndex((line) => line.startsWith("int_num,"));
    const cells = all[header + 2].split(",");
    const columns = all[header].split(",");
    expect(cells[columns.indexOf("white_flux")]).toBe("");
    expect(cells[columns.indexOf("white_npixels_mean")]).toBe("");
    expect(cells[columns.indexOf("white_flux_err")]).toBe("0.5");
  });

  it("names the two files after the first input, the series and the mode", () => {
    expect(tsoCurvesCsvFileName({ label: "NIS, order 2", mode: "masked" }, "jw07073-o004_t001_niriss_x1dints.fits")).toBe(
      "jw07073-o004_t001_niriss_x1dints_nis_order_2_masked_lightcurves.csv",
    );
    expect(tsoCurvesCsvFileName({ label: "NRS1", mode: "pipeline" }, "a_x1dints.fits.gz")).toBe(
      "a_x1dints_nrs1_pipeline_lightcurves.csv",
    );
    expect(tsoDepthSpectrumCsvFileName({ label: "NRS1", mode: "pipeline" }, "a_x1dints.fits")).toBe(
      "a_x1dints_nrs1_pipeline_depth_spectrum.csv",
    );
    expect(tsoCurvesCsvFileName({ label: "NRS1", mode: "pipeline" }, "")).toBe("tso_lightcurves.csv");
    expect(tsoDepthSpectrumCsvFileName({ label: "NRS1", mode: "pipeline" }, "")).toBe("tso_depth_spectrum.csv");
  });

  it("writes the depth spectrum with the quick-look label and one row per band", () => {
    const all = lines(tsoDepthSpectrumCsv(spectrum(), result(), { baselineKind: "mean" }, CONTEXT));
    expect(all.slice(0, 3)).toEqual([
      "# software: AstroBurst v0.8.0",
      "# exported_utc: 2026-11-12T10:00:00Z",
      "# file: a-seg001_nrs1_x1dints.fits; a-seg002_nrs1_x1dints.fits",
    ]);
    expect(all.slice(3)).toEqual([
      "# product: tso_depth_spectrum",
      "# series: NRS1",
      "# detector: NRS1",
      "# sporder: 0",
      "# mode: pipeline",
      "# quick_look: quick look, no limb darkening or systematics model",
      "# depth: D = 1 - F_in / F_out; depth_err is the larger of depth_err_scatter and depth_err_formal",
      "# window_start: 60000.2",
      "# window_end: 60000.3",
      "# window_time: BMJD (TDB)",
      "# baseline_margin_days: 0.01",
      "# baseline_kind: mean",
      "# set_id: abc",
      "# source_level: 2",
      "# source_sha256: aa; bb",
      "# warning: w2",
      "# unit band_lo_um: um",
      "# unit band_hi_um: um",
      "# unit centre_um: um",
      "# unit depth: fraction",
      "# unit depth_err: fraction",
      "# unit depth_err_scatter: fraction",
      "# unit depth_err_formal: fraction",
      "band,band_lo_um,band_hi_um,centre_um,depth,depth_err,depth_err_scatter,depth_err_formal,n_in,n_out,npixels_ratio,pixel_loss",
      "white,1,2,1.5,0.5,0.01,0.01,0.002,2,6,0.43,true",
      "1 to 1.5 um,1,1.5,1.25,0.25,0.02,,0.02,2,6,,false",
      "",
    ]);

    const bare = lines(tsoDepthSpectrumCsv(spectrum({ window: null, white: null }), result(), { baselineKind: "median" }, CONTEXT));
    expect(bare).toContain("# window_start:");
    expect(bare).toContain("# window_end:");
    expect(bare).toContain("# baseline_kind: median");
    const header = bare.indexOf(
      "band,band_lo_um,band_hi_um,centre_um,depth,depth_err,depth_err_scatter,depth_err_formal,n_in,n_out,npixels_ratio,pixel_loss",
    );
    expect(bare.slice(header + 1)).toEqual(["1 to 1.5 um,1,1.5,1.25,0.25,0.02,,0.02,2,6,,false", ""]);
  });
});
