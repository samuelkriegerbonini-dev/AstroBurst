import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { LightCurveTableColumn, LightCurveTableInput } from "../lightCurveDoc";
import {
  BMJD_REASON,
  JD_MJD_BOUNDARY,
  NO_TIME_COLUMN_NOTE,
  NO_VALUE_COLUMN_NOTE,
  guessTimeScale,
  lightCurveDocsFromTable,
  planLightCurveTable,
} from "../lightCurveTable";

function col(name: string, values: (number | null)[], unit: string | null = null): LightCurveTableColumn {
  return { name, unit, values };
}

function text(name: string): LightCurveTableColumn {
  return { name, unit: null, values: null };
}

function table(...columns: LightCurveTableColumn[]): LightCurveTableInput {
  return { columns };
}

function appCsv(overrides: Record<string, (number | null)[]> = {}): LightCurveTableInput {
  const numbers: [string, (number | null)[]][] = [
    ["jd_mid", [2460310.5, 2460310.6]],
    ["bjd_tdb", [2460310.5041, 2460310.6041]],
    ["hjd_utc", [2460310.5033, 2460310.6033]],
    ["diff_mag", [1.5, 1.6]],
    ["diff_err", [0.01, 0.02]],
    ["target_flux", [500, 480]],
    ["target_err", [5, 5]],
    ["rel_flux", [0.25, 0.23]],
    ["rel_flux_err", [0.003, 0.003]],
  ];
  return table(col("index", [0, 1]), text("file"), ...numbers.map(([name, values]) => col(name, overrides[name] ?? values)));
}

function oneDetector(): LightCurveTableInput {
  return table(
    col("MJD_UTC_NRS1", [60061.4413, 60061.4417]),
    col("BJD_TDB_NRS1", [60061.4422, 60061.4425]),
    col("whitelight_flux_NRS1", [0.1637, 0.164], "Jy"),
  );
}

describe("guessTimeScale", () => {
  it("a BJD_TDB column near 60000 is BMJD and one near 2460000 is BJD", () => {
    expect(JD_MJD_BOUNDARY).toBe(2400000.5);
    expect(guessTimeScale("BJD_TDB_NRS1", [60061.44, 60061.45])).toEqual({
      column: "BJD_TDB_NRS1",
      scale: "bmjd_tdb",
      reason: BMJD_REASON,
    });
    expect(guessTimeScale("bjd_tdb", [2460310.5, null])).toEqual({ column: "bjd_tdb", scale: "bjd_tdb", reason: null });
    expect(guessTimeScale("BJD", [2460310.5, 60061.44])).toEqual({ column: "BJD", scale: "bmjd_tdb", reason: BMJD_REASON });
    expect(guessTimeScale("bjd_tdb", [2400000.5])).toEqual({ column: "bjd_tdb", scale: "bjd_tdb", reason: null });
    expect(guessTimeScale("MJD", [60126.8])).toEqual({ column: "MJD", scale: "mjd_utc", reason: null });
    expect(guessTimeScale("MJD_UTC_NRS2", [60496.5])).toEqual({ column: "MJD_UTC_NRS2", scale: "mjd_utc", reason: null });
    expect(guessTimeScale("mjd_utc", [60496.5])?.scale).toBe("mjd_utc");
    expect(guessTimeScale("jd_utc", [2460310.5])).toEqual({ column: "jd_utc", scale: "jd_utc", reason: null });
    expect(guessTimeScale("hjd_utc", [2460310.5])).toEqual({ column: "hjd_utc", scale: "hjd_utc", reason: null });
    expect(guessTimeScale("HJD", [2460310.5])?.scale).toBe("hjd_utc");
    expect(guessTimeScale("jd_mid", [2460310.5])).toEqual({ column: "jd_mid", scale: "jd_header", reason: null });
    expect(guessTimeScale("jd", [2460310.5])?.scale).toBe("jd_header");
    expect(guessTimeScale("index", [0, 1])).toEqual({ column: "index", scale: "index", reason: null });
    expect(guessTimeScale("frame", [0, 1])?.scale).toBe("index");
    expect(guessTimeScale("bmjd_tdb", [60061.4])).toEqual({ column: "bmjd_tdb", scale: "bmjd_tdb", reason: null });
    expect(guessTimeScale("bjd_tdb", [null, null])).toBeNull();
    expect(guessTimeScale("bjd_tdb", [Number.NaN, Number.POSITIVE_INFINITY])).toBeNull();
    expect(guessTimeScale("bjd_tdb", [])).toBeNull();
    expect(guessTimeScale("time", [1, 2])).toBeNull();
    expect(guessTimeScale("BTJD", [1500.5])).toBeNull();
    expect(guessTimeScale("TIME", [1500.5])).toBeNull();
    expect(guessTimeScale("mjd_tdb", [60061.4])).toBeNull();
    expect(guessTimeScale("jd_utc_NRS1", [2460310.5])).toBeNull();
  });
});

