import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { spectrum1dTables, tsoLightCurves, tsoOpen, tsoSpectrum, tsoWhtltCompare, whtltImport } from "../tso";
import type {
  SeriesKey,
  Spectrum1dTables,
  TsoCurve,
  TsoExposureCompleteness,
  TsoFile,
  TsoLightCurves,
  TsoLightCurvesRequest,
  TsoLimitsOrigin,
  TsoOpenResult,
  TsoPick,
  TsoSeriesSummary,
  TsoWhiteLightLog,
  WhtltComparison,
  WhtltImport,
  WhtltSeries,
  X1dIntsKind,
  X1dTableInfo,
} from "../../shared/types/tso";
import type { Spectrum1dWire } from "../../shared/types/spectrum1d";

const TABLES: Spectrum1dTables = {
  tables: [
    {
      hdu: 1,
      extver: 2,
      n_rows: 5,
      n_channels: 5,
      sporder: null,
      detector: "NRS1",
      slit_name: null,
      source_id: null,
      source_name: null,
      int_num: null,
      srctype: "POINT",
      label: "table 1 of 2 (EXTVER 2)",
    },
    {
      hdu: 2,
      extver: 1,
      n_rows: 5,
      n_channels: 5,
      sporder: null,
      detector: "NRS1",
      slit_name: null,
      source_id: null,
      source_name: null,
      int_num: null,
      srctype: "POINT",
      label: "table 2 of 2 (EXTVER 1)",
    },
  ],
};

