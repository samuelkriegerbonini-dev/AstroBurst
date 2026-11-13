import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { CompareBandInfo, ComparePreset, CompareProbe, CompareReport, CompareRequest, CompareRow } from "../../../shared/types/catalogCompare";
import { compareRequestOf, formFromPreset, type CompareRun } from "../../../utils/catalogCompare";
import { formatWithUncertainty } from "../../../utils/formatUncertainty";
import CompareSummary from "../CompareSummary";

const LEGACY_R: ComparePreset = {
  id: "legacy_dr10",
  label: "Legacy Surveys DR10",
  band: "r",
  mapping: {
    id: "ls_id",
    ra: "ra",
    dec: "dec",
    flux: "apflux_r_5",
    flux_unit: "nanomaggy",
    zero_point: null,
    flux_err: null,
    flux_ivar: "apflux_ivar_r_5",
    class: "type",
    missing_value: null,
  },
  aperture: { radius_arcsec: 2, radius_px: null, annulus_arcsec: null, annulus_px: null },
  cuts: [
    { column: "type", op: "==", value: "PSF" },
    { column: "maskbits", op: "==", value: 0 },
    { column: "fracflux_r", op: "<", value: 0.01 },
  ],
  min_snr: null,
  max_snr: null,
  min_neighbour_arcsec: null,
  notes: [],
};

const LEGACY_G: ComparePreset = {
  ...LEGACY_R,
  band: "g",
  mapping: { ...LEGACY_R.mapping, flux: "apflux_g_5", flux_ivar: "apflux_ivar_g_5" },
  cuts: [
    { column: "type", op: "==", value: "PSF" },
    { column: "maskbits", op: "==", value: 0 },
    { column: "fracflux_g", op: "<", value: 0.01 },
  ],
};

const BAND: CompareBandInfo = {
  file_name: "legacysurvey-0532m280-image-r.fits.fz",
  label: "r",
  filter: { dataset: "DECAM", filter: "r", pivot_um: 0.6418, source: "header FILTER" },
  calibration_origin: "BUNIT nanomaggy, zero point 22.5",
  jy_per_native_unit: 3.6307805477010175e-6,
  bunit: "nanomaggy",
  pixel_scale_arcsec: 0.262,
  aperture_radius_px: 7.633587786259541,
  annulus_px: null,
  err_origin: {
    role: "sigma",
    source: "inverse variance from legacysurvey-0532m280-invvar-r.fits.fz",
    assumed: false,
    n_no_data: 12,
  },
  dq_source: null,
  warnings: [],
};

const ROWS: CompareRow[] = [
  {
    row: 1,
    id: "10995300315432397",
    ra_deg: 53.2001,
    dec_deg: -27.9002,
    x: null,
    y: null,
    class: "REX",
    flux_cat_jy: 1e-4,
    flux_err_cat_jy: null,
    snr_cat: null,
    flux_native: null,
    flux_err_native: null,
    flux_jy: null,
    flux_err_jy: null,
    ratio: null,
    masked_fraction: null,
    used: false,
    reason: "cut type == PSF",
  },
  {
    row: 2,
    id: "10995300315432398",
    ra_deg: 53.2101,
    dec_deg: -27.9102,
    x: 3333.8115,
    y: 625.4192,
    class: "PSF",
    flux_cat_jy: 2.871799e-4,
    flux_err_cat_jy: 1.0e-7,
    snr_cat: 2871.799,
    flux_native: 79.1,
    flux_err_native: 0.03,
    flux_jy: 2.872e-4,
    flux_err_jy: 1.1e-7,
    ratio: 1.00007,
    masked_fraction: 0,
    used: true,
    reason: null,
  },
  {
    row: 3,
    id: "row 3",
    ra_deg: 53.2201,
    dec_deg: -27.9202,
    x: 10.5,
    y: 20.25,
    class: null,
    flux_cat_jy: 5e-5,
    flux_err_cat_jy: 2e-7,
    snr_cat: 250,
    flux_native: 13.5,
    flux_err_native: 0.05,
    flux_jy: 4.9e-5,
    flux_err_jy: 1.8e-7,
    ratio: 0.98,
    masked_fraction: 0.25,
    used: false,
    reason: "aperture touches pixels without data",
  },
  {
    row: 4,
    id: "a,b",
    ra_deg: 53.2301,
    dec_deg: -27.9302,
    x: 100,
    y: 200,
    class: "PSF",
    flux_cat_jy: 1e-4,
    flux_err_cat_jy: 1e-7,
    snr_cat: 1000,
    flux_native: 27.5,
    flux_err_native: 0.03,
    flux_jy: 9.99e-5,
    flux_err_jy: 1.1e-7,
    ratio: 0.999,
    masked_fraction: 0,
    used: true,
    reason: null,
  },
];

const REPORT: CompareReport = {
  n_table: 69,
  n_on_image: 69,
  n_after_cuts: 21,
  n_measured: 21,
  n_used: 21,
  ratio_median: 1.00007,
  ratio_nmad: 0.00021,
  ratio_median_err: 0.00012,
  n_ratio: 21,
  ratio_offset_mag: -0.000076,
  err_ratio_median: 0.98434,
  err_ratio_nmad: 0.0054,
  n_err: 21,
  implied_zero_point_ab: 22.50008,
  implied_zero_point_nmad: 0.0012,
  n_zero_point: 21,
  rows: ROWS,
  rejected: [
    { reason: "cut type == PSF", count: 33 },
    { reason: "cut maskbits == 0", count: 14 },
    { reason: "cut fracflux_r < 0.01", count: 1 },
  ],
  aperture: { radius_px: 7.633587786259541, radius_arcsec: 2, annulus_px: null, pixel_scale_arcsec: 0.262, local_sky: false },
  band: BAND,
  calibration_origin: "BUNIT nanomaggy, zero point 22.5",
  preset: "legacy_dr10",
  catalogue_band: "r",
  table: {
    path: "C:/cat/C1b_ls_dr10_brick_0532m280.csv",
    file_name: "C1b_ls_dr10_brick_0532m280.csv",
    format: "csv",
    n_rows: 69,
  },
  notes: ["note one", "note two"],
  warnings: ["w1"],
  elapsed_ms: 12,
};

