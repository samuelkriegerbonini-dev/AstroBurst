import { describe, it, expect } from "vitest";
import { catalogueMagnitude, splitZeroPointPoints, zeroPointSplitOf } from "../zeroPointPlot";
import type { CrossMatchEntry, CrossMatchResult, MatchedCatalogRow, ZeroPointStatus } from "../../shared/types/catalog";

describe("splitZeroPointPoints", () => {
  it("groups the points by the status the fit gave them and spans the line over the fitted and clipped x", () => {
    const split = splitZeroPointPoints([10, 12, 14, null, 16], [-10, -8, -5, -4, -4.05], 20, ["used", "used", "clipped", "used", "used"]);
    expect(split.fitted).toEqual({ x: [10, 12, 16], y: [-10, -8, -4.05] });
    expect(split.outliers).toEqual({ x: [14], y: [-5] });
    expect(split.saturated).toEqual({ x: [], y: [] });
    expect(split.line).toEqual({ x: [10, 16], y: [-10, -4] });
  });

  it("draws the stars the fit clipped as clipped and the ones it kept as fitted, whatever their distance to the line", () => {
    const split = splitZeroPointPoints([14, 15, 16], [14 - 25 - 0.03, 15 - 25 - 0.2, 16 - 25], 25, ["clipped", "used", "used"]);
    expect(split.outliers).toEqual({ x: [14], y: [14 - 25 - 0.03] });
    expect(split.fitted).toEqual({ x: [15, 16], y: [15 - 25 - 0.2, 16 - 25] });
  });

  it("draws the saturated stars the fit left out apart and keeps them out of the line", () => {
    const split = splitZeroPointPoints([11, 12, 13], [-14, -13, -12], 25, ["saturated", "used", "used"]);
    expect(split.saturated).toEqual({ x: [11], y: [-14] });
    expect(split.fitted).toEqual({ x: [12, 13], y: [-13, -12] });
    expect(split.line).toEqual({ x: [12, 13], y: [-13, -12] });
  });

  it("skips the matches without a status and draws no line without a zero point", () => {
    const split = splitZeroPointPoints([10, 12], [-10, -5], null, [null, "saturated"]);
    expect(split.fitted).toEqual({ x: [], y: [] });
    expect(split.outliers).toEqual({ x: [], y: [] });
    expect(split.saturated).toEqual({ x: [12], y: [-5] });
    expect(split.line).toBeNull();
  });

  it("returns no line without a fitted or clipped point", () => {
    expect(splitZeroPointPoints([null], [null], 20, ["used"]).line).toBeNull();
    expect(splitZeroPointPoints([12], [-13], 25, ["saturated"]).line).toBeNull();
  });

  it("applies the fitted colour term so stars on the colour relation lie on the line", () => {
    const cat = [12, 13, 14, 15];
    const bpRp = [0.5, 1, 2, 2.5];
    const inst = cat.map((g, i) => g - 25 - 0.3 * bpRp[i]);
    const split = splitZeroPointPoints(cat, inst, 25, ["used", "used", "used", "used"], bpRp, 0.3);
    expect(split.outliers.x).toEqual([]);
    expect(split.noColour.x).toEqual([]);
    expect(split.fitted.x).toEqual(cat);
    split.fitted.y.forEach((y, i) => expect(y).toBeCloseTo(cat[i] - 25, 12));
    expect(split.line).toEqual({ x: [12, 15], y: [-13, -10] });
  });

  it("sets the stars the colour fit left out for lack of BP-RP aside, uncorrected", () => {
    const split = splitZeroPointPoints([12, 13, 14], [-13.3, -12, -11.6], 25, ["used", "no_colour", "no_colour"], [1, null, NaN], 0.3);
    expect(split.fitted.x).toEqual([12]);
    expect(split.fitted.y[0]).toBeCloseTo(-13, 12);
    expect(split.outliers.x).toEqual([]);
    expect(split.noColour).toEqual({ x: [13, 14], y: [-12, -11.6] });
    expect(split.line).toEqual({ x: [12, 12], y: [-13, -13] });
  });

  it("applies the colour term to a saturated star with BP-RP and leaves one without BP-RP as measured", () => {
    const split = splitZeroPointPoints([12, 13], [-13.3, -12.1], 25, ["saturated", "saturated"], [1, null], 0.3);
    expect(split.saturated.x).toEqual([12, 13]);
    expect(split.saturated.y[0]).toBeCloseTo(-13, 12);
    expect(split.saturated.y[1]).toBe(-12.1);
    expect(split.noColour).toEqual({ x: [], y: [] });
  });

  it("ignores the colour column when no colour term was fitted", () => {
    const split = splitZeroPointPoints([12, 13], [-13, -12], 25, ["used", "used"], [1, null], null);
    expect(split.fitted).toEqual({ x: [12, 13], y: [-13, -12] });
    expect(split.noColour).toEqual({ x: [], y: [] });
  });
});