describe("tso service", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("spectrum1dTables pins spectrum1d_tables_cmd with the path", async () => {
    typedInvokeMock.mockResolvedValue(TABLES);
    const res = await spectrum1dTables("C:/d/a_x1d.fits");
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("spectrum1d_tables_cmd", { path: "C:/d/a_x1d.fits" });
    expect(res).toBe(TABLES);
  });

  it("the series kind and the table row keep the shapes the backend serialises", () => {
    expectTypeOf<X1dIntsKind["kind"]>().toEqualTypeOf<"x1dints">();
    expectTypeOf<X1dIntsKind["layout"]>().toEqualTypeOf<"rows" | "extensions">();
    expectTypeOf<X1dTableInfo["sporder"]>().toEqualTypeOf<number | null>();
    expectTypeOf<X1dTableInfo["label"]>().toEqualTypeOf<string>();
    const series: X1dIntsKind = { kind: "x1dints", layout: "rows", n_series: 1, n_integrations: 240, n_channels: 431 };
    expect(Object.keys(series).sort()).toEqual(["kind", "layout", "n_channels", "n_integrations", "n_series"]);
    expect(Object.keys(TABLES.tables[0]).sort()).toEqual([
      "detector",
      "extver",
      "hdu",
      "int_num",
      "label",
      "n_channels",
      "n_rows",
      "slit_name",
      "source_id",
      "source_name",
      "sporder",
      "srctype",
    ]);
  });

  it("tsoOpen pins tso_open_cmd with the paths", async () => {
    const opened: TsoOpenResult = {
      set_id: "4b385da81d5c70b8",
      files: [
        {
          path: "C:/d/a_x1dints.fits",
          file_name: "a_x1dints.fits",
          sha256: "a5e7022d22e779fdf43b14fba6e7d23644c9547c920b6a52c13435c63706bfbc",
          level: 3,
          datamodel: "TSOMultiSpecModel",
          cal_ver: "1.19.1",
          exp_type: "NRS_BRIGHTOBJ",
          layout: "rows",
          nints: 240,
          intstart: null,
          intend: null,
          exsegnum: null,
          exsegtot: null,
          instrument: "NIRSPEC",
          detector: "NRS1",
          filter: "CLEAR",
          pupil: null,
          grating: "PRISM",
          subarray: "SUB2048",
          target: "WD 1856+534 b",
          expstart: 60061.44,
          outlier_detection: "COMPLETE",
          whitelight_log: { min_um: null, max_um: null },
          whtlt_path: null,
          n_extract1d: 1,
          data_bytes: 14496000,
        },
      ],
      series: [
        {
          key: { detector: "NRS1", sporder: 0 },
          label: "NRS1",
          n_integrations: 240,
          n_channels: 431,
          flux_unit: "Jy",
          time_scale: "bmjd_tdb",
          time_source: "TDB-MID column",
          time_first: 60061.4421733457,
          time_last: 60061.52706665472,
          wavelength_min_um: 0.5530469695008425,
          wavelength_max_um: 5.370352185327354,
          common_grid: true,
          outlier_detection: "COMPLETE",
          completeness: [{ exposure: "a", nints: 240, found: 240, missing: [], complete: true }],
          notes: ["240 integrations from 1 EXTRACT1D table(s) in 1 file(s)"],
          warnings: [],
        },
      ],
      notes: ["240 integrations from 1 EXTRACT1D table(s) in 1 file(s)"],
      warnings: [],
    };
    typedInvokeMock.mockResolvedValue(opened);
    const res = await tsoOpen(["C:/d/a_x1dints.fits", "C:/d/b_x1dints.fits"]);
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("tso_open_cmd", {
      paths: ["C:/d/a_x1dints.fits", "C:/d/b_x1dints.fits"],
    });
    expect(res).toBe(opened);
    expect(Object.keys(opened).sort()).toEqual(["files", "notes", "series", "set_id", "warnings"]);
    expect(Object.keys(opened.files[0])).toHaveLength(26);
    expect(Object.keys(opened.series[0])).toHaveLength(16);
  });

  it("tsoSpectrum pins tso_spectrum_cmd and turns wire nulls into NaN", async () => {
    const series: SeriesKey = { detector: "NRS1", sporder: -1 };
    const pick: TsoPick = { kind: "mean", first: 0, last: 7 };
    const wire: Spectrum1dWire = {
      source: { kind: "x1dints", paths: ["C:/d/a_x1dints.fits"], detector: "NRS1", sporder: -1, pick },
      axis: {
        kind: "wave",
        ctype: "WAVELENGTH",
        unit: "um",
        header_unit: "um",
        header_scale: 1,
        values: [1, null],
        crval: 1,
        cdelt: null,
        crpix: 1,
        rest_wavelength_um: null,
        rest_frequency_hz: null,
        specsys: null,
        velosys: null,
        notes: [],
        tabulated: true,
      },
      wavelength_frame: "vacuum",
      precision: "f64",
      flux: [1.5, null],
      err: null,
      dq: [0, 1],
      dq_table: "jwst",
      flux_unit: "Jy",
      n_rows: 2,
      cards: { DETECTOR: "NRS1" },
      notes: ["time series NRS1: mean of integrations 1 to 8 of 240"],
      wavelength_unit_source: "file",
    };
    typedInvokeMock.mockResolvedValue(wire);
    const doc = await tsoSpectrum(["C:/d/a_x1dints.fits"], series, pick);
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("tso_spectrum_cmd", {
      paths: ["C:/d/a_x1dints.fits"],
      series: { detector: "NRS1", sporder: -1 },
      pick: { kind: "mean", first: 0, last: 7 },
    });
    expect(doc.flux).toEqual([1.5, Number.NaN]);
    expect(doc.err).toBeNull();
    expect(doc.axis.values).toEqual([1, Number.NaN]);
    expect(doc.axis.crval).toBe(1);
    expect(doc.axis.cdelt).toBeNaN();
    expect(doc.source).toEqual(wire.source);
    expect(doc.dq).toEqual([0, 1]);
  });

  it("the series summary and the pick keep the shapes the backend serialises", () => {
    expectTypeOf<TsoPick>().toEqualTypeOf<
      { kind: "integration"; index: number } | { kind: "median" } | { kind: "mean"; first: number; last: number }
    >();
    expectTypeOf<TsoSeriesSummary["time_scale"]>().toEqualTypeOf<"bmjd_tdb" | "mjd_utc" | "index">();
    expectTypeOf<TsoExposureCompleteness["missing"]>().toEqualTypeOf<[number, number][]>();
    expectTypeOf<TsoFile["whitelight_log"]>().toEqualTypeOf<TsoWhiteLightLog | null>();
    expectTypeOf<TsoSeriesSummary["key"]>().toEqualTypeOf<SeriesKey>();
    expectTypeOf<SeriesKey["sporder"]>().toEqualTypeOf<number | null>();
    expectTypeOf<TsoSeriesSummary["time_first"]>().toEqualTypeOf<number | null>();
    expectTypeOf<TsoOpenResult["files"]>().toEqualTypeOf<TsoFile[]>();
  });

  it("tsoLightCurves pins tso_light_curves_cmd with the series and the request", async () => {
    const white: TsoCurve = {
      band_um: [0.5530469695008425, 5.370352185327354],
      limits_origin: "open",
      flux: [1.5, null],
      flux_err: [0.25, null],
      n_used: [316, 316],
      npixels_mean: [7, null],
    };
    const curves: TsoLightCurves = {
      set_id: "4b385da81d5c70b8",
      label: "NRS1",
      series: { detector: "NRS1", sporder: 0 },
      mode: "masked",
      time_scale: "bmjd_tdb",
      time: [60061.4421733457, 60061.44253],
      mjd_utc: [60061.44133176829, null],
      int_num: [1, 2],
      row_index: [0, 1],
      flux_unit: "Jy",
      white,
      bins: [{ ...white, band_um: [0.6, 1.0], limits_origin: "user" }],
      dropped: { nan_time: 0, repeated_time: 0 },
      rejected_elements: [{ index: 230, wavelength_um: 3.0178602767154326, reason: "time-median flux" }],
      notes: ["316 of 431 channels are valid in every integration"],
      warnings: [],
    };
    typedInvokeMock.mockResolvedValue(curves);
    const request: TsoLightCurvesRequest = { mode: "masked", bins: [[0.6, 1.0]], reject_elements_sigma: 100 };
    const res = await tsoLightCurves(["C:/d/a_x1dints.fits"], { detector: "NRS1", sporder: 0 }, request);
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("tso_light_curves_cmd", {
      paths: ["C:/d/a_x1dints.fits"],
      series: { detector: "NRS1", sporder: 0 },
      request: { mode: "masked", bins: [[0.6, 1.0]], reject_elements_sigma: 100 },
    });
    expect(res).toBe(curves);
    expect(res.white.flux).toEqual([1.5, null]);
    expect(res.mjd_utc).toEqual([60061.44133176829, null]);
    expect(Object.keys(curves).sort()).toEqual([
      "bins",
      "dropped",
      "flux_unit",
      "int_num",
      "label",
      "mjd_utc",
      "mode",
      "notes",
      "rejected_elements",
      "row_index",
      "series",
      "set_id",
      "time",
      "time_scale",
      "warnings",
      "white",
    ]);
    expect(Object.keys(white).sort()).toEqual(["band_um", "flux", "flux_err", "limits_origin", "n_used", "npixels_mean"]);
  });

  it("the light-curve types keep the shapes the backend serialises", () => {
    expectTypeOf<TsoCurve["limits_origin"]>().toEqualTypeOf<"open" | "user" | "file log" | "built-in default">();
    expectTypeOf<TsoCurve["flux"]>().toEqualTypeOf<(number | null)[]>();
    expectTypeOf<TsoCurve["band_um"]>().toEqualTypeOf<[number, number]>();
    expectTypeOf<TsoLightCurves["time"]>().toEqualTypeOf<number[]>();
    expectTypeOf<TsoLightCurves["mjd_utc"]>().toEqualTypeOf<(number | null)[]>();
    expectTypeOf<TsoLightCurves["mode"]>().toEqualTypeOf<"pipeline" | "masked">();
    expectTypeOf<TsoLightCurves["series"]>().toEqualTypeOf<SeriesKey>();
    expectTypeOf<TsoLightCurvesRequest["bins"]>().toEqualTypeOf<[number, number][] | undefined>();
    expectTypeOf<TsoLightCurvesRequest["wavelength_min_um"]>().toEqualTypeOf<number | null | undefined>();
  });

  it("whtltImport and tsoWhtltCompare pin their commands and arguments", async () => {
    const imported: WhtltImport = {
      path: "C:/d/a_whtlt.ecsv",
      file_name: "a_whtlt.ecsv",
      schema: "current",
      series: [
        {
          column: "whitelight_flux_NRS1",
          detector: "NRS1",
          sporder: null,
          mjd_utc: [60061.44133176829, 60061.44168696],
          bmjd_tdb: [60061.4421733457, null],
          flux: [0.16369951048163586, 0.1639],
          flux_unit: "Jy",
        },
      ],
      meta: { instrument: "NIRSPEC", number_of_integrations: 2 },
      notes: [],
    };
    typedInvokeMock.mockResolvedValue(imported);
    const table = await whtltImport("C:/d/a_whtlt.ecsv");
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("whtlt_import_cmd", { path: "C:/d/a_whtlt.ecsv" });
    expect(table).toBe(imported);
    expect(table.series[0].bmjd_tdb).toEqual([60061.4421733457, null]);
    expect(Object.keys(imported).sort()).toEqual(["file_name", "meta", "notes", "path", "schema", "series"]);
    expect(Object.keys(imported.series[0]).sort()).toEqual([
      "bmjd_tdb",
      "column",
      "detector",
      "flux",
      "flux_unit",
      "mjd_utc",
      "sporder",
    ]);

    const compared: WhtltComparison[] = [
      {
        series: { detector: "NRS1", sporder: 0 },
        label: "NRS1",
        column: "whitelight_flux_NRS1",
        n_whtlt: 2,
        n_joined: 2,
        max_rel_diff: 2.5e-16,
        within_1e4: true,
        limits_um: [0.5530469695008425, 5.370352185327354],
        limits_origin: "open",
        notes: [],
      },
      {
        series: null,
        label: null,
        column: "whitelight_flux_NRS2",
        n_whtlt: 2,
        n_joined: 0,
        max_rel_diff: null,
        within_1e4: false,
        limits_um: null,
        limits_origin: null,
        notes: [
          "column whitelight_flux_NRS2: 0 series of these files match it (detector NRS2, order any); nothing compared",
        ],
      },
    ];
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue(compared);
    const entries = await tsoWhtltCompare(["C:/d/a_x1dints.fits"], "C:/d/a_whtlt.ecsv");
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("tso_whtlt_compare_cmd", {
      paths: ["C:/d/a_x1dints.fits"],
      whtltPath: "C:/d/a_whtlt.ecsv",
    });
    expect(entries).toBe(compared);
    expect(entries[1].max_rel_diff).toBeNull();
    expect(Object.keys(compared[0]).sort()).toEqual([
      "column",
      "label",
      "limits_origin",
      "limits_um",
      "max_rel_diff",
      "n_joined",
      "n_whtlt",
      "notes",
      "series",
      "within_1e4",
    ]);
  });

  it("the whtlt types keep the shapes the backend serialises", () => {
    expectTypeOf<WhtltSeries["bmjd_tdb"]>().toEqualTypeOf<(number | null)[]>();
    expectTypeOf<WhtltSeries["mjd_utc"]>().toEqualTypeOf<number[]>();
    expectTypeOf<WhtltSeries["flux_unit"]>().toEqualTypeOf<string | null>();
    expectTypeOf<WhtltImport["schema"]>().toEqualTypeOf<"legacy" | "current">();
    expectTypeOf<WhtltImport["meta"]>().toEqualTypeOf<Record<string, unknown> | null>();
    expectTypeOf<WhtltComparison["series"]>().toEqualTypeOf<SeriesKey | null>();
    expectTypeOf<WhtltComparison["max_rel_diff"]>().toEqualTypeOf<number | null>();
    expectTypeOf<WhtltComparison["limits_um"]>().toEqualTypeOf<[number, number] | null>();
    expectTypeOf<WhtltComparison["limits_origin"]>().toEqualTypeOf<TsoLimitsOrigin | null>();
  });
});
