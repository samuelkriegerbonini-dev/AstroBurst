import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ForcedBand, ForcedPhotometryResult } from "../../../shared/types/forcedPhotometry";
import ForcedBandCards from "../ForcedBandCards";
import { MISSING, bandCardRows } from "../forcedBandCardsModel";

const FULL: ForcedBand = {
  index: 0,
  path: "C:/d/brick-r.fits",
  file_name: "legacysurvey-0532m280-image-r.fits.fz",
  label: "r",
  error: null,
  filter: { dataset: "DECAM", filter: "r", pivot_um: 0.64176, source: "header FILTER" },
  photcal: null,
  calibration_origin: "BUNIT nanomaggy, zero point 22.5",
  jy_per_native_unit: 3.630780547701009e-6,
  bunit: "nanomaggy",
  pixel_scale_arcsec: 0.262,
  aperture_radius_px: 7.633587786259541,
  annulus_px: null,
  aperture_correction: {
    radius_px: 7.633587786259541,
    reference_radius_px: 13.358778625954198,
    factor: 1.04127,
    scatter: 0.0056,
    n_stars: 31,
    n_candidates: 67,
  },
  err_origin: { role: "inverse_variance", source: "legacysurvey-0532m280-invvar-r.fits.fz", assumed: false, n_no_data: 14346 },
  dq_source: "legacysurvey-0532m280-maskbits.fits.fz",
  f_corr: 1.647,
  n_measured: 68,
  n_failed: 1,
  warnings: ["2 of 69 apertures of r hold pixels without data: their flux is a lower limit"],
  flag_names: [
    { value: 1, name: "NPRIMARY" },
    { value: 1024, name: "BAILOUT" },
  ],
};

const BARE: ForcedBand = {
  ...FULL,
  filter: null,
  calibration_origin: null,
  jy_per_native_unit: null,
  bunit: null,
  aperture_correction: null,
  err_origin: null,
  dq_source: null,
  f_corr: null,
  n_failed: 0,
  n_measured: 69,
  flag_names: undefined,
};

const FAILED: ForcedBand = {
  index: 1,
  path: "C:/d/x.fits",
  file_name: "x.fits",
  label: "x.fits",
  error: "x.fits has no celestial WCS: sky positions cannot be placed on it",
  filter: null,
  photcal: null,
  calibration_origin: null,
  jy_per_native_unit: null,
  bunit: null,
  pixel_scale_arcsec: null,
  aperture_radius_px: null,
  annulus_px: null,
  aperture_correction: null,
  err_origin: null,
  dq_source: null,
  f_corr: null,
  n_measured: 0,
  n_failed: 0,
  warnings: [],
};

const ROW_LABELS = ["File", "Filter", "Pixel scale", "Aperture", "Calibration", "Error layer", "Flags", "Aperture correction", "Noise factor", "Measured"];

const rowOf = (band: ForcedBand, label: string) => Object.fromEntries(bandCardRows(band, 69))[label];