const ZERO_POINT = 25;

function zeroPointMatch(index: number, residual: number, zpStatus: ZeroPointStatus | null): CrossMatchEntry {
  const g = 12 + index / 8;
  return {
    star: {
      x: 10 + index, y: 20, ra: 274.7, dec: -13.8, flux: 1000, fwhm: 3.2, snr: 80, mag_inst: g - ZERO_POINT - residual, mag_ab: null,
      saturated: zpStatus === "saturated", aperture_radius: 4.8,
    },
    row: { id: `Gaia DR3 ${index}`, ra: 274.7, dec: -13.8, g, bp: null, rp: null, bp_rp: null },
    sep_arcsec: 0.2,
    d_ra_arcsec: 0.01,
    d_dec_arcsec: 0.004,
    zp_status: zpStatus,
  };
}

function crossMatchResult(overrides: Partial<CrossMatchResult> = {}): CrossMatchResult {
  const residuals = [0, 0, 0.02, -0.02, 1.5];
  const statuses: ZeroPointStatus[] = ["saturated", "used", "used", "used", "clipped"];
  const matches = residuals.map((residual, index) => zeroPointMatch(index, residual, statuses[index]));
  return {
    matches,
    sources: matches.map((m) => m.star),
    astrometry: { median_d_ra_arcsec: 0.01, median_d_dec_arcsec: 0.004, rms_arcsec: 0.052, n: matches.length },
    zero_point: {
      zp: ZERO_POINT, zp_err: 0.012, colour_coeff: null, rms: 0.02, n_used: 3, n_rejected: 1, n_without_colour: 0,
      band: "G", colour_term_used: false,
    },
    photcal_present: false,
    epoch_year: 2024.5,
    n_detected: matches.length,
    n_catalog: 120,
    band: "G",
    match_radius_arcsec: 2,
    elapsed_ms: 9,
    warnings: [],
    ...overrides,
  };
}

describe("catalogueMagnitude", () => {
  it("picks the catalogue magnitude of the band", () => {
    const row: MatchedCatalogRow = { id: "a", ra: 1, dec: 2, g: 12, bp: 12.5, rp: 11.4, bp_rp: 1.1 };
    expect(catalogueMagnitude(row, "G")).toBe(12);
    expect(catalogueMagnitude(row, "BP")).toBe(12.5);
    expect(catalogueMagnitude(row, "RP")).toBe(11.4);
    expect(catalogueMagnitude({ ...row, rp: null }, "RP")).toBeNull();
  });
});

describe("zeroPointSplitOf", () => {
  it("builds the split of a cross-match result from its matches, its band and its fit", () => {
    const split = zeroPointSplitOf(crossMatchResult());
    expect(split.fitted.x).toEqual([12.125, 12.25, 12.375]);
    expect(split.outliers.x).toEqual([12.5]);
    expect(split.saturated.x).toEqual([12]);
    expect(split.noColour.x).toEqual([]);
    expect(split.line).toEqual({ x: [12.125, 12.5], y: [-12.875, -12.5] });
    expect(split.fitted.y[1]).toBe(12.25 - 25 - 0.02);

    const red = zeroPointSplitOf(crossMatchResult({ band: "RP" }));
    expect(red.fitted).toEqual({ x: [], y: [] });
    expect(red.outliers).toEqual({ x: [], y: [] });
    expect(red.saturated).toEqual({ x: [], y: [] });
    expect(red.noColour).toEqual({ x: [], y: [] });
    expect(red.line).toBeNull();

    const unfitted = zeroPointSplitOf(crossMatchResult({ zero_point: null }));
    expect(unfitted.fitted).toEqual(split.fitted);
    expect(unfitted.outliers).toEqual(split.outliers);
    expect(unfitted.saturated).toEqual(split.saturated);
    expect(unfitted.noColour).toEqual(split.noColour);
    expect(unfitted.line).toBeNull();
  });
});
