import { describe, expect, it } from "vitest";
import type { TsoLightCurves, WhtltComparison, WhtltImport, WhtltSeries } from "../../shared/types/tso";
import { APP_WHITE_DOC_ID, WHTLT_OVERLAY, comparisonRowText, defaultWhtltPath, whtltDocs, whtltSeriesFor } from "../tsoWhtlt";

const NRS1: WhtltSeries = {
  column: "whitelight_flux_NRS1",
  detector: "NRS1",
  sporder: null,
  mjd_utc: [60000.098, 60000.198],
  bmjd_tdb: [60000.1, 60000.2],
  flux: [10, 11],
  flux_unit: "Jy",
};

const NRS2: WhtltSeries = {
  column: "whitelight_flux_NRS2",
  detector: "NRS2",
  sporder: null,
  mjd_utc: [60000.099, 60000.199],
  bmjd_tdb: [60000.101, 60000.201],
  flux: [20, 21],
  flux_unit: "Jy",
};

const IMPORTED: WhtltImport = {
  path: "C:/d/a_whtlt.ecsv",
  file_name: "a_whtlt.ecsv",
  schema: "current",
  series: [NRS1, NRS2],
  meta: null,
  notes: [],
};

const LEGACY: WhtltImport = {
  path: "C:/d/b_whtlt.ecsv",
  file_name: "b_whtlt.ecsv",
  schema: "legacy",
  series: [
    {
      column: "whitelight_flux",
      detector: "MIRIMAGE",
      sporder: null,
      mjd_utc: [60000.098, 60000.198],
      bmjd_tdb: [null, null],
      flux: [10, 11],
      flux_unit: "Jy",
    },
  ],
  meta: null,
  notes: [],
};

function series(column: string, detector: string | null, sporder: number | null): WhtltSeries {
  return { ...NRS1, column, detector, sporder };
}

function table(...items: WhtltSeries[]): WhtltImport {
  return { ...IMPORTED, series: items };
}

function result(extra: Partial<TsoLightCurves> = {}): TsoLightCurves {
  const time = [60000.1, 60000.2, 60000.3, 60000.4];
  const flux = [10, 11, 12, 13];
  return {
    set_id: "abc",
    label: "NRS1",
    series: { detector: "NRS1", sporder: 0 },
    mode: "pipeline",
    time_scale: "bmjd_tdb",
    time,
    mjd_utc: time.map((t) => t - 0.002),
    int_num: [1, 2, 3, 4],
    row_index: [0, 1, 2, 3],
    flux_unit: "Jy",
    white: {
      band_um: [1, 2],
      limits_origin: "open",
      flux,
      flux_err: flux.map(() => 0.5),
      n_used: flux.map(() => 5),
      npixels_mean: flux.map(() => 7),
    },
    bins: [],
    dropped: { nan_time: 0, repeated_time: 0 },
    rejected_elements: [],
    notes: [],
    warnings: [],
    ...extra,
  };
}

const CURVES = result();
const PATHS = ["C:/d/a_x1dints.fits"];

const E1_ROW: WhtltComparison = {
  series: { detector: "NRS1", sporder: 0 },
  label: "NRS1",
  column: "whitelight_flux_NRS1",
  n_whtlt: 240,
  n_joined: 240,
  max_rel_diff: 1.863700988171452e-16,
  within_1e4: true,
  limits_um: [0.5530469695008425, 5.370352185327354],
  limits_origin: "open",
  notes: [],
};