const numeric = (name: string) => ({ name, numeric: true, unit: null });
const text = (name: string) => ({ name, numeric: false, unit: null });

const PROBE: CompareProbe = {
  table_path: "C:/cat/c1b.csv",
  file_name: "c1b.csv",
  format: "csv",
  n_rows: 69,
  columns: [
    text("ls_id"),
    numeric("ra"),
    numeric("dec"),
    text("type"),
    numeric("maskbits"),
    numeric("fracflux_g"),
    numeric("fracflux_r"),
    numeric("apflux_g_5"),
    numeric("apflux_ivar_g_5"),
    numeric("apflux_r_5"),
    numeric("apflux_ivar_r_5"),
  ],
  presets: [{ ...LEGACY_G, notes: ["apflux_g_5: circular aperture of 2.0 arcsec radius, a plain sum with no local sky"] }, LEGACY_R],
  notes: [],
};

const IMG = "C:/d/brick-r.fits";

const BUILT = compareRequestOf(formFromPreset(LEGACY_R), IMG, PROBE);
const REQUEST = BUILT.request as CompareRequest;

const escaped = (value: string) =>
  value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#x27;");

const markup = (run: CompareRun) => renderToStaticMarkup(createElement(CompareSummary, { run }));
const summaryOf = (report: CompareReport, presetEdited = false) => markup({ report, request: REQUEST, presetEdited });

function cardOf(html: string, key: string): string {
  const start = html.indexOf(`data-card="${key}"`);
  expect(start, `card ${key}`).toBeGreaterThanOrEqual(0);
  const next = html.indexOf("data-card=", start + 1);
  return html.slice(start, next < 0 ? undefined : next);
}

describe("CompareSummary", () => {
  it("builds the request of the fixtures", () => {
    expect(BUILT.error).toBeNull();
    expect(BUILT.request).not.toBeNull();
  });

  it("prints the ratio with its error, scatter and N, and the offset in percent and in mag", () => {
    const html = summaryOf(REPORT);
    expect(html).toContain("data-compare-summary");
    expect(html).toContain(escaped(`${formatWithUncertainty(1.00007, 0.00012)} (scatter ${formatWithUncertainty(0.00021, null)}, N = 21)`));
    expect(html).toContain("+0.007 percent, -0.0001 mag");
    expect(html).toContain("69 rows in the table, 69 on the image, 21 after the cuts, 21 measured, 21 used");
    expect(html).toContain("0.984 (scatter 0.005, N = 21)");
    expect(html).toContain("22.500 AB (scatter 0.001 mag, N = 21)");
    expect(html).toContain("r = 7.63 px (2.00 arcsec at 0.2620 arcsec per px), no local sky");
  });

  it("says what was compared and with which calibration", () => {
    const html = summaryOf(REPORT);
    expect(html).toContain("C1b_ls_dr10_brick_0532m280.csv, preset legacy_dr10, catalogue band r");
    expect(html).toContain("legacysurvey-0532m280-image-r.fits.fz, filter DECAM r");
    expect(html).toContain("BUNIT nanomaggy, zero point 22.5");
    expect(summaryOf(REPORT, true)).toContain("preset legacy_dr10 (edited)");
    expect(summaryOf({ ...REPORT, preset: null })).toContain("C1b_ls_dr10_brick_0532m280.csv, custom columns, catalogue band r");
    expect(summaryOf({ ...REPORT, band: { ...BAND, filter: null } })).toContain(">legacysurvey-0532m280-image-r.fits.fz</div>");
    expect(summaryOf({ ...REPORT, catalogue_band: null })).toContain("catalogue band --");
  });

  it("lists every reason with its count", () => {
    const html = summaryOf(REPORT);
    expect(html).toContain("33 x cut type == PSF");
    expect(html).toContain("14 x cut maskbits == 0");
    expect(html).toContain("1 x cut fracflux_r &lt; 0.01");
    expect(cardOf(summaryOf({ ...REPORT, rejected: [] }), "rejected")).toContain(">none</div>");
  });

  it("an image without calibration has no ratio and says why", () => {
    const html = summaryOf({
      ...REPORT,
      ratio_median: null,
      ratio_nmad: null,
      ratio_median_err: null,
      n_ratio: 0,
      ratio_offset_mag: null,
      err_ratio_median: null,
      err_ratio_nmad: null,
      n_err: 0,
      calibration_origin: null,
    });
    expect(html).toContain("not available");
    expect(html).toContain("none: the image has no flux calibration in Jy, so no ratio");
    expect(cardOf(html, "errors")).toContain(">--</div>");
    expect(cardOf(html, "zero-point")).toContain("22.500 AB");
  });

  it("shows the warnings and folds the notes", () => {
    const html = summaryOf(REPORT);
    expect(html).toContain("w1");
    expect(html).toMatch(/<summary[^>]*>notes \(2\)<\/summary>/);
    expect(summaryOf({ ...REPORT, notes: [] })).not.toContain("<details");
  });
});
