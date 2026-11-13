import { describe, expect, it } from "vitest";
import type { TsoFile } from "../../shared/types/tso";
import type { TsoTransitBin, TsoTransitSpectrum } from "../../shared/types/tsoTransit";
import type { DepthRow, DepthSpectrum } from "../tsoLightCurves";
import { TRANSIT_FIT_LABEL } from "../tsoTransitSpectrum";
import {
  NOT_MODELLED,
  TRANSIT_SPECTRUM_PRODUCT,
  tsoTransitSpectrumCsv,
  tsoTransitSpectrumCsvFileName,
  tsoTransitSpectrumEntries,
} from "../tsoTransitExports";

function bin(lo: number, hi: number, extra: Partial<TsoTransitBin> = {}): TsoTransitBin {
  return {
    band_um: [lo, hi],
    n_points: 240,
    n_in_transit: 18,
    weighted: true,
    rp: 0.1,
    rp_err: 0.001,
    depth: 0.01,
    depth_err: 0.0002,
    u1: 0.2,
    u1_err: null,
    u2: 0.25,
    u2_err: null,
    chi2: 250.5,
    dof: 237,
    rms: 0.0003,
    grazing: false,
    npixels_ratio: 1,
    pixel_loss: false,
    error: null,
    notes: [],
    warnings: [],
    ...extra,
  };
}

function failed(lo: number, hi: number, error: string): TsoTransitBin {
  return bin(lo, hi, { rp: null, rp_err: null, depth: null, depth_err: null, u1: null, u2: null, chi2: null, rms: null, error });
}

const FILES: Pick<TsoFile, "file_name" | "sha256" | "level">[] = [
  { file_name: "a-seg001_nrs1_x1dints.fits", sha256: "aa", level: 2 },
  { file_name: "a-seg002_nrs1_x1dints.fits", sha256: "bb", level: 2 },
];
const CONTEXT = { software: "AstroBurst v0.8.0", exportedUtc: "2026-11-13T10:00:00Z", files: FILES };
const MODEL = "Mandel and Agol 2002, quadratic limb darkening, circular orbit";
const NO_FLUX = "no integration has a flux in this band";

const FIRST_BIN = bin(0.6, 1, {
  rp: 7.7,
  rp_err: 0.02,
  depth: 59.29,
  depth_err: 0.31,
  chi2: 468409.5,
  grazing: true,
  pixel_loss: true,
  npixels_ratio: 0.64,
});

const SPEC: TsoTransitSpectrum = {
  set_id: "abc",
  label: "NRS1",
  series: { detector: "NRS1", sporder: 0 },
  mode: "pipeline",
  time_scale: "bmjd_tdb",
  fit: {
    period_days: 1.4079405,
    t0: 60061.507668,
    a_rs: 336,
    b: 7.16,
    rp_start: 7.73,
    u1: 0.2,
    u2: 0.25,
    limb_darkening: "fixed",
    baseline_order: 1,
    exclude_rows: [],
    scale_errors: false,
  },
  model: MODEL,
  n_integrations: 240,
  n_excluded: 0,
  n_in_transit: 18,
  bins: [FIRST_BIN, failed(1, 2, NO_FLUX)],
  notes: ["n1"],
  warnings: ["w1"],
};

const QUICK_ROW: DepthRow = {
  docId: "bin:0",
  label: "0.6 to 1 um",
  band: [0.6, 1],
  centre: 0.8,
  halfWidth: 0.2,
  depth: 0.9674,
  errScatter: 0.01,
  errFormal: 0.002,
  err: 0.01,
  nIn: 18,
  nOut: 200,
  pixelRatio: 0.64,
  pixelLoss: true,
};

const QUICK: DepthSpectrum = {
  white: null,
  rows: [QUICK_ROW],
  window: { start: 60061.45, end: 60061.56 },
  marginDays: 0,
  notes: [],
  warnings: [],
};

function lines(text: string): string[] {
  return text.split("\r\n");
}

