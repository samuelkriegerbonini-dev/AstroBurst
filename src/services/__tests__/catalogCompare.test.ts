import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));

import { compareWithCatalogue, probeCatalogueTable } from "../catalogCompare";
import type {
  CompareMapping,
  CompareProbe,
  CompareReport,
  CompareRequest,
} from "../../shared/types/catalogCompare";

const TABLE_PATH = "C:/d/c1b.csv";

const MAPPING: CompareMapping = {
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
};

const PROBE: CompareProbe = {
  table_path: TABLE_PATH,
  file_name: "c1b.csv",
  format: "csv",
  n_rows: 69,
  columns: [
    { name: "ls_id", numeric: false, unit: null },
    { name: "ra", numeric: true, unit: "deg" },
  ],
  presets: [
    {
      id: "legacy_dr10",
      label: "Legacy Surveys DR10",
      band: "r",
      mapping: MAPPING,
      aperture: { radius_arcsec: 2, radius_px: null, annulus_arcsec: null, annulus_px: null },
      cuts: [
        { column: "type", op: "==", value: "PSF" },
        { column: "maskbits", op: "==", value: 0 },
        { column: "fracflux_r", op: "<", value: 0.01 },
      ],
      min_snr: null,
      max_snr: null,
      min_neighbour_arcsec: null,
      notes: ["the catalogue leaves pixels of zero inverse variance out of the sum"],
    },
  ],
  notes: ["column 'ls_id': integers beyond 2^53 are kept as text so that identifiers stay exact"],
};

const REQUEST: CompareRequest = {
  band: { path: "C:/d/legacy_r.fits", label: "r", dataset: "legacy_dr10" },
  table_path: TABLE_PATH,
  preset: "legacy_dr10",
  catalogue_band: "r",
  mapping: MAPPING,
  aperture: { radius_arcsec: 2, radius_px: null, annulus_arcsec: null, annulus_px: [20, 30] },
  cuts: [{ column: "maskbits", op: "==", value: 0 }],
  min_snr: 5,
  max_snr: null,
  min_neighbour_arcsec: 2,
  exclude_dq: null,
};

const REPORT: CompareReport = {
  n_table: 69,
  n_on_image: 40,
  n_after_cuts: 20,
  n_measured: 18,
  n_used: 15,
  ratio_median: 0.9996,
  ratio_nmad: 0.0002,
  ratio_median_err: 0.00006,
  n_ratio: 15,
  ratio_offset_mag: 0.0004,
  err_ratio_median: 0.98,
  err_ratio_nmad: 0.01,
  n_err: 15,
  implied_zero_point_ab: 22.5,
  implied_zero_point_nmad: 0.001,
  n_zero_point: 15,
  rows: [
    {
      row: 1,
      id: "10995300315432397",
      ra_deg: 150,
      dec_deg: 2,
      x: 10.5,
      y: null,
      class: "PSF",
      flux_cat_jy: 0.000287,
      flux_err_cat_jy: null,
      snr_cat: 2832,
      flux_native: 79.1,
      flux_err_native: 0.03,
      flux_jy: 0.000287,
      flux_err_jy: null,
      ratio: 0.9996,
      masked_fraction: 0,
      used: true,
      reason: null,
    },
  ],
  rejected: [{ reason: "off the image", count: 29 }],
  aperture: { radius_px: 7.63, radius_arcsec: 2, annulus_px: null, pixel_scale_arcsec: 0.262, local_sky: false },
  band: {
    file_name: "legacy_r.fits",
    label: "r",
    filter: { dataset: "legacy_dr10", filter: "r", pivot_um: 0.6432, source: "survey filter table" },
    calibration_origin: "nanomaggy",
    jy_per_native_unit: 3.631e-6,
    bunit: "nanomaggy",
    pixel_scale_arcsec: 0.262,
    aperture_radius_px: 7.63,
    annulus_px: null,
    err_origin: { role: "invvar", source: "legacy_r_invvar.fits", assumed: false, n_no_data: 0 },
    dq_source: null,
    warnings: [],
  },
  calibration_origin: "nanomaggy",
  preset: "legacy_dr10",
  catalogue_band: "r",
  table: { path: TABLE_PATH, file_name: "c1b.csv", format: "csv", n_rows: 69 },
  notes: [],
  warnings: [],
  elapsed_ms: 12,
};

describe("probeCatalogueTable", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins catalog_compare_probe_cmd with the table path and returns the probe unchanged", async () => {
    typedInvokeMock.mockResolvedValue(PROBE);
    const probe = await probeCatalogueTable(TABLE_PATH);
    expect(typedInvokeMock).toHaveBeenCalledWith("catalog_compare_probe_cmd", { tablePath: TABLE_PATH });
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(probe).toBe(PROBE);
    expectTypeOf(probe).toEqualTypeOf<CompareProbe>();
  });

  it("pins catalog_compare_cmd with the request unchanged and returns the report", async () => {
    typedInvokeMock.mockResolvedValue(REPORT);
    const report = await compareWithCatalogue(REQUEST);
    expect(typedInvokeMock).toHaveBeenCalledWith("catalog_compare_cmd", { request: REQUEST });
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock.mock.calls[0][1].request).toBe(REQUEST);
    expect(report).toBe(REPORT);
    expectTypeOf(report).toEqualTypeOf<CompareReport>();
  });

  it("keeps the request and report fixtures in step with the contract", () => {
    expectTypeOf(REQUEST).toEqualTypeOf<CompareRequest>();
    expectTypeOf(REPORT).toEqualTypeOf<CompareReport>();
    expectTypeOf<CompareReport["table"]["format"]>().toEqualTypeOf<"csv" | "ecsv">();
    expectTypeOf<CompareRequest["mapping"]>().toEqualTypeOf<CompareMapping>();
  });
});
