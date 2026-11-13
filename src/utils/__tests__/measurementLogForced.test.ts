import { describe, it, expect } from "vitest";
import { FORCED_LOG_UNIT_NOTE, forcedPhotometryEntry, sedEntry, type SedLogInput } from "../measurementLogForced";
import { MeasurementLogCore, measurementLogCsv } from "../measurementLog";
import { CSV_LINE_END } from "../catalogCsv";
import type {
  ForcedApertureCorrection,
  ForcedBand,
  ForcedCell,
  ForcedPhotometryRequest,
  ForcedPhotometryResult,
} from "../../shared/types/forcedPhotometry";

const CLOCK = "2026-11-13T12:00:00.000Z";
const NO_WCS_ERROR = "r.fits has no celestial WCS: sky positions cannot be placed on it";
const RADIUS_PX = 7.633587786259541;
const JY_PER_UNIT = 3.630780547701009e-6;

function cell(overrides: Partial<ForcedCell> = {}): ForcedCell {
  return {
    x: 10.5,
    y: 20.25,
    on_image: true,
    error: null,
    flux_native: 0.550813,
    flux_err_native: 0.0275406,
    flux_jy: 2e-6,
    flux_err_jy: 1e-7,
    flux_err_corr_jy: null,
    flux_total_jy: null,
    mag_ab: 23.147425,
    mag_ab_err: 0.054287,
    upper_limit_jy: null,
    upper_limit_ab: null,
    snr: 20,
    masked_fraction: 0,
    n_nonfinite: 0,
    n_masked: 0,
    flags_or: 0,
    saturated: false,
    fwhm_px: null,
    ...overrides,
  };
}

function band(index: number, label: string, pivot: number | null, overrides: Partial<ForcedBand> = {}): ForcedBand {
  return {
    index,
    path: `C:/data/${label}.fits`,
    file_name: `${label}.fits`,
    label,
    error: null,
    filter: pivot === null ? null : { dataset: "DECAM", filter: label, pivot_um: pivot, source: "header" },
    photcal: null,
    calibration_origin: "BUNIT nanomaggy, zero point 22.5",
    jy_per_native_unit: 3.631e-6,
    bunit: "nanomaggy",
    pixel_scale_arcsec: 0.262,
    aperture_radius_px: 7.6,
    annulus_px: null,
    aperture_correction: null,
    err_origin: null,
    dq_source: null,
    f_corr: null,
    n_measured: 1,
    n_failed: 0,
    warnings: [],
    ...overrides,
  };
}

function correction(factor: number): ForcedApertureCorrection {
  return { radius_px: 7.6, reference_radius_px: 13.4, factor, scatter: 0.01, n_stars: 12, n_candidates: 20 };
}

function run(bands: ForcedBand[], cellsBySource: (ForcedCell | null)[][]): ForcedPhotometryResult {
  return {
    bands,
    sources: cellsBySource.map((cells, i) => ({ id: `s${i}`, ra_deg: [53.1, 53.2][i], dec_deg: [-28.1, -28.2][i], cells })),
    aperture: { radius_arcsec: 2, correction: "growth_curve", reference_radius_arcsec: 3.5 },
    limit_sigma: 3,
    elapsed_ms: 12,
    warnings: ["w1"],
  };
}

const REQUEST: ForcedPhotometryRequest = {
  bands: [{ path: "C:/d/g.fits" }, { path: "C:/d/r.fits" }],
  positions: [
    { id: "s0", ra_deg: 53.1, dec_deg: -28.1 },
    { id: "s1", ra_deg: 53.2, dec_deg: -28.2 },
  ],
  aperture: { radius_arcsec: 2, correction: "growth_curve" },
  options: { exclude_dq: false, limit_sigma: 3 },
};

const G_BAND = band(0, "g", 0.4808, {
  aperture_radius_px: RADIUS_PX,
  aperture_correction: correction(1.0413),
  jy_per_native_unit: JY_PER_UNIT,
  n_measured: 2,
});
const R_BAND = band(1, "r", 0.6418, { aperture_radius_px: RADIUS_PX, f_corr: 1.647, jy_per_native_unit: JY_PER_UNIT, n_measured: 2 });

const S0_G = cell({ flux_jy: 1e-6, flux_err_jy: 1e-7, mag_ab: 23.9 });
const S0_R = cell({ flux_jy: 2e-6, flux_err_jy: 1e-7, flux_err_corr_jy: 1.647e-7, mag_ab: 23.147425 });
const S1_G = cell({ flux_jy: 1e-7, flux_err_jy: 1e-7, upper_limit_jy: 3e-7, upper_limit_ab: 25.207, mag_ab: null, mag_ab_err: null });
const S1_R = cell();

const RESULT = run([G_BAND, R_BAND], [
  [S0_G, S0_R],
  [S1_G, S1_R],
]);

