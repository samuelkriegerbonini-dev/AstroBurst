import { describe, expect, it } from "vitest";
import { imageDepthEntry, resolutionCheckEntry } from "../measurementLogQa";
import { MeasurementLogCore, measurementLogCsv } from "../measurementLog";
import { CSV_LINE_END } from "../catalogCsv";
import type { ImageDepth } from "../../shared/types/depth";
import type { ResolutionCheck } from "../../shared/types/resolution";

const PROV = { file: "brick-r.fits", image: "original" };

const DEPTH: ImageDepth = {
  radius_arcsec: 1,
  radius_px: 3.8167938931297707,
  pixel_scale_arcsec: 0.262,
  n_requested: 500,
  n_placed: 500,
  n_used: 498,
  seed: 1,
  n_sigma: 5,
  detect_nsigma: 3,
  masked_fraction: 0.45,
  median_sum_native: 0.0002,
  sigma_empty_native: 0.0118,
  sigma_clipped_native: 0.0117,
  n_after_clip: 490,
  sigma_empty_rel_err: 0.0522,
  sigma_formal_native: 0.0071,
  f_corr: 1.662,
  limit_native: 0.059,
  limit_jy: 2.142e-7,
  limit_ab: 25.573,
  psf_fwhm_arcsec: null,
  psf_limit_formal_native: null,
  psf_limit_formal_ab: null,
  sigma_pix_median_native: 0.00072,
  sigma_pix_spread: 1.99,
  bunit: "nanomaggy",
  calibration_origin: "BUNIT nanomaggy, zero point 22.5",
  err_origin: "inverse variance from brick-invvar-r.fits",
  dq_source: "maskbits from brick-maskbits.fits",
  definition: "5 sigma, empty apertures of radius 1.00 arcsec, 1.4826 x MAD of 498 sums, sources masked at 3 sigma, seed 1",
  apertures: [
    { x: 10.25, y: 20.5, sum: 0.012, formal_err: 0.007, usable: true },
    { x: 30, y: 40, sum: -0.004, formal_err: null, usable: false },
  ],
  notes: ["n1"],
  warnings: ["w1"],
  elapsed_ms: 900,
};

const COSMOLOGY = "flat LambdaCDM, H0 67.66, Omega_m 0.30966, plus Planck 2018 neutrinos and radiation";

const CHECK: ResolutionCheck = {
  verdict: "marginal",
  size_basis: "fwhm",
  psf_fwhm_arcsec: 1.14,
  psf_reference: "typed value",
  pixels_per_fwhm: 5.7,
  nyquist_sampled: true,
  psf_area_px: 73.62819050398275,
  intrinsic_size_arcsec: 0.374699879903904,
  snr_floor_fraction: 0.250290234995889,
  psf_floor_fraction: 0.24677925358506123,
  floor_fraction: 0.250290234995889,
  size_floor_arcsec: 0.2853308678953134,
  resolution_elements: 0.3286841051788632,
  kpc_per_arcsec: 8.23125144264795,
  kpc_per_pixel: 1.6462502885295902,
  angular_diameter_distance_mpc: 1697.8174839889118,
  psf_fwhm_kpc: 9.383626644618662,
  intrinsic_size_kpc: 3.084248927019024,
  size_floor_kpc: 2.3486301179952904,
  scale_of_interest_in_psf: 0.10656860485529672,
  snr: 50,
  redshift: 1,
  psf_fwhm_rel_err: 0.03,
  resolved_elements: 3,
  min_pixels_per_fwhm: 2,
  scale_of_interest_kpc: 1,
  h0_km_s_mpc: 67.66,
  omega_m: 0.30966,
  omega_nu_massive: 0.0014147613306646316,
  omega_rad: 7.893312479128647e-5,
  cosmology: COSMOLOGY,
  notes: [],
  warnings: [],
};

const DEPTH_PARAMS = [
  "radius_arcsec",
  "radius_px",
  "n_requested",
  "n_sigma",
  "seed",
  "detect_nsigma",
  "psf_fwhm_arcsec",
  "calibration",
  "error_layer",
  "mask_layer",
];

const DEPTH_VALUES = [
  "limit_ab",
  "limit_ujy",
  "limit_native",
  "sigma_empty_native",
  "sigma_clipped_native",
  "sigma_formal_native",
  "f_corr",
  "sigma_empty_rel_err",
  "n_used",
  "n_placed",
  "masked_fraction",
  "psf_limit_formal_ab",
  "psf_limit_formal_native",
  "sigma_pix_median_native",
  "sigma_pix_spread",
  "pixel_scale_arcsec",
];

const RESOLUTION_PARAMS = [
  "context",
  "psf_fwhm_arcsec",
  "psf_reference",
  "snr",
  "redshift",
  "psf_fwhm_rel_err",
  "resolved_elements",
  "min_pixels_per_fwhm",
  "scale_of_interest_kpc",
  "h0_km_s_mpc",
  "omega_m",
];

const RESOLUTION_VALUES = [
  "verdict",
  "size_basis",
  "intrinsic_size_arcsec",
  "size_floor_arcsec",
  "floor_fraction",
  "resolution_elements",
  "pixels_per_fwhm",
  "nyquist_sampled",
  "kpc_per_arcsec",
  "psf_fwhm_kpc",
  "intrinsic_size_kpc",
  "size_floor_kpc",
  "scale_of_interest_in_psf",
];

const SUMS_NOTE_PREFIX = "limit_native, the sigma values and psf_limit_formal_native are sums in";

