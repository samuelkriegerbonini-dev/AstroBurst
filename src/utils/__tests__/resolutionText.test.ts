import { describe, it, expect } from "vitest";
import {
  DEFAULT_RESOLUTION_SETTINGS,
  VERDICT_LABEL,
  imagePsfText,
  psfChoice,
  resolutionRequest,
  resolutionRows,
  robustWidth,
  saguiRegionBudget,
  sanitizeResolutionSettings,
  scaleText,
  seeingCardsFromHeader,
  significant,
} from "../resolutionText";
import type { ImagePsfFwhm, ResolutionCheck, ResolutionSettings } from "../../shared/types/resolution";
import type { SaguiInputInfo } from "../../shared/types/sagui";

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

const IMAGE = {
  fwhm_arcsec: 0.9185,
  half_light_radius_arcsec: 0.52,
  n_stars: 108,
  model: "M",
  header_values: [{ key: "SEEING", value_arcsec: 1.1 }],
} as ImagePsfFwhm;

const PSF17 = { fwhm_px: 3.5057, fwhm_arcsec: 0.9184, fwhm_px_scatter: 0.1314, n_stars: 108 } as ImagePsfFwhm;

const NO_PSF = { fwhmArcsec: null, halfLightRadiusArcsec: null, reference: null };

function withSource(psfSource: ResolutionSettings["psfSource"], psfFwhmArcsec: number | null = null): ResolutionSettings {
  return { ...DEFAULT_RESOLUTION_SETTINGS, psfSource, psfFwhmArcsec };
}

describe("resolution settings", () => {
  it("the defaults are the decisions of the round", () => {
    expect(DEFAULT_RESOLUTION_SETTINGS).toEqual({
      redshift: null,
      h0: 67.66,
      omegaM: 0.30966,
      psfSource: "image",
      psfFwhmArcsec: null,
      eps: 0.03,
      qResolved: 3,
      minPixelsPerFwhm: 2,
      scaleOfInterestKpc: 1,
    });
    expect(VERDICT_LABEL.marginal === "Marginally resolved").toBe(true);
  });

  it("stored settings are kept when valid and replaced member by member when not", () => {
    const valid: ResolutionSettings = {
      redshift: 1.2,
      h0: 70,
      omegaM: 0.3,
      psfSource: "typed",
      psfFwhmArcsec: 0.9,
      eps: 0.05,
      qResolved: 2.5,
      minPixelsPerFwhm: 2.5,
      scaleOfInterestKpc: 0.5,
    };
    expect(sanitizeResolutionSettings(valid)).toEqual(valid);
    const broken = sanitizeResolutionSettings({ h0: 5, omegaM: 1.2, psfSource: "x", redshift: -1, eps: 1, qResolved: 0, psfFwhmArcsec: 0 });
    expect(broken.h0).toBe(DEFAULT_RESOLUTION_SETTINGS.h0);
    expect(broken.omegaM).toBe(DEFAULT_RESOLUTION_SETTINGS.omegaM);
    expect(broken.psfSource).toBe(DEFAULT_RESOLUTION_SETTINGS.psfSource);
    expect(broken.redshift).toBe(DEFAULT_RESOLUTION_SETTINGS.redshift);
    expect(broken.eps).toBe(DEFAULT_RESOLUTION_SETTINGS.eps);
    expect(broken.qResolved).toBe(DEFAULT_RESOLUTION_SETTINGS.qResolved);
    expect(broken.psfFwhmArcsec).toBe(DEFAULT_RESOLUTION_SETTINGS.psfFwhmArcsec);
    expect(sanitizeResolutionSettings({ redshift: 0 }).redshift).toBeNull();
    expect(sanitizeResolutionSettings({ redshift: 25 }).redshift).toBeNull();
    for (const raw of [null, "abc", [], 42]) {
      expect(sanitizeResolutionSettings(raw)).toEqual(DEFAULT_RESOLUTION_SETTINGS);
    }
    expect(sanitizeResolutionSettings({ ...valid, foo: 1 })).toEqual(valid);
    expect("foo" in sanitizeResolutionSettings({ ...valid, foo: 1 })).toBe(false);
    const noDarkEnergy = sanitizeResolutionSettings({ h0: 20, omegaM: 0.99 });
    expect(noDarkEnergy.omegaM).toBe(0.30966);
    expect(noDarkEnergy.h0).toBe(20);
    const kept = sanitizeResolutionSettings({ h0: 20, omegaM: 0.9 });
    expect(kept.omegaM).toBe(0.9);
    expect(kept.h0).toBe(20);
  });
});