describe("planLightCurveTable", () => {
  it("the app's own light-curve CSV opens on BJD_TDB with the differential magnitude and its error", () => {
    const plan = planLightCurveTable(appCsv());
    const time = { column: "bjd_tdb", scale: "bjd_tdb", reason: null };
    expect(plan.time).toEqual(time);
    expect(plan.times.map((t) => t.column)).toEqual(["index", "jd_mid", "bjd_tdb", "hjd_utc"]);
    expect(plan.values).toEqual([
      { column: "diff_mag", quantity: "diff_mag", errColumn: "diff_err", time },
      { column: "rel_flux", quantity: "rel_flux", errColumn: "rel_flux_err", time },
    ]);
    expect(plan.notes).toEqual([]);
    expect(plan.numericColumns).toEqual([
      "index",
      "jd_mid",
      "bjd_tdb",
      "hjd_utc",
      "diff_mag",
      "diff_err",
      "target_flux",
      "target_err",
      "rel_flux",
      "rel_flux_err",
    ]);
  });

  it("without a barycentric column the app's CSV falls back to the header JD", () => {
    const withoutBarycentre = planLightCurveTable(appCsv({ bjd_tdb: [null, null], hjd_utc: [null, null] }));
    expect(withoutBarycentre.time).toEqual({ column: "jd_mid", scale: "jd_header", reason: null });
    expect(withoutBarycentre.times.map((t) => t.column)).toEqual(["index", "jd_mid"]);
    expect(withoutBarycentre.numericColumns).toContain("bjd_tdb");
    expect(withoutBarycentre.values.map((v) => v.time.column)).toEqual(["jd_mid", "jd_mid"]);
    const withoutAnyTime = planLightCurveTable(appCsv({ bjd_tdb: [null, null], hjd_utc: [null, null], jd_mid: [null, null] }));
    expect(withoutAnyTime.time).toEqual({ column: "index", scale: "index", reason: null });
    const heliocentric = planLightCurveTable(appCsv({ bjd_tdb: [null, null] }));
    expect(heliocentric.time?.column).toBe("hjd_utc");
  });

  it("a white-light table of one detector reads its BJD_TDB column as BMJD", () => {
    const plan = planLightCurveTable(oneDetector());
    const time = { column: "BJD_TDB_NRS1", scale: "bmjd_tdb", reason: BMJD_REASON };
    expect(plan.time).toEqual(time);
    expect(plan.values).toEqual([{ column: "whitelight_flux_NRS1", quantity: "flux", errColumn: null, time }]);
    expect(plan.notes).toEqual([]);
  });

  it("each detector's white light is paired with its own time column", () => {
    const twoDetectors = table(
      col("MJD_UTC_NRS1", [60496.5212, 60496.5264]),
      col("BJD_TDB_NRS1", [60496.5225, 60496.5277]),
      col("whitelight_flux_NRS1", [-5953.6, -6100.9], "Jy"),
      col("MJD_UTC_NRS2", [60496.5211, 60496.5263]),
      col("BJD_TDB_NRS2", [60496.5224, 60496.5276]),
      col("whitelight_flux_NRS2", [3.71, 3.72], "Jy"),
    );
    const plan = planLightCurveTable(twoDetectors);
    expect(plan.time?.column).toBe("BJD_TDB_NRS1");
    expect(plan.values[0].time.column).toBe("BJD_TDB_NRS1");
    expect(plan.values[1].time.column).toBe("BJD_TDB_NRS2");
    const docs = lightCurveDocsFromTable(twoDetectors, "C:/d/jw03375_whtlt.ecsv");
    expect(docs.map((d) => d.id)).toEqual(["whitelight_flux_NRS1", "whitelight_flux_NRS2"]);
    expect(docs[0].time).toEqual([60496.5225, 60496.5277]);
    expect(docs[1].time).toEqual([60496.5224, 60496.5276]);
    expect(docs[0].value).toEqual([-5953.6, -6100.9]);
    expect(docs.map((d) => d.timeScale)).toEqual(["bmjd_tdb", "bmjd_tdb"]);
    expect(docs.map((d) => d.quantity)).toEqual(["flux", "flux"]);
    expect(docs[1].source).toEqual({
      kind: "table",
      path: "C:/d/jw03375_whtlt.ecsv",
      time_column: "BJD_TDB_NRS2",
      value_column: "whitelight_flux_NRS2",
    });

    const withoutBarycentreOnNrs2 = table(
      col("BJD_TDB_NRS1", [60496.5225, 60496.5277]),
      col("whitelight_flux_NRS1", [1, 2]),
      col("MJD_UTC_NRS2", [60496.5211, 60496.5263]),
      col("BJD_TDB_NRS2", [null, null]),
      col("whitelight_flux_NRS2", [3.71, 3.72]),
      col("whitelight_flux_NRS3", [5, 6]),
    );
    const partial = planLightCurveTable(withoutBarycentreOnNrs2);
    expect(partial.values.map((v) => v.time)).toEqual([
      { column: "BJD_TDB_NRS1", scale: "bmjd_tdb", reason: BMJD_REASON },
      { column: "MJD_UTC_NRS2", scale: "mjd_utc", reason: null },
      { column: "BJD_TDB_NRS1", scale: "bmjd_tdb", reason: BMJD_REASON },
    ]);
  });

  it("the orders of one detector share one time column", () => {
    const soss = table(
      col("MJD_UTC", [59786.8655, 59786.8656]),
      col("BJD_TDB", [59786.8663, 59786.8664]),
      col("whitelight_flux_order_1", [124.4, 124.5], "MJy"),
      col("whitelight_flux_order_2", [211.5, 211.6], "MJy"),
      col("whitelight_flux_order_3", [4.21, 4.22], "MJy"),
    );
    const plan = planLightCurveTable(soss);
    expect(plan.values.map((v) => v.column)).toEqual([
      "whitelight_flux_order_1",
      "whitelight_flux_order_2",
      "whitelight_flux_order_3",
    ]);
    expect(plan.values.map((v) => v.time.column)).toEqual(["BJD_TDB", "BJD_TDB", "BJD_TDB"]);
    const docs = lightCurveDocsFromTable(soss, "soss_whtlt.ecsv");
    expect(docs).toHaveLength(3);
    expect(docs.map((d) => d.unit)).toEqual(["MJy", "MJy", "MJy"]);

    const orderOfOneDetector = table(
      col("BJD_TDB", [59786.8663]),
      col("BJD_TDB_NRS2", [59786.8669]),
      col("whitelight_flux_order_2_NRS2", [1]),
    );
    expect(planLightCurveTable(orderOfOneDetector).values[0].time.column).toBe("BJD_TDB_NRS2");
  });

  it("a legacy white-light table has only MJD", () => {
    const legacy = table(col("MJD", [60126.8006, 60126.8007]), col("whitelight_flux", [2.16, 2.39]));
    const plan = planLightCurveTable(legacy);
    expect(plan.time).toEqual({ column: "MJD", scale: "mjd_utc", reason: null });
    const docs = lightCurveDocsFromTable(legacy, "legacy_whtlt.ecsv");
    expect(docs).toHaveLength(1);
    expect(docs[0].timeScale).toBe("mjd_utc");
    expect(docs[0].unit).toBeNull();
    expect(docs[0].notes).toEqual([]);
  });

  it("the order of preference is flux families, not table order", () => {
    const jd = col("jd_utc", [2460310.5, 2460310.6]);
    const time = { column: "jd_utc", scale: "jd_utc", reason: null };
    const fluxAndMag = table(
      col("mag", [12.1, 12.2]),
      col("mag_err", [0.01, 0.01]),
      col("flux", [100, 99]),
      col("flux_err", [1, 1]),
      jd,
    );
    expect(planLightCurveTable(fluxAndMag).values).toEqual([{ column: "flux", quantity: "flux", errColumn: "flux_err", time }]);
    expect(planLightCurveTable(table(col("mag", [12.1, 12.2]), col("err", [0.01, 0.01]), jd)).values).toEqual([
      { column: "mag", quantity: "diff_mag", errColumn: "err", time },
    ]);
    expect(planLightCurveTable(table(col("target_flux", [500, 480]), col("target_err", [5, 5]), jd)).values).toEqual([
      { column: "target_flux", quantity: "flux", errColumn: "target_err", time },
    ]);
    expect(planLightCurveTable(table(col("Flux", [100, 99]), col("ERR", [1, 1]), jd)).values).toEqual([
      { column: "Flux", quantity: "flux", errColumn: "ERR", time },
    ]);
    expect(planLightCurveTable(table(col("flux", [100, 99]), text("flux_err"), jd)).values).toEqual([
      { column: "flux", quantity: "flux", errColumn: null, time },
    ]);
    expect(planLightCurveTable(table(col("flux", [100, 99]), text("flux_err"), col("err", [1, 1]), jd)).values).toEqual([
      { column: "flux", quantity: "flux", errColumn: "err", time },
    ]);
    const whiteLightBeforeFlux = table(col("flux", [100, 99]), col("whitelight_flux", [1, 2]), col("rel_flux", [0.5, 0.6]), jd);
    expect(planLightCurveTable(whiteLightBeforeFlux).values.map((v) => v.column)).toEqual(["rel_flux"]);
    expect(planLightCurveTable(table(col("flux", [100, 99]), col("whitelight_flux", [1, 2]), jd)).values.map((v) => v.column)).toEqual([
      "whitelight_flux",
    ]);
    const relFluxFirst = table(col("rel_flux", [0.5, 0.6]), col("diff_mag", [1.5, 1.6]), jd);
    expect(planLightCurveTable(relFluxFirst).values).toEqual([
      { column: "rel_flux", quantity: "rel_flux", errColumn: null, time },
      { column: "diff_mag", quantity: "diff_mag", errColumn: null, time },
    ]);
    expect(planLightCurveTable(table(col("SAP_FLUX", [1, 2]), col("PDCSAP_FLUX", [1, 2]), jd)).values).toEqual([]);
  });

  it("a table without a time column, or without a value column, says what was looked for", () => {
    const catalogue = table(col("ra", [10.1, 10.2]), col("dec", [-5.1, -5.2]), col("flux_r", [100, 99]));
    const withoutTime = planLightCurveTable(catalogue);
    expect(withoutTime.time).toBeNull();
    expect(withoutTime.times).toEqual([]);
    expect(withoutTime.values).toEqual([]);
    expect(withoutTime.notes).toEqual([NO_TIME_COLUMN_NOTE]);
    expect(lightCurveDocsFromTable(catalogue, "catalogue.csv")).toEqual([]);

    const fluxWithoutTime = planLightCurveTable(table(col("flux", [100, 99]), col("flux_err", [1, 1])));
    expect(fluxWithoutTime.values).toEqual([]);
    expect(fluxWithoutTime.notes).toEqual([NO_TIME_COLUMN_NOTE]);

    const withoutValue = planLightCurveTable(table(col("jd_utc", [2460310.5, 2460310.6]), col("airmass", [1.1, 1.2])));
    expect(withoutValue.time?.column).toBe("jd_utc");
    expect(withoutValue.values).toEqual([]);
    expect(withoutValue.notes).toEqual([NO_VALUE_COLUMN_NOTE]);

    const onlyText = planLightCurveTable(table(text("bjd_tdb"), text("flux")));
    expect(onlyText.numericColumns).toEqual([]);
    expect(onlyText.times).toEqual([]);
    expect(onlyText.values).toEqual([]);
    expect(onlyText.notes).toEqual([NO_TIME_COLUMN_NOTE]);

    const empty = planLightCurveTable({ columns: [] });
    expect(empty.numericColumns).toEqual([]);
    expect(empty.time).toBeNull();
    expect(empty.notes).toEqual([NO_TIME_COLUMN_NOTE]);

    expect(NO_TIME_COLUMN_NOTE).toBe(
      "no time column found (looked for BJD_TDB, BMJD_TDB, MJD_UTC, MJD, jd_utc, hjd_utc, jd_mid, index): choose the time column and its scale",
    );
    expect(NO_VALUE_COLUMN_NOTE).toBe(
      "no flux or magnitude column found (looked for diff_mag, rel_flux, whitelight_flux, flux, mag, target_flux): choose the value column",
    );
  });
});

