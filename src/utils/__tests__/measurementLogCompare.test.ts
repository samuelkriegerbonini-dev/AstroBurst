import { describe, expect, it } from "vitest";
import type {
  CompareBandInfo,
  ComparePreset,
  CompareProbe,
  CompareReport,
  CompareRequest,
  CompareRow,
} from "../../shared/types/catalogCompare";
import { compareRequestOf, formFromPreset } from "../catalogCompare";
import { MeasurementLogCore, measurementLogCsv } from "../measurementLog";
import { catalogCompareEntry, COMPARE_LOG_NOTE } from "../measurementLogCompare";

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

const IMG = "C:/d/legacysurvey-0532m280-image-r.fits.fz#hdu=1";

function probeFor(preset: ComparePreset, textColumns: string[] = []): CompareProbe {
  const m = preset.mapping;
  const used = [m.id, m.ra, m.dec, m.flux, m.flux_err, m.flux_ivar, m.class, ...preset.cuts.map((c) => c.column)];
  const names = [...new Set(used.filter((name): name is string => name !== null))];
  return {
    table_path: "C:/cat/t.csv",
    file_name: "t.csv",
    format: "csv",
    n_rows: 3,
    columns: names.map((name) => ({ name, numeric: name !== m.id && !textColumns.includes(name), unit: null })),
    presets: [preset],
    notes: [],
  };
}

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

function sentRequest(): CompareRequest {
  const result = compareRequestOf(formFromPreset(LEGACY_R), IMG, probeFor(LEGACY_R, ["type"]));
  if (result.request === null) throw new Error(result.error);
  return result.request;
}

const REQUEST = sentRequest();

const PROV = { file: "legacysurvey-0532m280-image-r.fits.fz", image: "original" };

describe("catalogCompareEntry", () => {
  it("catalogCompareEntry logs the statistics with their N and what was compared", () => {
    const entry = catalogCompareEntry(PROV, REPORT, { request: REQUEST, presetEdited: false });
    expect(entry.kind).toBe("catalog_compare");
    expect(entry.file).toBe("legacysurvey-0532m280-image-r.fits.fz");
    expect(entry.image).toBe("original");
    expect(entry.dq).toBe("off");
    expect(entry.unit).toBeNull();
    expect(entry.source).toBe('{"kind":"catalogue","table":"C1b_ls_dr10_brick_0532m280.csv"}');
    const expectedParams = {
      table: "C1b_ls_dr10_brick_0532m280.csv",
      table_format: "csv",
      preset: "legacy_dr10",
      preset_edited: false,
      catalogue_band: "r",
      image_filter: "DECAM r",
      ra_column: "ra",
      dec_column: "dec",
      id_column: "ls_id",
      flux_column: "apflux_r_5",
      flux_unit: "nanomaggy",
      zero_point: null,
      error_column: "apflux_ivar_r_5",
      error_kind: "flux_ivar",
      class_column: "type",
      missing_value: null,
      radius_px: 7.633587786259541,
      radius_arcsec: 2,
      annulus_inner_px: null,
      annulus_outer_px: null,
      pixel_scale_arcsec: 0.262,
      cuts: "type == PSF; maskbits == 0; fracflux_r < 0.01",
      min_snr: null,
      max_snr: null,
      min_neighbour_arcsec: null,
      calibration: "BUNIT nanomaggy, zero point 22.5",
    };
    expect(entry.params).toEqual(expectedParams);
    expect(Object.keys(entry.params)).toEqual(Object.keys(expectedParams));
    const expectedValues = {
      ratio_median: 1.00007,
      ratio_median_err: 0.00012,
      ratio_nmad: 0.00021,
      n_ratio: 21,
      ratio_offset_mag: -0.000076,
      err_ratio_median: 0.98434,
      err_ratio_nmad: 0.0054,
      n_err: 21,
      implied_zero_point_ab: 22.50008,
      implied_zero_point_nmad: 0.0012,
      n_zero_point: 21,
      n_table: 69,
      n_on_image: 69,
      n_after_cuts: 21,
      n_measured: 21,
      n_used: 21,
      rejected: "33 x cut type == PSF; 14 x cut maskbits == 0; 1 x cut fracflux_r < 0.01",
    };
    expect(entry.values).toEqual(expectedValues);
    expect(Object.keys(entry.values)).toEqual(Object.keys(expectedValues));
    expect(entry.notes).toEqual(["w1", COMPARE_LOG_NOTE]);
  });

  it("the flag handling follows the request and the mask of the image", () => {
    const masked = { ...REPORT, band: { ...BAND, dq_source: "DQ layer of the file" } };
    const excluding = { request: { ...REQUEST, exclude_dq: true }, presetEdited: false };
    expect(catalogCompareEntry(PROV, masked, excluding).dq).toBe("excluded");
    expect(catalogCompareEntry(PROV, REPORT, excluding).dq).toBe("requested");
    expect(catalogCompareEntry(PROV, masked, { request: REQUEST, presetEdited: false }).dq).toBe("off");
  });

  it("an edited preset, a custom comparison, a local sky and a missing calibration are told apart", () => {
    const opts = { request: REQUEST, presetEdited: false };
    expect(catalogCompareEntry(PROV, REPORT, { ...opts, presetEdited: true }).params.preset_edited).toBe(true);
    expect(catalogCompareEntry(PROV, { ...REPORT, preset: null }, opts).params.preset).toBeNull();
    const annulus = catalogCompareEntry(
      PROV,
      { ...REPORT, aperture: { ...REPORT.aperture, annulus_px: [28.55, 44.21], local_sky: true } },
      opts,
    );
    expect(annulus.params.annulus_inner_px).toBe(28.55);
    expect(annulus.params.annulus_outer_px).toBe(44.21);
    const uncalibrated = catalogCompareEntry(PROV, { ...REPORT, ratio_median: null }, opts);
    expect(uncalibrated.values.ratio_median).toBeNull();
    expect(uncalibrated.values.implied_zero_point_ab).toBe(22.50008);
    expect(catalogCompareEntry(PROV, REPORT, { ...opts, request: { ...REQUEST, cuts: [] } }).params.cuts).toBeNull();
    expect(catalogCompareEntry(PROV, { ...REPORT, rejected: [] }, opts).values.rejected).toBeNull();
    expect(catalogCompareEntry({ ...PROV, file: null }, REPORT, opts).file).toBe("legacysurvey-0532m280-image-r.fits.fz");
  });

  it("the entry goes through the log and its CSV", () => {
    const entry = catalogCompareEntry(PROV, REPORT, { request: REQUEST, presetEdited: false });
    expect(new MeasurementLogCore().append(entry).kind).toBe("catalog_compare");
    const header = measurementLogCsv([{ ...entry, id: "e0", timestamp_utc: "2026-11-12T00:00:00.000Z" }]).split("\r\n")[0];
    expect(header).toContain("p_table,p_table_format,p_preset,p_preset_edited");
    expect(header).toContain("v_ratio_median,v_ratio_median_err,v_ratio_nmad,v_n_ratio");
  });
});
