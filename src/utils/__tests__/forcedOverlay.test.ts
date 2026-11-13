import { describe, it, expect } from "vitest";
import { FORCED_LAYER_ID, forcedApertureLayer } from "../forcedOverlay";
import type { ForcedBand, ForcedCell, ForcedPhotometryResult } from "../../shared/types/forcedPhotometry";

const LEGACY_RADIUS_PX = 7.633587786259541;

function cell(overrides: Partial<ForcedCell> = {}): ForcedCell {
  return {
    x: 100.25,
    y: 200.5,
    on_image: true,
    error: null,
    flux_native: 10,
    flux_err_native: 1,
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
    flags_or: null,
    saturated: false,
    fwhm_px: 3.74,
    ...overrides,
  };
}

function band(index: number, path: string, overrides: Partial<ForcedBand> = {}): ForcedBand {
  const fileName = path.slice(path.lastIndexOf("/") + 1);
  return {
    index,
    path,
    file_name: fileName,
    label: fileName,
    error: null,
    filter: null,
    photcal: null,
    calibration_origin: null,
    jy_per_native_unit: null,
    bunit: null,
    pixel_scale_arcsec: 0.262,
    aperture_radius_px: null,
    annulus_px: null,
    aperture_correction: null,
    err_origin: null,
    dq_source: null,
    f_corr: null,
    n_measured: 0,
    n_failed: 0,
    warnings: [],
    mag_system: null,
    ...overrides,
  };
}

function result(bands: ForcedBand[], cellsBySource: (ForcedCell | null)[][]): ForcedPhotometryResult {
  return {
    bands,
    sources: cellsBySource.map((cells, i) => ({ id: `s${i}`, ra_deg: 53.1, dec_deg: -28.1, cells })),
    aperture: { radius_arcsec: 2, correction: "none" },
    limit_sigma: 3,
    elapsed_ms: 0,
    warnings: [],
  };
}

const R_BAND = band(0, "C:/d/r.fits", { label: "r", aperture_radius_px: LEGACY_RADIUS_PX, annulus_px: null, error: null });
const CELLS: (ForcedCell | null)[][] = [
  [cell({ x: 10.25, y: 20.5 })],
  [cell({ x: 30, y: 40, flux_jy: 5e-8, mag_ab: null, mag_ab_err: null, upper_limit_jy: 3e-7, upper_limit_ab: 25.207197, snr: 0.5 })],
  [cell({ x: null, y: null, on_image: false, flux_jy: null, flux_err_jy: null, mag_ab: null, mag_ab_err: null, snr: null })],
  [cell({ x: 50, y: 60, error: "the aperture holds no pixel with data", flux_jy: null, flux_err_jy: null, mag_ab: null, mag_ab_err: null, snr: null })],
];
const RESULT = result([R_BAND], CELLS);

describe("forcedApertureLayer", () => {
  it("one marker per aperture on the image, at the measured pixel with the radius of the band", () => {
    expect(FORCED_LAYER_ID).toBe("forced-apertures");
    const layer = forcedApertureLayer(RESULT, 0, null)!;
    expect(layer).not.toBeNull();
    expect(layer.apertures).toHaveLength(3);
    const expected = [
      [10.25, 20.5],
      [30, 40],
      [50, 60],
    ];
    layer.apertures.forEach((marker, i) => {
      expect(marker.x).toBe(expected[i][0]);
      expect(marker.y).toBe(expected[i][1]);
      expect(marker.rAp).toBe(7.633587786259541);
      expect(marker.skyIn).toBeNaN();
      expect(marker.skyOut).toBeNaN();
      expect(marker.label).toBe("");
    });
    expect(layer.highlightIndex).toBeNull();
  });

  it("the selected source is highlighted and named", () => {
    const layer = forcedApertureLayer(RESULT, 0, 3)!;
    expect(layer.highlightIndex).toBe(2);
    expect(layer.apertures.map((m) => m.label)).toEqual(["", "", "s3"]);
    const offImage = forcedApertureLayer(RESULT, 0, 2)!;
    expect(offImage.highlightIndex).toBeNull();
    expect(offImage.apertures.map((m) => m.label)).toEqual(["", "", ""]);
    const first = forcedApertureLayer(RESULT, 0, 0)!;
    expect(first.highlightIndex).toBe(0);
    expect(first.apertures.map((m) => m.label)).toEqual(["s0", "", ""]);
  });

  it("a sky annulus is passed in pixels", () => {
    const withAnnulus = result([{ ...R_BAND, annulus_px: [28.55, 44.21] }], CELLS);
    const layer = forcedApertureLayer(withAnnulus, 0, null)!;
    expect(layer.apertures).toHaveLength(3);
    for (const marker of layer.apertures) {
      expect(marker.skyIn).toBe(28.55);
      expect(marker.skyOut).toBe(44.21);
      expect(marker.rAp).toBe(LEGACY_RADIUS_PX);
    }
  });

  it("a band that was not measured has no layer", () => {
    expect(forcedApertureLayer(result([{ ...R_BAND, error: "no WCS" }], CELLS), 0, null)).toBeNull();
    expect(forcedApertureLayer(result([{ ...R_BAND, aperture_radius_px: null }], CELLS), 0, null)).toBeNull();
    expect(forcedApertureLayer(result([{ ...R_BAND, aperture_radius_px: 0 }], CELLS), 0, null)).toBeNull();
    expect(forcedApertureLayer(result([{ ...R_BAND, aperture_radius_px: Number.NaN }], CELLS), 0, null)).toBeNull();
    expect(forcedApertureLayer(RESULT, 1, null)).toBeNull();
    expect(forcedApertureLayer(RESULT, -1, null)).toBeNull();
  });
});