const COUNTS = {
  n_positions: 2,
  n_measured_g: 2,
  n_limits_g: 1,
  n_failed_g: 0,
  aperture_radius_px_g: RADIUS_PX,
  aperture_correction_g: 1.0413,
  f_corr_g: null,
  ujy_per_unit_g: 3.6307805477,
  n_measured_r: 2,
  n_limits_r: 0,
  n_failed_r: 0,
  aperture_radius_px_r: RADIUS_PX,
  aperture_correction_r: null,
  f_corr_r: 1.647,
  ujy_per_unit_r: 3.6307805477,
};

const SED_INPUT: SedLogInput = {
  file: "g.fits; r.fits",
  image: "original",
  label: "s0, aperture fluxes",
  unit: "uJy",
  points: [
    { index: 0, filter: "g", pivotUm: 0.4808, value: 2, err: 0.1 },
    { index: 1, filter: "r", pivotUm: 0.6418, value: 0.3, err: null, limit: true },
  ],
  savedPath: null,
};

function makeStore(): MeasurementLogCore {
  let n = 0;
  return new MeasurementLogCore(
    () => new Date(CLOCK),
    () => `id${++n}`,
  );
}

describe("forced photometry log entry", () => {
  it("a run is logged with its aperture, its bands and the counts per band", () => {
    const entry = forcedPhotometryEntry(REQUEST, RESULT, null);
    expect(entry.kind).toBe("forced_photometry");
    expect(entry.file).toBe("g.fits; r.fits");
    expect(entry.image).toBe("original");
    expect(entry.dq).toBe("off");
    expect(entry.unit).toBe("uJy");
    expect(entry.source).toBe('{"kind":"sky_positions","n_positions":2}');
    const params = {
      aperture_radius_arcsec: 2,
      aperture_radius_px: null,
      annulus_inner: null,
      annulus_outer: null,
      annulus_unit: null,
      correction: "growth_curve",
      reference_radius_arcsec: 3.5,
      limit_sigma: 3,
      gain: null,
      n_bands: 2,
      bands: "g; r",
    };
    expect(entry.params).toEqual(params);
    expect(Object.keys(entry.params)).toEqual(Object.keys(params));
    expect(entry.values).toEqual(COUNTS);
    expect(Object.keys(entry.values)).toEqual(Object.keys(COUNTS));
    expect(entry.notes).toEqual([FORCED_LOG_UNIT_NOTE, "w1"]);
    expect(FORCED_LOG_UNIT_NOTE).toBe(
      "flux_ujy, flux_err_ujy, upper_limit_ujy in uJy: aperture fluxes, not corrected to total (multiply by aperture_correction); flux_err_ujy includes f_corr where a band has one; mag_ab in AB mag; n_ values are counts; aperture_radius_px in px; ujy_per_unit in uJy per image unit; aperture_correction and f_corr are factors",
    );
  });

  it("with a selected source its fluxes come first, in uJy, with the noise factor in the error", () => {
    const entry = forcedPhotometryEntry(REQUEST, RESULT, 0);
    expect(entry.source).toBe('{"kind":"sky_position","id":"s0","ra_deg":53.1,"dec_deg":-28.1,"n_positions":2}');
    const keys = Object.keys(entry.values);
    expect(keys.slice(0, 8)).toEqual([
      "flux_ujy_g",
      "flux_err_ujy_g",
      "upper_limit_ujy_g",
      "mag_ab_g",
      "flux_ujy_r",
      "flux_err_ujy_r",
      "upper_limit_ujy_r",
      "mag_ab_r",
    ]);
    expect(keys.slice(0, 8).map((key) => entry.values[key])).toEqual([1, 0.1, null, 23.9, 2, 0.1647, null, 23.147425]);
    expect(keys[8]).toBe("n_positions");
    expect(keys.slice(8)).toEqual(Object.keys(COUNTS));
    expect(entry.params).toEqual(forcedPhotometryEntry(REQUEST, RESULT, null).params);

    const limit = forcedPhotometryEntry(REQUEST, RESULT, 1);
    expect(limit.values.flux_ujy_g).toBe(0.1);
    expect(limit.values.upper_limit_ujy_g).toBe(0.3);
    expect(limit.values.mag_ab_g).toBeNull();
    expect(limit.source).toBe('{"kind":"sky_position","id":"s1","ra_deg":53.2,"dec_deg":-28.2,"n_positions":2}');

    expect(forcedPhotometryEntry(REQUEST, RESULT, 7)).toEqual(forcedPhotometryEntry(REQUEST, RESULT, null));
  });

  it("the annulus, the pixel radius and the exclusion are recorded as asked", () => {
    const request: ForcedPhotometryRequest = {
      ...REQUEST,
      aperture: { radius_px: 18.58, annulus_px: [28.55, 44.21], correction: "none" },
      options: { exclude_dq: true, gain: 4.76 },
    };
    const entry = forcedPhotometryEntry(request, RESULT, null);
    expect(entry.params.aperture_radius_px).toBe(18.58);
    expect(entry.params.aperture_radius_arcsec).toBeNull();
    expect(entry.params.annulus_inner).toBe(28.55);
    expect(entry.params.annulus_outer).toBe(44.21);
    expect(entry.params.annulus_unit).toBe("px");
    expect(entry.params.correction).toBe("none");
    expect(entry.params.gain).toBe(4.76);
    expect(entry.dq).toBe("requested");

    const masked = run([G_BAND, R_BAND], [
      [S0_G, cell({ n_masked: 2 })],
      [S1_G, S1_R],
    ]);
    expect(forcedPhotometryEntry(request, masked, null).dq).toBe("excluded");
    expect(forcedPhotometryEntry(REQUEST, masked, null).dq).toBe("off");

    const arcsecAnnulus = forcedPhotometryEntry({ ...REQUEST, aperture: { radius_arcsec: 2, annulus_arcsec: [4, 6], correction: "none" } }, RESULT, null);
    expect(arcsecAnnulus.params.annulus_inner).toBe(4);
    expect(arcsecAnnulus.params.annulus_outer).toBe(6);
    expect(arcsecAnnulus.params.annulus_unit).toBe("arcsec");
  });

  it("a band that failed is named in the notes and keeps its columns", () => {
    const failedR = band(1, "r", null, {
      error: NO_WCS_ERROR,
      calibration_origin: null,
      jy_per_native_unit: null,
      bunit: null,
      pixel_scale_arcsec: null,
      aperture_radius_px: null,
      n_measured: 0,
    });
    const result = run([G_BAND, failedR], [
      [S0_G, null],
      [S1_G, null],
    ]);
    const entry = forcedPhotometryEntry(REQUEST, result, null);
    expect(entry.notes).toEqual([FORCED_LOG_UNIT_NOTE, `r: not measured: ${NO_WCS_ERROR}`, "w1"]);
    expect(entry.values.n_measured_r).toBe(0);
    expect(entry.values.n_limits_r).toBe(0);
    expect(entry.values.aperture_radius_px_r).toBeNull();
    expect(entry.values.ujy_per_unit_r).toBeNull();
    const selected = forcedPhotometryEntry(REQUEST, result, 0);
    expect(selected.values.flux_ujy_r).toBeNull();
    expect(selected.values.flux_err_ujy_r).toBeNull();
    expect(selected.values.upper_limit_ujy_r).toBeNull();
    expect(selected.values.mag_ab_r).toBeNull();
    expect(selected.values.flux_ujy_g).toBe(1);
  });
});