describe("tsoWhtlt", () => {
  it("finds the table next to a level 3 file", () => {
    expect(defaultWhtltPath([{ whtlt_path: null }, { whtlt_path: "C:/d/a_whtlt.ecsv" }])).toBe("C:/d/a_whtlt.ecsv");
    expect(defaultWhtltPath([{ whtlt_path: "C:/d/first_whtlt.ecsv" }, { whtlt_path: "C:/d/a_whtlt.ecsv" }])).toBe("C:/d/first_whtlt.ecsv");
    expect(defaultWhtltPath([{ whtlt_path: null }, { whtlt_path: null }])).toBeNull();
    expect(defaultWhtltPath([])).toBeNull();
  });

  it("picks the column of a series by detector and order, and nothing when it is ambiguous", () => {
    expect(whtltSeriesFor(IMPORTED, { detector: "nrs1", sporder: 0 })).toBe(NRS1);
    expect(whtltSeriesFor(IMPORTED, { detector: "NRS2", sporder: null })).toBe(NRS2);
    expect(whtltSeriesFor(LEGACY, { detector: "NRCALONG", sporder: 1 })).toBeNull();
    const bare = series("whitelight_flux", null, null);
    expect(whtltSeriesFor(table(bare), { detector: "MIRIMAGE", sporder: 1 })).toBe(bare);
    expect(whtltSeriesFor(table(bare), { detector: "NRS1", sporder: null })).toBe(bare);
    const order1 = series("whitelight_flux_order_1", null, 1);
    const order2 = series("whitelight_flux_order_2", null, 2);
    expect(whtltSeriesFor(table(order1, order2), { detector: "NIS", sporder: 2 })).toBe(order2);
    expect(whtltSeriesFor(table(order1, order2), { detector: "NIS", sporder: 3 })).toBeNull();
    expect(whtltSeriesFor(table(order1, order2), { detector: "NIS", sporder: null })).toBeNull();
    expect(whtltSeriesFor(IMPORTED, { detector: "NRS3", sporder: 0 })).toBeNull();
    expect(whtltSeriesFor(table(series("a", null, null), series("b", null, null)), { detector: "NRS1", sporder: 0 })).toBeNull();
    expect(whtltSeriesFor(table(), { detector: "NRS1", sporder: 0 })).toBeNull();
  });

  it("draws the archived curve on the time axis of the series, beside the app's white light", () => {
    const docs = whtltDocs(IMPORTED, CURVES, PATHS);
    expect(docs.map((doc) => doc.id)).toEqual(["white:app", "whtlt:whitelight_flux_NRS1"]);
    expect(docs.map((doc) => doc.overlay)).toEqual(["white light and whtlt", "white light and whtlt"]);
    expect(APP_WHITE_DOC_ID).toBe("white:app");
    expect(WHTLT_OVERLAY).toBe("white light and whtlt");

    const [app, archived] = docs;
    expect(app.label).toBe("white light (app)");
    expect(app.value).toEqual([10, 11, 12, 13]);
    expect(app.time).toEqual(CURVES.time);
    expect(app.source).toEqual({ kind: "tso", paths: PATHS, detector: "NRS1", sporder: 0, band_um: [1, 2], mode: "pipeline" });

    expect(archived).toEqual({
      id: "whtlt:whitelight_flux_NRS1",
      label: "archived whtlt (whitelight_flux_NRS1)",
      quantity: "flux",
      unit: "Jy",
      timeScale: "bmjd_tdb",
      time: [60000.1, 60000.2],
      value: [10, 11],
      err: [null, null],
      pointRef: [0, 1],
      flags: [[], []],
      source: { kind: "table", path: "C:/d/a_whtlt.ecsv", time_column: "BJD_TDB", value_column: "whitelight_flux_NRS1" },
      notes: ["pipeline white light read from a_whtlt.ecsv"],
      overlay: "white light and whtlt",
    });

    const utc = whtltDocs(IMPORTED, result({ time_scale: "mjd_utc" }), PATHS)[1];
    expect(utc.time).toEqual([60000.098, 60000.198]);
    expect(utc.timeScale).toBe("mjd_utc");
    expect(utc.source).toEqual({ kind: "table", path: "C:/d/a_whtlt.ecsv", time_column: "MJD_UTC", value_column: "whitelight_flux_NRS1" });

    const gap = table({ ...NRS1, mjd_utc: [1, 2, 3], bmjd_tdb: [60000.1, null, 60000.3], flux: [10, 11, 12] });
    const kept = whtltDocs(gap, CURVES, PATHS)[1];
    expect(kept.time).toEqual([60000.1, 60000.3]);
    expect(kept.value).toEqual([10, 12]);
    expect(kept.pointRef).toEqual([0, 2]);
    expect(kept.err).toEqual([null, null]);
    expect(kept.flags).toEqual([[], []]);

    const unitless = whtltDocs(table({ ...NRS1, flux_unit: null }), CURVES, PATHS)[1];
    expect(unitless.unit).toBeNull();

    expect(whtltDocs(IMPORTED, result({ time_scale: "index" }), PATHS)).toEqual([]);
    expect(whtltDocs(IMPORTED, result({ series: { detector: "NRS3", sporder: 0 } }), PATHS)).toEqual([]);

    const miri = { detector: "MIRIMAGE", sporder: 1 };
    expect(whtltDocs(LEGACY, result({ series: miri }), PATHS)).toEqual([]);
    const legacyUtc = whtltDocs(LEGACY, result({ series: miri, time_scale: "mjd_utc" }), PATHS);
    expect(legacyUtc.map((doc) => doc.id)).toEqual(["white:app", "whtlt:whitelight_flux"]);
    expect(legacyUtc[1].time).toEqual([60000.098, 60000.198]);
    expect(legacyUtc[1].notes).toEqual(["pipeline white light read from b_whtlt.ecsv"]);
  });

  it("words one row of the comparison", () => {
    expect(comparisonRowText(E1_ROW)).toEqual({
      column: "whitelight_flux_NRS1",
      series: "NRS1",
      rows: "240 of 240",
      difference: "1.86e-16",
      verdict: "matches (within 1e-4)",
      limits: "0.553 to 5.37 um (open)",
    });

    const differs = comparisonRowText({ ...E1_ROW, max_rel_diff: 0.0011169888054512525, within_1e4: false });
    expect(differs.difference).toBe("0.00112");
    expect(differs.verdict).toBe("differs");

    expect(
      comparisonRowText({
        ...E1_ROW,
        series: null,
        label: null,
        n_whtlt: 8,
        n_joined: 0,
        max_rel_diff: null,
        within_1e4: false,
        limits_um: null,
        limits_origin: null,
      }),
    ).toEqual({
      column: "whitelight_flux_NRS1",
      series: "no single series",
      rows: "0 of 8",
      difference: "--",
      verdict: "not compared",
      limits: null,
    });

    const unjoined = comparisonRowText({ ...E1_ROW, n_joined: 0, max_rel_diff: null, within_1e4: false });
    expect(unjoined.verdict).toBe("not compared");
    expect(unjoined.series).toBe("NRS1");
    expect(unjoined.limits).toBe("0.553 to 5.37 um (open)");
    expect(comparisonRowText({ ...E1_ROW, series: null, within_1e4: true }).verdict).toBe("not compared");
    expect(comparisonRowText({ ...E1_ROW, limits_um: [5, 12], limits_origin: "built-in default" }).limits).toBe("5 to 12 um (built-in default)");
  });
});