describe("psfChoice and resolutionRequest", () => {
  it("the PSF comes from the chosen source or says what is missing", () => {
    expect(psfChoice(withSource("image"), IMAGE)).toEqual({
      fwhmArcsec: 0.9185,
      halfLightRadiusArcsec: 0.52,
      reference: "108 stars of this image (M)",
      missing: null,
    });
    expect(psfChoice(withSource("image"), { ...IMAGE, n_stars: 1 }).reference).toBe("1 star of this image (M)");
    expect(psfChoice(withSource("header"), IMAGE)).toEqual({
      fwhmArcsec: 1.1,
      halfLightRadiusArcsec: null,
      reference: "header card SEEING",
      missing: null,
    });
    expect(psfChoice(withSource("typed", 0.8), IMAGE)).toEqual({
      fwhmArcsec: 0.8,
      halfLightRadiusArcsec: null,
      reference: "typed value",
      missing: null,
    });
    expect(psfChoice(withSource("typed"), IMAGE)).toEqual({ ...NO_PSF, missing: "Type a PSF FWHM in the Resolution section." });
    expect(psfChoice(withSource("image"), null)).toEqual({
      ...NO_PSF,
      missing: "Measure the PSF of this image in the Resolution section.",
    });
    expect(psfChoice(withSource("image"), { ...IMAGE, fwhm_arcsec: null })).toEqual({
      ...NO_PSF,
      missing: "This image has no celestial WCS, so its PSF is known in pixels only: type the PSF FWHM in arcsec.",
    });
    expect(psfChoice(withSource("header"), null)).toEqual({
      ...NO_PSF,
      missing: "Measure the PSF of this image in the Resolution section.",
    });
    expect(psfChoice(withSource("header"), { ...IMAGE, header_values: [] })).toEqual({
      ...NO_PSF,
      missing: "This file has no SEEING, PSF_FWHM or PSFFWHM card: choose another PSF source.",
    });
  });

  it("the request carries the settings and drops unmeasured sizes", () => {
    const psf = { fwhmArcsec: 1.14, halfLightRadiusArcsec: null, reference: "typed value", missing: null };
    const settings = { ...DEFAULT_RESOLUTION_SETTINGS, redshift: 1 };
    expect(resolutionRequest(settings, psf, 0.2, { fwhmArcsec: 1.2, snr: 50 })).toEqual({
      psf_fwhm_arcsec: 1.14,
      psf_reference: "typed value",
      pixel_scale_arcsec: 0.2,
      source_fwhm_arcsec: 1.2,
      source_half_light_radius_arcsec: null,
      psf_half_light_radius_arcsec: null,
      snr: 50,
      redshift: 1,
      psf_fwhm_rel_err: 0.03,
      resolved_elements: 3,
      min_pixels_per_fwhm: 2,
      scale_of_interest_kpc: 1,
      h0_km_s_mpc: 67.66,
      omega_m: 0.30966,
    });
    const unmeasured = resolutionRequest(settings, psf, 0, { fwhmArcsec: 0, halfLightRadiusArcsec: Number.NaN, snr: Number.POSITIVE_INFINITY });
    expect(unmeasured.pixel_scale_arcsec).toBeNull();
    expect(unmeasured.source_fwhm_arcsec).toBeNull();
    expect(unmeasured.source_half_light_radius_arcsec).toBeNull();
    expect(unmeasured.snr).toBeNull();
  });

  it("header cards are read like the Rust side reads them", () => {
    expect(seeingCardsFromHeader({ SEEING: "0.85", PSF_FWHM: "'1.1'", FWHM: "4.5", PSFFWHM: "-1.0" })).toEqual([
      { key: "SEEING", value_arcsec: 0.85 },
      { key: "PSF_FWHM", value_arcsec: 1.1 },
    ]);
    expect(seeingCardsFromHeader({ PSFFWHM: " ' 0.7 ' " })).toEqual([{ key: "PSFFWHM", value_arcsec: 0.7 }]);
    expect(seeingCardsFromHeader({ SEEING: "60" })).toEqual([{ key: "SEEING", value_arcsec: 60 }]);
    const rejected: Record<string, string>[] = [{ SEEING: "'NaN'" }, { SEEING: "75.0" }, { SEEING: "''" }, { SEEING: "abc" }, {}];
    for (const header of rejected) {
      expect(seeingCardsFromHeader(header)).toEqual([]);
    }
  });

  it("the header source works before any star measurement", () => {
    const settings = withSource("header");
    expect(psfChoice(settings, null, [{ key: "SEEING", value_arcsec: 1.1 }])).toEqual({
      fwhmArcsec: 1.1,
      halfLightRadiusArcsec: null,
      reference: "header card SEEING",
      missing: null,
    });
    expect(psfChoice(settings, null, [])).toEqual({
      ...NO_PSF,
      missing: "This file has no SEEING, PSF_FWHM or PSFFWHM card: choose another PSF source.",
    });
    expect(psfChoice(settings, null).missing).toBe("Measure the PSF of this image in the Resolution section.");
    const cards = [{ key: "PSFFWHM", value_arcsec: 0.7 }];
    expect(psfChoice(settings, IMAGE, cards).reference).toBe("header card SEEING");
    const fromCards = psfChoice(settings, { ...IMAGE, header_values: [] }, cards);
    expect(fromCards.fwhmArcsec).toBe(0.7);
    expect(fromCards.reference).toBe("header card PSFFWHM");
    for (const settingsOfSource of [withSource("image"), withSource("typed", 0.8), withSource("typed")]) {
      for (const image of [IMAGE, null, { ...IMAGE, fwhm_arcsec: null }]) {
        expect(psfChoice(settingsOfSource, image, cards)).toEqual(psfChoice(settingsOfSource, image));
      }
    }
  });
});