describe("image depth log entry", () => {
  it("the depth entry leads with the AB limit and names every other unit", () => {
    const entry = imageDepthEntry(PROV, DEPTH);
    expect(entry.kind).toBe("image_depth");
    expect(entry.file).toBe("brick-r.fits");
    expect(entry.image).toBe("original");
    expect(entry.dq).toBe("excluded");
    expect(entry.unit).toBe("AB mag");
    expect(entry.source).toBe(DEPTH.definition);
    expect(Object.keys(entry.params)).toEqual(DEPTH_PARAMS);
    expect(Object.keys(entry.values)).toEqual(DEPTH_VALUES);
    const limitUjy = entry.values.limit_ujy as number;
    expect(Math.abs(limitUjy - 0.2142) / 0.2142).toBeLessThan(1e-12);
    expect(entry.values.f_corr).toBe(1.662);
    expect(entry.values.limit_ab).toBe(25.573);
    expect(entry.params.calibration).toBe("BUNIT nanomaggy, zero point 22.5");
    expect(entry.params.error_layer).toBe("inverse variance from brick-invvar-r.fits");
    expect(entry.params.mask_layer).toBe("maskbits from brick-maskbits.fits");
    expect(entry.notes).toEqual([
      "limit_native, the sigma values and psf_limit_formal_native are sums in nanomaggy; sigma_pix_median_native is per pixel; limit_ujy is uJy",
      "n1",
      "w1",
    ]);
  });

  it("an uncalibrated depth is logged in the unit of the sums", () => {
    const uncalibrated: ImageDepth = { ...DEPTH, limit_ab: null, limit_jy: null, calibration_origin: null, dq_source: null };
    const entry = imageDepthEntry(PROV, uncalibrated);
    expect(entry.unit).toBe("nanomaggy");
    expect(entry.dq).toBe("off");
    expect(entry.values.limit_ujy).toBeNull();
    expect(entry.values.limit_ab).toBeNull();
    expect(entry.params.calibration).toBeNull();
    expect(entry.params.mask_layer).toBeNull();

    const surface = imageDepthEntry(PROV, { ...uncalibrated, bunit: "MJy/sr" });
    expect(surface.unit).toBe("MJy/sr x pix");
    expect(surface.notes[0].startsWith(`${SUMS_NOTE_PREFIX} MJy/sr x pix;`)).toBe(true);

    const unitless = imageDepthEntry(PROV, { ...uncalibrated, bunit: null });
    expect(unitless.unit).toBeNull();
    expect(unitless.notes[0].startsWith(`${SUMS_NOTE_PREFIX} the image unit x pix;`)).toBe(true);
  });
});

describe("resolution check log entry", () => {
  it("the resolution entry records the verdict with its inputs and thresholds", () => {
    const entry = resolutionCheckEntry(PROV, CHECK, "photometry", "photometry at (512.3, 498.7)");
    expect(entry.kind).toBe("resolution_check");
    expect(entry.file).toBe("brick-r.fits");
    expect(entry.image).toBe("original");
    expect(entry.unit).toBeNull();
    expect(entry.dq).toBe("off");
    expect(entry.source).toBe("photometry at (512.3, 498.7)");
    expect(Object.keys(entry.params)).toEqual(RESOLUTION_PARAMS);
    expect(Object.keys(entry.values)).toEqual(RESOLUTION_VALUES);
    expect(entry.values.verdict).toBe("Marginally resolved");
    expect(entry.values.psf_fwhm_kpc).toBe(9.383626644618662);
    expect(entry.values.nyquist_sampled).toBe(true);
    expect(entry.values.size_basis).toBe("fwhm");
    expect(entry.params.context).toBe("photometry");
    expect(entry.params.psf_reference).toBe("typed value");
    expect(entry.params.psf_fwhm_arcsec).toBe(1.14);
    expect(entry.notes[0]).toBe(`sizes in arcsec and proper kpc; ${COSMOLOGY}`);
    expect(entry.notes).toHaveLength(1);
  });

  it("a check without redshift or verdict logs nulls, not text", () => {
    const check: ResolutionCheck = {
      ...CHECK,
      verdict: null,
      kpc_per_arcsec: null,
      kpc_per_pixel: null,
      psf_fwhm_kpc: null,
      intrinsic_size_kpc: null,
      size_floor_kpc: null,
      scale_of_interest_in_psf: null,
      redshift: null,
      pixels_per_fwhm: Number.NaN,
      snr: Number.NaN,
      notes: ["c1"],
      warnings: ["c2"],
    };
    const entry = resolutionCheckEntry(PROV, check, "manual", "region A");
    expect(entry.values.verdict).toBeNull();
    expect(entry.values.kpc_per_arcsec).toBeNull();
    expect(entry.values.psf_fwhm_kpc).toBeNull();
    expect(entry.values.pixels_per_fwhm).toBeNull();
    expect(entry.params.snr).toBeNull();
    expect(entry.params.redshift).toBeNull();
    expect(entry.notes).toEqual(["sizes in arcsec", "c1", "c2"]);
  });
});

describe("QA entries in the log", () => {
  it("both drafts are accepted by the log", () => {
    let n = 0;
    const store = new MeasurementLogCore(
      () => new Date("2026-11-12T18:00:00.000Z"),
      () => `id${++n}`,
    );
    const depth = store.append(imageDepthEntry(PROV, DEPTH));
    expect(store.getSnapshot()).toHaveLength(1);
    expect(depth.kind).toBe("image_depth");
    const resolution = store.append(resolutionCheckEntry(PROV, CHECK, "profile", "region A"));
    expect(store.getSnapshot()).toHaveLength(2);
    expect(resolution.kind).toBe("resolution_check");
    const header = measurementLogCsv(store.getSnapshot()).split(CSV_LINE_END)[0].split(",");
    for (const column of ["p_radius_arcsec", "v_limit_ab", "p_psf_reference", "v_verdict"]) {
      expect(header).toContain(column);
    }
  });
});
