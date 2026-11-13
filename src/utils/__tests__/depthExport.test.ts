import { describe, expect, it } from "vitest";
import { DEPTH_CSV_COLUMNS, DEPTH_WEIGHTS_NOTE, depthCsv, depthCsvFileName } from "../depthExport";
import { CSV_LINE_END } from "../catalogCsv";
import { provenanceLines, unitLines, withProvenance, type ProvenanceEntry } from "../csvProvenance";
import type { ImageDepth } from "../../shared/types/depth";

const CONTEXT = { fileName: "brick-r.fits", appVersion: "v0.8.0", exportedUtc: "2026-11-12T18:00:00Z" };

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

const ENTRIES: ProvenanceEntry[] = [
  ["software", "AstroBurst v0.8.0"],
  ["exported_utc", "2026-11-12T18:00:00Z"],
  ["file", "brick-r.fits"],
  ["definition", "5 sigma, empty apertures of radius 1.00 arcsec, 1.4826 x MAD of 498 sums, sources masked at 3 sigma, seed 1"],
  ["radius_arcsec", 1],
  ["radius_px", 3.8167938931297707],
  ["pixel_scale_arcsec", 0.262],
  ["n_requested", 500],
  ["n_placed", 500],
  ["n_used", 498],
  ["seed", 1],
  ["n_sigma", 5],
  ["detect_nsigma", 3],
  ["masked_fraction", 0.45],
  ["sigma_empty", 0.0118],
  ["sigma_clipped", 0.0117],
  ["sigma_formal", 0.0071],
  ["f_corr", 1.662],
  ["limit", 0.059],
  ["limit_ujy", 2.142e-7 * 1e6],
  ["limit_ab", 25.573],
  ["psf_fwhm_arcsec", null],
  ["psf_limit_formal", null],
  ["psf_limit_formal_ab", null],
  ["calibration", "BUNIT nanomaggy, zero point 22.5"],
  ["error_layer", "inverse variance from brick-invvar-r.fits"],
  ["mask_layer", "maskbits from brick-maskbits.fits"],
  ["aperture_weights", "5 x 5 sub-pixel samples per boundary pixel; formal error = sqrt(sum of (weight x sigma)^2)"],
  ["positions", "0-based pixels, integer pixel centres"],
];

const UNITS = { x_px: "pixel", y_px: "pixel", sum: "nanomaggy", formal_err: "nanomaggy" };

const BODY = ["x_px,y_px,sum,formal_err,usable", "10.25,20.5,0.012,0.007,true", "30,40,-0.004,,false"].join(CSV_LINE_END) + CSV_LINE_END;

describe("depth CSV", () => {
  it("the depth CSV starts with software, time and file, then the definition", () => {
    expect(ENTRIES).toHaveLength(29);
    expect(DEPTH_CSV_COLUMNS.map((c) => c.header)).toEqual(["x_px", "y_px", "sum", "formal_err", "usable"]);
    expect(DEPTH_WEIGHTS_NOTE).toBe("5 x 5 sub-pixel samples per boundary pixel; formal error = sqrt(sum of (weight x sigma)^2)");
    const text = depthCsv(DEPTH, CONTEXT);
    expect(text).toBe(withProvenance([...provenanceLines(ENTRIES), ...unitLines(UNITS)], BODY, CSV_LINE_END));
    const lines = text.split(CSV_LINE_END);
    expect(lines).toContain(
      "# definition: 5 sigma, empty apertures of radius 1.00 arcsec, 1.4826 x MAD of 498 sums, sources masked at 3 sigma, seed 1",
    );
    expect(lines).toContain("# unit sum: nanomaggy");
    expect(lines).toContain("# positions: 0-based pixels, integer pixel centres");
    expect(lines[0].startsWith("# software: AstroBurst v0.8.0")).toBe(true);
    expect(lines[1].startsWith("# exported_utc: 2026-11-12T18:00:00Z")).toBe(true);
    expect(lines[2].startsWith("# file: brick-r.fits")).toBe(true);
    expect(lines[3].startsWith("# definition: ")).toBe(true);
  });

  it("the unit of the sums follows the unit of the pixels", () => {
    const unitless = depthCsv({ ...DEPTH, bunit: null }, CONTEXT).split(CSV_LINE_END);
    expect(unitless).toContain("# unit sum: image unit x pix");
    expect(unitless).toContain("# unit formal_err: image unit x pix");
    const surface = depthCsv({ ...DEPTH, bunit: "MJy/sr" }, CONTEXT).split(CSV_LINE_END);
    expect(surface).toContain("# unit sum: MJy/sr x pix");
    expect(surface).toContain("# unit formal_err: MJy/sr x pix");
  });

  it("no aperture still gives a header row", () => {
    const lines = depthCsv({ ...DEPTH, apertures: [] }, CONTEXT).split(CSV_LINE_END).filter((line) => line !== "");
    expect(lines[lines.length - 1]).toBe("x_px,y_px,sum,formal_err,usable");
  });

  it("the file name follows the image", () => {
    expect(depthCsvFileName("C:/data/brick-r.fits.fz#hdu=1")).toBe("brick-r_depth.csv");
    expect(depthCsvFileName(null)).toBe("image_depth.csv");
  });
});