describe("resolution rows and texts", () => {
  it("rows of a marginal source with a redshift", () => {
    expect(resolutionRows(CHECK)).toEqual([
      ["PSF FWHM", "1.14 arcsec = 9.38 kpc"],
      ["PSF from", "typed value"],
      ["Sampling", "5.70 px per FWHM"],
      ["Intrinsic size", "0.375 arcsec = 3.08 kpc (from the FWHM)"],
      ["Size floor", "0.285 arcsec = 2.35 kpc (0.25 x PSF FWHM)"],
      ["Resolution elements", "0.33 (resolved from 3)"],
      ["Signal-to-noise", "50.0"],
      ["Scale", "8.23 kpc/arcsec at z = 1 (1.65 kpc/px)"],
      ["Scale of interest", "1 kpc is 0.107 x the PSF FWHM"],
      ["Cosmology", CHECK.cosmology],
    ]);
  });

  it("rows without a redshift, a pixel scale or a signal-to-noise", () => {
    const check: ResolutionCheck = {
      ...CHECK,
      psf_fwhm_kpc: null,
      intrinsic_size_kpc: null,
      size_floor_kpc: null,
      angular_diameter_distance_mpc: null,
      kpc_per_arcsec: null,
      kpc_per_pixel: null,
      scale_of_interest_in_psf: null,
      redshift: null,
      pixels_per_fwhm: null,
      nyquist_sampled: null,
      snr: null,
    };
    expect(resolutionRows(check)).toEqual([
      ["PSF FWHM", "1.14 arcsec"],
      ["PSF from", "typed value"],
      ["Sampling", "not checked (no pixel scale)"],
      ["Intrinsic size", "0.375 arcsec (from the FWHM)"],
      ["Size floor", "0.285 arcsec (0.25 x PSF FWHM)"],
      ["Resolution elements", "0.33 (resolved from 3)"],
      ["Signal-to-noise", "not given"],
    ]);
  });

  it("an undersampled PSF and an unmeasurable floor are said in the rows", () => {
    const rows = (check: ResolutionCheck) => new Map(resolutionRows(check));
    expect(rows({ ...CHECK, nyquist_sampled: false, pixels_per_fwhm: 1.67, min_pixels_per_fwhm: 2 }).get("Sampling")).toBe(
      "1.67 px per FWHM, below the limit of 2",
    );
    expect(rows({ ...CHECK, size_floor_arcsec: null, floor_fraction: null }).get("Size floor")).toBe(
      "no size can be measured at this signal-to-noise",
    );
    expect(rows({ ...CHECK, size_basis: "half_light_radius" }).get("Intrinsic size")?.endsWith("(from half-light radii)")).toBe(true);
  });

  it("the scale-only answer has two rows", () => {
    const scaleOnly: ResolutionCheck = {
      ...CHECK,
      verdict: null,
      size_basis: null,
      psf_fwhm_arcsec: null,
      psf_reference: null,
      pixels_per_fwhm: null,
      nyquist_sampled: null,
      psf_area_px: null,
      intrinsic_size_arcsec: null,
      snr_floor_fraction: null,
      floor_fraction: null,
      size_floor_arcsec: null,
      resolution_elements: null,
      kpc_per_pixel: null,
      psf_fwhm_kpc: null,
      intrinsic_size_kpc: null,
      size_floor_kpc: null,
      scale_of_interest_in_psf: null,
      snr: null,
    };
    expect(resolutionRows(scaleOnly)).toEqual([
      ["Scale", "8.23 kpc/arcsec at z = 1"],
      ["Cosmology", COSMOLOGY],
    ]);
    expect(scaleText({ ...CHECK, kpc_per_arcsec: null })).toBeNull();
  });

  it("significant keeps three digits and prints -- for nothing", () => {
    expect(significant(9.383626644618662)).toBe("9.38");
    expect(significant(0.250290234995889)).toBe("0.25");
    expect(significant(1203.9)).toBe("1200");
    expect(significant(0.000123456)).toBe("0.000123");
    expect(significant(null)).toBe("--");
    expect(significant(undefined)).toBe("--");
    expect(significant(Number.NaN)).toBe("--");
    expect(significant(3.14159, 5)).toBe("3.1416");
  });

  it("the image PSF line", () => {
    expect(imagePsfText(PSF17)).toBe("3.51 px = 0.918 arcsec ± 0.13 px, 108 stars");
    expect(imagePsfText({ ...PSF17, fwhm_px: 2.03, fwhm_arcsec: null, fwhm_px_scatter: null, n_stars: 1 })).toBe("2.03 px, 1 star");
  });

  it("robustWidth is a median and 1.4826 times the MAD", () => {
    const width = robustWidth([4, 4.2, 3.8, 4.1, 9]);
    expect(width?.median).toBe(4.1);
    expect(Math.abs((width?.scatter as number) - 0.14826)).toBeLessThanOrEqual(1e-12);
    expect(width?.n).toBe(5);
    expect(robustWidth([4, 5])).toEqual({ median: 4.5, scatter: null, n: 2 });
    expect(robustWidth([4, null, Number.NaN, undefined])).toEqual({ median: 4, scatter: null, n: 1 });
    expect(robustWidth([])).toBeNull();
  });

  it("the SAGUI region budget compares pixels per region with the PSF area", () => {
    const input = {
      width: 100,
      height: 80,
      bands: [
        { filter: "F200W", pivot_um: 2, psf_fwhm_arcsec: 0.1, pixscale_arcsec: 0.036, psf_area_px: 17.485957337296277 },
        { filter: "F150W", pivot_um: 1.5 },
      ],
    } as SaguiInputInfo;
    const budget = saguiRegionBudget(input, " f200w ", 20);
    expect(budget?.pixelsPerRegion).toBe(400);
    expect(budget?.maxRegions).toBe(457);
    expect(budget?.enough).toBe(true);
    expect(budget?.filter).toBe("F200W");
    const crowded = saguiRegionBudget(input, "F200W", 500);
    expect(crowded?.pixelsPerRegion).toBe(16);
    expect(crowded?.enough).toBe(false);
    expect(saguiRegionBudget(input, "F150W", 20)).toBeNull();
    expect(saguiRegionBudget(input, "F444W", 20)).toBeNull();
    expect(saguiRegionBudget(input, "F200W", 0)).toBeNull();
    expect(saguiRegionBudget(input, "F200W", 2.5)).toBeNull();
  });
});