function withFirstBin(spec: TsoTransitSpectrum, extra: Partial<TsoTransitBin>): TsoTransitSpectrum {
  return { ...spec, bins: [{ ...spec.bins[0], ...extra }, ...spec.bins.slice(1)] };
}

describe("tsoTransitExports", () => {
  it("writes the fitted spectrum with the model, what was fixed and what was not modelled", () => {
    const text = tsoTransitSpectrumCsv(SPEC, QUICK, CONTEXT);
    const all = lines(text);
    const head = [
      "# software: AstroBurst v0.8.0",
      "# exported_utc: 2026-11-13T10:00:00Z",
      "# file: a-seg001_nrs1_x1dints.fits; a-seg002_nrs1_x1dints.fits",
      "# product: tso_transit_spectrum",
      "# series: NRS1",
      "# detector: NRS1",
      "# sporder: 0",
      "# mode: pipeline",
      "# model: Mandel and Agol 2002, quadratic limb darkening, circular orbit",
      "# fitted_per_bin: Rp/Rs and a baseline polynomial of order 1",
      "# not_modelled: orbital eccentricity; the finite exposure time (the model is evaluated at the mid-integration times); limb darkening from stellar models; systematics beyond the baseline polynomial; correlated noise; MCMC",
      "# period_days: 1.4079405",
      "# t0: 60061.507668",
      "# t0_time: BMJD (TDB)",
      "# a_rs: 336",
      "# b: 7.16",
      "# limb_darkening: fixed",
      "# u1: 0.2",
      "# u2: 0.25",
      "# baseline_order: 1",
      "# rp_start: 7.73",
      "# n_integrations: 240",
      "# n_excluded: 0",
      "# excluded_rows:",
      "# depth: (Rp/Rs)^2 from the fit; depth_err is its 1 sigma error",
      "# errors: 1 sigma from the covariance of the fit",
      "# quick_depth: quick look D = 1 - F_in / F_out of the same band",
      "# set_id: abc",
      "# source_level: 2",
      "# source_sha256: aa; bb",
      "# warning: w1",
      "# note: n1",
      "# unit band_lo_um: um",
      "# unit band_hi_um: um",
      "# unit centre_um: um",
      "# unit depth: (Rp/Rs)^2",
      "# unit depth_err: (Rp/Rs)^2",
      "# unit rp: Rp/Rs",
      "# unit rp_err: Rp/Rs",
      "# unit rms: fraction of the median flux",
      "# unit quick_depth: fraction",
      "# unit quick_depth_err: fraction",
    ];
    expect(all.slice(0, head.length)).toEqual(head);
    expect(all.slice(head.length)).toEqual([
      "band,band_lo_um,band_hi_um,centre_um,depth,depth_err,rp,rp_err,u1,u1_err,u2,u2_err,chi2,dof,rms,n_points,n_in_transit,weighted,grazing,npixels_ratio,pixel_loss,quick_depth,quick_depth_err,error,warnings",
      "0.6 to 1 um,0.6,1,0.8,59.29,0.31,7.7,0.02,0.2,,0.25,,468409.5,237,0.0003,240,18,true,true,0.64,true,0.9674,0.01,,",
      "1 to 2 um,1,2,1.5,,,,,,,,,,237,,240,18,true,false,1,false,,,no integration has a flux in this band,",
      "",
    ]);
    expect(text.endsWith("\r\n")).toBe(true);
    expect(TRANSIT_SPECTRUM_PRODUCT).toBe("tso_transit_spectrum");
    expect(NOT_MODELLED).toBe(
      "orbital eccentricity; the finite exposure time (the model is evaluated at the mid-integration times); limb darkening from stellar models; systematics beyond the baseline polynomial; correlated noise; MCMC",
    );
  });

  it("says when the limb darkening was fitted, which rows were left out, which errors the file holds and the warnings of a bin", () => {
    const free: TsoTransitSpectrum = {
      ...SPEC,
      fit: { ...SPEC.fit, limb_darkening: "free", exclude_rows: [3, 7], scale_errors: true },
      n_excluded: 2,
      series: { detector: "NRS1", sporder: null },
      time_scale: "mjd_utc",
    };
    const all = lines(tsoTransitSpectrumCsv(free, null, CONTEXT));
    for (const line of [
      "# sporder: none",
      "# fitted_per_bin: Rp/Rs, u1, u2 and a baseline polynomial of order 1",
      "# t0_time: MJD (UTC)",
      "# limb_darkening: free",
      "# u1_start: 0.2",
      "# u2_start: 0.25",
      "# n_excluded: 2",
      "# excluded_rows: 3; 7",
      "# errors: 1 sigma from the covariance, multiplied by sqrt(chi2 / dof) of each bin",
    ]) {
      expect(all).toContain(line);
    }
    expect(all.some((line) => line.startsWith("# u1:"))).toBe(false);
    expect(all.some((line) => line.startsWith("# u2:"))).toBe(false);
    expect(all.indexOf("# u1_start: 0.2")).toBe(all.indexOf("# limb_darkening: free") + 1);
    expect(all.indexOf("# u2_start: 0.25")).toBe(all.indexOf("# limb_darkening: free") + 2);
    const header = all.findIndex((line) => line.startsWith("band,"));
    expect(all[header + 1].endsWith("0.64,true,,,,")).toBe(true);

    const warned = withFirstBin(SPEC, { warnings: ["wa", "wb"] });
    const warnedLines = lines(tsoTransitSpectrumCsv(warned, QUICK, CONTEXT));
    const warnedHeader = warnedLines.findIndex((line) => line.startsWith("band,"));
    expect(warnedLines[warnedHeader + 1].endsWith(",0.9674,0.01,,wa; wb")).toBe(true);

    const comma = withFirstBin(SPEC, { warnings: ["a, b"] });
    const commaLines = lines(tsoTransitSpectrumCsv(comma, QUICK, CONTEXT));
    expect(commaLines[commaLines.findIndex((line) => line.startsWith("band,")) + 1].endsWith(',,"a, b"')).toBe(true);

    const shifted: DepthSpectrum = { ...QUICK, rows: [{ ...QUICK_ROW, band: [0.6, 1.1] }] };
    const shiftedLines = lines(tsoTransitSpectrumCsv(SPEC, shifted, CONTEXT));
    expect(shiftedLines[shiftedLines.findIndex((line) => line.startsWith("band,")) + 1].endsWith("0.64,true,,,,")).toBe(true);
  });

  it("names the file after the first input, the series and the mode", () => {
    expect(tsoTransitSpectrumCsvFileName({ label: "NRS1", mode: "pipeline" }, "a_x1dints.fits")).toBe(
      "a_x1dints_nrs1_pipeline_transit_spectrum.csv",
    );
    expect(
      tsoTransitSpectrumCsvFileName({ label: "NIS, order 2", mode: "masked" }, "jw07073-o004_t001_niriss_x1dints.fits.gz"),
    ).toBe("jw07073-o004_t001_niriss_x1dints_nis_order_2_masked_transit_spectrum.csv");
    expect(tsoTransitSpectrumCsvFileName({ label: "NRS1", mode: "pipeline" }, "")).toBe("tso_transit_spectrum.csv");
  });

  it("logs one row per fitted band with the model and the fixed orbit", () => {
    const entries = tsoTransitSpectrumEntries(SPEC, FILES, "C:/out/x.csv");
    expect(entries).toHaveLength(1);
    const entry = entries[0];
    expect(entry).toEqual({
      kind: "tso_transit_spectrum",
      file: "a-seg001_nrs1_x1dints.fits",
      image: "18 in transit, 240 points",
      dq: "not_applied",
      unit: "(Rp/Rs)^2",
      source: JSON.stringify({
        kind: "x1dints",
        files: ["a-seg001_nrs1_x1dints.fits", "a-seg002_nrs1_x1dints.fits"],
        detector: "NRS1",
        sporder: 0,
        band_um: [0.6, 1],
      }),
      params: {
        series: "NRS1",
        mode: "pipeline",
        band: "0.6 to 1 um",
        band_lo_um: 0.6,
        band_hi_um: 1,
        model: MODEL,
        period_days: 1.4079405,
        t0: 60061.507668,
        time_scale: "BMJD (TDB)",
        a_rs: 336,
        b: 7.16,
        limb_darkening: "fixed",
        baseline_order: 1,
        scale_errors: false,
        rp_start: 7.73,
        n_excluded: 0,
        set_id: "abc",
        source_sha256: "aa; bb",
        saved_path: "C:/out/x.csv",
      },
      values: {
        depth: 59.29,
        depth_err: 0.31,
        rp: 7.7,
        rp_err: 0.02,
        u1: 0.2,
        u1_err: null,
        u2: 0.25,
        u2_err: null,
        chi2: 468409.5,
        dof: 237,
        rms: 0.0003,
        n_points: 240,
        n_in_transit: 18,
        npixels_ratio: 0.64,
      },
      notes: [
        TRANSIT_FIT_LABEL,
        "depth is (Rp/Rs)^2 from the fit; depth_err, rp_err, u1_err and u2_err are 1 sigma errors of the least-squares fit",
        "band edges in um; t0 in days, BMJD (TDB)",
        "grazing transit (b + Rp/Rs above 1): (Rp/Rs)^2 is not the fraction of light blocked",
        "mean NPIXELS inside the transit is 0.64 of the baseline: the extraction lost pixels during the event, so this depth is biased",
      ],
    });
    expect(Object.keys(entry.params)).toEqual([
      "series",
      "mode",
      "band",
      "band_lo_um",
      "band_hi_um",
      "model",
      "period_days",
      "t0",
      "time_scale",
      "a_rs",
      "b",
      "limb_darkening",
      "baseline_order",
      "scale_errors",
      "rp_start",
      "n_excluded",
      "set_id",
      "source_sha256",
      "saved_path",
    ]);
    expect(Object.keys(entry.values)).toEqual([
      "depth",
      "depth_err",
      "rp",
      "rp_err",
      "u1",
      "u1_err",
      "u2",
      "u2_err",
      "chi2",
      "dof",
      "rms",
      "n_points",
      "n_in_transit",
      "npixels_ratio",
    ]);
  });

  it("marks the masked mode, keeps the notes and warnings of a bin and logs nothing without a fitted bin", () => {
    const binNote =
      "fitted without weights: 1 of 240 integrations have no positive flux error, so the errors of this bin are scaled by sqrt(chi2 / dof)";
    const masked = withFirstBin(
      { ...SPEC, mode: "masked" },
      { grazing: false, pixel_loss: false, notes: [binNote], warnings: ["w-bin"] },
    );
    const [entry] = tsoTransitSpectrumEntries(masked, FILES, null);
    expect(entry.dq).toBe("excluded");
    expect(entry.params.saved_path).toBeNull();
    expect(entry.notes).toEqual([
      TRANSIT_FIT_LABEL,
      "depth is (Rp/Rs)^2 from the fit; depth_err, rp_err, u1_err and u2_err are 1 sigma errors of the least-squares fit",
      "band edges in um; t0 in days, BMJD (TDB)",
      binNote,
      "w-bin",
    ]);

    const [bare] = tsoTransitSpectrumEntries(SPEC, [], null);
    expect(bare.file).toBeNull();
    expect(bare.params.source_sha256).toBe("");
    expect(JSON.parse(bare.source).files).toEqual([]);

    const none: TsoTransitSpectrum = { ...SPEC, bins: [failed(0.6, 1, NO_FLUX), failed(1, 2, NO_FLUX)] };
    expect(tsoTransitSpectrumEntries(none, FILES, "C:/out/x.csv")).toEqual([]);
  });
});