describe("lightCurveDocsFromTable", () => {
  it("the documents carry the reason of a BMJD reading and leave the table untouched", () => {
    const input = oneDetector();
    input.columns.forEach((column) => Object.freeze(column.values));
    Object.freeze(input.columns);
    const before = JSON.parse(JSON.stringify(input));
    const path = "C:/d/jw02358_whtlt.ecsv";
    const docs = lightCurveDocsFromTable(input, path);
    expect(docs).toHaveLength(1);
    const doc = docs[0];
    expect(doc.id).toBe("whitelight_flux_NRS1");
    expect(doc.label).toBe("whitelight_flux_NRS1 (jw02358_whtlt.ecsv)");
    expect(doc.unit).toBe("Jy");
    expect(doc.quantity).toBe("flux");
    expect(doc.timeScale).toBe("bmjd_tdb");
    expect(doc.time).toEqual([60061.4422, 60061.4425]);
    expect(doc.value).toEqual([0.1637, 0.164]);
    expect(doc.err).toEqual([null, null]);
    expect(doc.pointRef).toEqual([0, 1]);
    expect(doc.source).toEqual({ kind: "table", path, time_column: "BJD_TDB_NRS1", value_column: "whitelight_flux_NRS1" });
    expect(doc.notes).toEqual(["time column BJD_TDB_NRS1: values below 2400000.5: read as BMJD (TDB) = BJD_TDB - 2400000.5"]);
    expect(doc.overlay).toBeUndefined();
    expect("overlay" in doc).toBe(false);
    expect(doc.magErrFactor).toBeUndefined();
    expect(input).toEqual(before);

    const appDocs = lightCurveDocsFromTable(appCsv(), "C:/d/curve.csv");
    expect(appDocs.map((d) => d.id)).toEqual(["diff_mag", "rel_flux"]);
    expect(appDocs[0].err).toEqual([0.01, 0.02]);
    expect(appDocs[0].notes).toEqual(["errors from column diff_err"]);
    expect(appDocs[0].timeScale).toBe("bjd_tdb");
    expect(appDocs[1].quantity).toBe("rel_flux");

    const chosen = planLightCurveTable(appCsv());
    const onHeliocentric = lightCurveDocsFromTable(appCsv(), "C:/d/curve.csv", {
      ...chosen,
      values: [{ column: "target_flux", quantity: "flux", errColumn: "target_err", time: chosen.times[3] }],
    });
    expect(onHeliocentric).toHaveLength(1);
    expect(onHeliocentric[0].id).toBe("target_flux");
    expect(onHeliocentric[0].timeScale).toBe("hjd_utc");
    expect(onHeliocentric[0].time).toEqual([2460310.5033, 2460310.6033]);
    expect(onHeliocentric[0].err).toEqual([5, 5]);
  });

  it("a value column whose rows are all missing gives no document", () => {
    const orders = table(
      col("BJD_TDB", [60061.44, 60061.45]),
      col("whitelight_flux_order_1", [null, null]),
      col("whitelight_flux_order_2", [1.5, 1.6]),
    );
    expect(planLightCurveTable(orders).values.map((v) => v.column)).toEqual(["whitelight_flux_order_2"]);
    const docs = lightCurveDocsFromTable(orders, "orders_whtlt.ecsv");
    expect(docs.map((d) => d.id)).toEqual(["whitelight_flux_order_2"]);

    const halfMissing = table(col("BJD_TDB", [60061.44, 60061.45]), col("whitelight_flux", [null, 1.6]));
    const half = lightCurveDocsFromTable(halfMissing, "half_whtlt.ecsv");
    expect(half).toHaveLength(1);
    expect(half[0].time).toEqual([60061.45]);
    expect(half[0].value).toEqual([1.6]);
    expect(half[0].pointRef).toEqual([1]);

    const neverTogether = table(col("BJD_TDB", [60061.44, null]), col("whitelight_flux", [null, 1.6]));
    expect(planLightCurveTable(neverTogether).values).toHaveLength(1);
    expect(lightCurveDocsFromTable(neverTogether, "never_whtlt.ecsv")).toEqual([]);
  });
});