describe("ForcedBandCards", () => {
  it("a measured band says what it is", () => {
    expect(bandCardRows(FULL, 69)).toEqual([
      ["File", "legacysurvey-0532m280-image-r.fits.fz"],
      ["Filter", "DECAM r, pivot 0.6418 um (header FILTER)"],
      ["Pixel scale", "0.262 arcsec/px"],
      ["Aperture", "7.634 px radius, no local sky"],
      ["Calibration", "BUNIT nanomaggy, zero point 22.5: 3.631 uJy per nanomaggy"],
      ["Error layer", "inverse variance from legacysurvey-0532m280-invvar-r.fits.fz, 14346 pixels without data"],
      ["Flags", "legacysurvey-0532m280-maskbits.fits.fz, 2 named bits"],
      ["Aperture correction", "x 1.041 to 13.36 px (scatter 0.0056, 31 of 67 sources)"],
      ["Noise factor", "f_corr 1.647"],
      ["Measured", "68 of 69 positions, 1 failed"],
    ]);
  });

  it("a bare band says what is missing", () => {
    const rows = bandCardRows(BARE, 69);
    expect(rows.map(([label]) => label)).toEqual(ROW_LABELS);
    expect(rows).toContainEqual(["Filter", "not identified: the band stays off the SED"]);
    expect(rows).toContainEqual(["Calibration", "none: fluxes stay in image unit"]);
    expect(rows).toContainEqual(["Error layer", "none: the fluxes have no error"]);
    expect(rows).toContainEqual(["Flags", "none"]);
    expect(rows).toContainEqual(["Aperture correction", "none"]);
    expect(rows).toContainEqual(["Noise factor", "not given: limits use the formal error"]);
    expect(rows).toContainEqual(["Measured", "69 of 69 positions"]);

    const annulus: ForcedBand = { ...BARE, annulus_px: [28.55, 44.21] };
    expect(rowOf(annulus, "Aperture")).toBe("7.634 px radius, sky annulus 28.55 to 44.21 px");
    expect(rowOf(annulus, "Error layer")).toBe("none: errors come from the sky annulus");
  });

  it("a Vega zero point and an assumed layer are told", () => {
    const band: ForcedBand = {
      ...FULL,
      calibration_origin: "user zero point 25.0 Vega",
      jy_per_native_unit: null,
      err_origin: { role: "inverse_variance", source: "des-dr2_A_r_extn3.fits", assumed: true, n_no_data: 1 },
      flag_names: [{ value: 1, name: "NPRIMARY" }],
    };
    expect(rowOf(band, "Calibration")).toBe("user zero point 25.0 Vega: no Jy and no AB magnitude");
    expect(rowOf(band, "Error layer")).toBe("inverse variance from des-dr2_A_r_extn3.fits (assumed), 1 pixel without data");
    expect(rowOf(band, "Flags")).toBe("legacysurvey-0532m280-maskbits.fits.fz, 1 named bit");
  });

  it("cards follow the bands of the result and mark the one on screen", () => {
    const result: ForcedPhotometryResult = {
      bands: [FULL, FAILED],
      sources: [],
      aperture: { radius_arcsec: 2, correction: "none" },
      limit_sigma: 3,
      elapsed_ms: 420,
      warnings: [],
    };
    const markup = renderToStaticMarkup(createElement(ForcedBandCards, { result, currentPath: "C:/d/brick-r.fits" }));
    expect(markup).toContain(">Bands<");
    const first = markup.indexOf('data-forced-band-card="0"');
    const second = markup.indexOf('data-forced-band-card="1"');
    expect(first).toBeGreaterThan(-1);
    expect(second).toBeGreaterThan(first);
    const measured = markup.slice(first, second);
    const failed = markup.slice(second);
    expect(measured).toContain('data-band-state="measured"');
    expect(measured).toContain("data-on-screen");
    expect(measured).toContain(">on screen<");
    expect(measured).toContain(FULL.warnings[0]);
    for (const [label, value] of bandCardRows(FULL, 0)) {
      expect(measured).toContain(`>${label}<`);
      expect(measured).toContain(`title="${value}"`);
    }
    expect(failed).toContain('data-band-state="failed"');
    expect(failed).not.toContain("data-on-screen");
    expect(failed).toMatch(/<[^>]*data-band-error[^>]*role="alert"[^>]*>Not measured: x\.fits has no celestial WCS: sky positions cannot be placed on it</);
    for (const label of ROW_LABELS) expect(failed).not.toContain(`>${label}<`);
    expect(markup.match(/data-on-screen/g)).toHaveLength(1);

    expect(MISSING).toBe("--");
    expect(rowOf({ ...FULL, pixel_scale_arcsec: null }, "Pixel scale")).toBe("-- arcsec/px");
    expect(rowOf({ ...FULL, aperture_radius_px: null }, "Aperture")).toBe("-- px radius, no local sky");
    expect(rowOf({ ...FULL, pixel_scale_arcsec: Number.NaN }, "Pixel scale")).toBe("-- arcsec/px");
  });
});