describe("sed log entry", () => {
  it("an sed entry records each point under its filter name", () => {
    const entry = sedEntry(SED_INPUT);
    expect(entry.kind).toBe("sed");
    expect(entry.file).toBe("g.fits; r.fits");
    expect(entry.image).toBe("original");
    expect(entry.dq).toBe("not_applied");
    expect(entry.unit).toBe("uJy");
    expect(entry.source).toBe('{"kind":"sed","label":"s0, aperture fluxes"}');
    expect(entry.params).toEqual({ n_points: 2, saved_path: null });
    expect(entry.values).toEqual({ flux_g: 2, flux_err_g: 0.1, limit_g: false, flux_r: 0.3, flux_err_r: null, limit_r: true });
    expect(Object.keys(entry.values)).toEqual(["flux_g", "flux_err_g", "limit_g", "flux_r", "flux_err_r", "limit_r"]);
    expect(entry.notes).toEqual(["flux and flux_err in uJy; limit true means the flux is an upper limit"]);
    expect(sedEntry({ ...SED_INPUT, savedPath: "C:/out/sed.csv" }).params).toEqual({ n_points: 2, saved_path: "C:/out/sed.csv" });
  });

  it("filter names become column names once each", () => {
    const point = (index: number, filter: string) => ({ index, filter, pivotUm: 1.5, value: index + 1, err: 0.1 });
    const entry = sedEntry({ ...SED_INPUT, unit: null, points: [point(0, "F150W"), point(1, "F150W"), point(2, "r band")] });
    expect(Object.keys(entry.values).filter((key) => key.startsWith("flux_") && !key.startsWith("flux_err_"))).toEqual([
      "flux_F150W",
      "flux_F150W_2",
      "flux_r_band",
    ]);
    expect(entry.values.flux_F150W).toBe(1);
    expect(entry.values.flux_F150W_2).toBe(2);
    expect(entry.values.flux_err_F150W_2).toBe(0.1);
    expect(entry.values.limit_r_band).toBe(false);
    expect(entry.notes).toEqual([]);
    const empty = sedEntry({ ...SED_INPUT, points: [] });
    expect(empty.values).toEqual({});
    expect(empty.params.n_points).toBe(0);
  });
});

describe("forced photometry and sed rows in the log", () => {
  it("both drafts are accepted by the log", () => {
    const store = makeStore();
    store.append(forcedPhotometryEntry(REQUEST, RESULT, null));
    store.append(sedEntry(SED_INPUT));
    const entries = store.getSnapshot();
    expect(entries.map((e) => e.kind)).toEqual(["forced_photometry", "sed"]);
    const header = measurementLogCsv(entries).split(CSV_LINE_END)[0].split(",");
    expect(header).toEqual(expect.arrayContaining(["p_aperture_radius_arcsec", "v_n_positions", "p_n_points", "v_flux_g"]));
  });
});