const DATA_DIR = process.env.ASTROBURST_SCIENCE_DATA_DIR;

function readEcsv(path: string): LightCurveTableInput {
  const lines = readFileSync(path, "utf8")
    .split(/\r?\n/)
    .filter((line) => line.length > 0);
  const units = new Map<string, string>();
  lines.forEach((line) => {
    const declared = /^# - \{name: ([^,}]+), unit: ([^,}]+), datatype: [^,}]+\}$/.exec(line);
    if (declared) units.set(declared[1], declared[2]);
  });
  const body = lines.filter((line) => !line.startsWith("#"));
  const rows = body.slice(1).map((line) => line.split(" ").map(Number));
  return {
    columns: body[0].split(" ").map((name, index) => ({
      name,
      unit: units.get(name) ?? null,
      values: rows.map((row) => row[index]),
    })),
  };
}

function whiteLightDocs(file: string) {
  const path = join(DATA_DIR as string, "tso", "whtlt", file);
  return lightCurveDocsFromTable(readEcsv(path), path);
}

describe.skipIf(!DATA_DIR)("archived JWST white-light tables (ASTROBURST_SCIENCE_DATA_DIR)", () => {
  it("real_e1_whtlt_becomes_one_bmjd_curve", () => {
    const docs = whiteLightDocs("jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv");
    expect(docs).toHaveLength(1);
    expect(docs[0].timeScale).toBe("bmjd_tdb");
    expect(docs[0].unit).toBe("Jy");
    expect(docs[0].time).toHaveLength(240);
    expect(docs[0].time[0]).toBe(60061.4421733457);
    expect(docs[0].time[239]).toBe(60061.52706665472);
    expect(docs[0].value[0]).toBe(0.16369951048163586);
    expect(docs[0].value[239]).toBe(0.1640142699204498);
  });

  it("real_two_detector_whtlt_pairs_each_flux_with_its_own_time", () => {
    const docs = whiteLightDocs("jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_whtlt.ecsv");
    expect(docs.map((d) => d.id)).toEqual(["whitelight_flux_NRS1", "whitelight_flux_NRS2"]);
    expect(docs.map((d) => d.time.length)).toEqual([8, 8]);
    expect(docs.map((d) => d.timeScale)).toEqual(["bmjd_tdb", "bmjd_tdb"]);
    expect(docs[0].time[0]).toBe(60496.522504382556);
    expect(docs[0].value[0]).toBe(-5953.629162915719);
    expect(docs[1].time[0]).toBe(60496.52240268599);
    expect(docs[1].value[0]).toBe(3.7127694054398646);
  });

  it("real_soss_whtlt_gives_one_curve_per_order", () => {
    const docs = whiteLightDocs("jw01366-o001_t001_niriss_clear-gr700xd-substrip256_whtlt.ecsv");
    expect(docs.map((d) => d.time.length)).toEqual([537, 537, 537]);
    expect(docs.map((d) => d.unit)).toEqual(["MJy", "MJy", "MJy"]);
    expect(docs.map((d) => d.time[0])).toEqual([59786.866358341525, 59786.866358341525, 59786.866358341525]);
    expect(docs[0].value[0]).toBe(124.48996614090491);
    expect(docs[1].value[0]).toBe(211.59072077224457);
    expect(docs[2].value[0]).toBe(4.210519392499863);
  });

  it("real_legacy_whtlt_is_read_on_mjd_utc", () => {
    const docs = whiteLightDocs("jw01274-o013_t008_miri_p750l-slitlessprism_whtlt.ecsv");
    expect(docs).toHaveLength(1);
    expect(docs[0].timeScale).toBe("mjd_utc");
    expect(docs[0].unit).toBeNull();
    expect(docs[0].time).toHaveLength(10);
    expect(docs[0].time[0]).toBe(60126.800615279506);
    expect(docs[0].value[0]).toBe(2.163616608616668);
  });

  it("real_one_row_whtlt_is_a_curve_of_one_point", () => {
    const docs = whiteLightDocs("jw06828-o003_t001_nirspec_clear-prism-s1600a1-sub512_whtlt.ecsv");
    expect(docs).toHaveLength(1);
    expect(docs[0].time).toHaveLength(1);
    expect(docs[0].time[0]).toBe(60599.14203399092);
    expect(docs[0].value[0]).toBe(2.988304267333806);
    expect(docs[0].unit).toBeNull();
  });
});
