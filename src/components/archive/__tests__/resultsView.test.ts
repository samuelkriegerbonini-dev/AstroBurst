import { describe, expect, it } from "vitest";
import {
  appliedText,
  cellText,
  columnWidth,
  displayRows,
  elapsedText,
  emptyView,
  filterSummary,
  fitColumns,
  footerMastText,
  notResolvedView,
  queuedThumbs,
  rowsText,
  searchProgress,
  sortText,
  summaryView,
  tableColumns,
  type RunLike,
} from "../logic/resultsView";
import { mergePage } from "../logic/paging";
import { createThumbQueue, markReady, markStarted, setDemand } from "../logic/thumbQueue";
import { COLUMNS, MISSIONS, missionById, page, row } from "./fixtures";
import type { MastColumn, MastMission, MastQuery, MastResolved } from "../../../shared/types/mast";

const M16: MastResolved = { query: "M16", canonicalName: "M 16", resolver: "SIMBAD", raDeg: 274.688, decDeg: -13.792, objectType: "open cluster", elapsedMs: 870, fromCache: false };

function positionQuery(radiusArcmin = 3, over: Partial<MastQuery> = {}): MastQuery {
  return {
    mission: "jwst",
    where: { kind: "position", raDeg: 274.688, decDeg: -13.792, radiusArcmin, label: "M 16" },
    program: null,
    datasetId: null,
    filters: { level: "combined" },
    sort: [],
    offset: 0,
    limit: 200,
    ...over,
  };
}

const TWO = [row("jw02739-o001_t001_nircam"), row("jw02739-o002_t001_miri", { instrument: "MIRI" })];

describe("appliedText", () => {
  it("lists every condition MAST applied as column = value", () => {
    expect(appliedText({ conditions: [{ productLevel: "3" }, { instrume: "MIRI" }], sortBy: [], sortDesc: [] })).toBe("productLevel = 3, instrume = MIRI");
  });

  it("keeps the operator of merged same-column conditions instead of adding =", () => {
    expect(appliedText({ conditions: [{ sci_start_time: ">=2014-01-01,<=2014-12-31" }], sortBy: [], sortDesc: [] })).toBe(
      "sci_start_time >=2014-01-01,<=2014-12-31",
    );
  });

  it("says none when MAST applied nothing", () => {
    expect(appliedText({ conditions: [], sortBy: [], sortDesc: [] })).toBe("none");
  });
});

describe("summaryView", () => {
  it("shows the resolved name, position, radius, count, time and applied conditions", () => {
    const results = mergePage(null, page(TWO));
    const v = summaryView({ results, resolved: M16, query: positionQuery(), missionLabel: "JWST" });
    expect(v.text).toBe('M16 → SIMBAD "M 16" 274.68800 -13.79200 · r 3\' · 2 datasets · 1.7 s · MAST applied: productLevel = 3');
    expect(v.raDeg).toBeCloseTo(274.688, 6);
    expect(v.decDeg).toBeCloseTo(-13.792, 6);
    expect(v.total).toBe(2);
    expect(v.applied).toBe("productLevel = 3");
  });

  it("names the programme and leaves the radius out for programme queries", () => {
    const rows = Array.from({ length: 14 }, (_, i) => row(`jw02739-o0${String(i).padStart(2, "0")}_t001_nircam`));
    const results = mergePage(null, page(rows, { radiusArcmin: null, searchPosition: null, elapsedMs: 3300, applied: { conditions: [{ program: "2739" }, { productLevel: "3" }], sortBy: [], sortDesc: [] } }));
    const query = positionQuery(3, { where: { kind: "none" }, program: 2739 });
    const v = summaryView({ results, resolved: null, query, missionLabel: "JWST" });
    expect(v.text).toBe("Programme 2739 · 14 datasets · 3.3 s · MAST applied: program = 2739, productLevel = 3");
    expect(v.raDeg).toBeNull();
    expect(v.decDeg).toBeNull();
  });

  it("names the dataset id, flags a clamped radius and a cached answer", () => {
    const dataset = mergePage(null, page([row("ICK909030")], { mission: "hst", radiusArcmin: null, searchPosition: null, fromCache: true, elapsedMs: 12, applied: { conditions: [{ sci_data_set_name: "ICK909030" }], sortBy: [], sortDesc: [] } }));
    const byId = summaryView({ results: dataset, resolved: null, query: positionQuery(3, { mission: "hst", where: { kind: "none" }, datasetId: "ICK909030", filters: {} }), missionLabel: "HST" });
    expect(byId.text).toBe("Dataset ICK909030 · 1 dataset · 0.0 s (cached) · MAST applied: sci_data_set_name = ICK909030");

    const clamped = mergePage(null, page(TWO, { radiusArcmin: 30, radiusClamped: true }));
    const wide = summaryView({ results: clamped, resolved: null, query: positionQuery(60, { where: { kind: "position", raDeg: 274.688, decDeg: -13.792, radiusArcmin: 60, label: null } }), missionLabel: "JWST" });
    expect(wide.text).toContain("274.68800 -13.79200 · r 30' (clamped) · 2 datasets");
  });

  it("marks the total approximate when MAST could not count a wildcard, and counts the loaded rows", () => {
    const results = mergePage(null, page(TWO, { totalReported: 113, totalReliable: false, hasMore: true }));
    const v = summaryView({ results, resolved: M16, query: positionQuery(), missionLabel: "JWST" });
    expect(v.text).toContain("2+ datasets (total approximate)");
    expect(v.total).toBe(2);
  });

  it("uses MAST's reported total when it is reliable and larger than the loaded rows", () => {
    const results = mergePage(null, page(TWO, { totalReported: 265, hasMore: true }));
    expect(summaryView({ results, resolved: M16, query: positionQuery(), missionLabel: "HST" }).total).toBe(265);
  });

  it("says MAST resolved the name when the lookup service failed and the search did it", () => {
    const results = mergePage(null, page(TWO));
    const query = positionQuery(3, { where: { kind: "name", name: "M16", radiusArcmin: 3 } });
    expect(summaryView({ results, resolved: null, query, missionLabel: "JWST" }).text).toBe(
      '"M16" resolved by MAST 274.68800 -13.79200 · r 3\' · 2 datasets · 1.7 s · MAST applied: productLevel = 3',
    );
  });
});

describe("elapsedText and filterSummary", () => {
  it("shows tenths below 10 s and whole seconds above", () => {
    expect(elapsedText(870)).toBe("0.9 s");
    expect(elapsedText(9_949)).toBe("9.9 s");
    expect(elapsedText(14_200)).toBe("14 s");
  });

  it("describes the filters in the words of the form", () => {
    expect(filterSummary({ level: "combined", instrument: "NIRCAM", filter: "F444W", dateFrom: "2014-01-01", dateTo: "2014-12-31", publicOnly: true })).toBe(
      "Combined L3, NIRCAM, filter F444W, from 2014-01-01, to 2014-12-31, public only",
    );
    expect(filterSummary({ level: "all" })).toBe("");
  });
});

describe("searchProgress", () => {
  const base: RunLike = { phase: "resolving", kind: "name", text: "M16", name: "M16", resolved: null, startedAt: 0, searchStartedAt: 0 };
  const ctx = { missionLabel: "JWST", radiusText: "3", filters: { level: "combined" as const } };

  it("shows the lookup line and adds the elapsed time only after 1 s", () => {
    expect(searchProgress(base, { ...ctx, now: 500 }).lines).toEqual([{ key: "resolve", text: 'Resolving "M16" with MAST name lookup', elapsed: null }]);
    expect(searchProgress(base, { ...ctx, now: 1200 }).lines[0].elapsed).toBe("1.2 s");
  });

  it("shows the resolved object and the search line once the name is known", () => {
    const run: RunLike = { ...base, phase: "searching", resolved: M16, searchStartedAt: 900 };
    const view = searchProgress(run, { ...ctx, now: 14_900 });
    expect(view.lines).toEqual([
      { key: "resolve", text: 'Resolving "M16" with MAST name lookup', elapsed: "0.9 s" },
      { key: "resolved", text: '= SIMBAD "M 16" 274.68800 -13.79200 open cluster', elapsed: null },
      { key: "search", text: "Searching JWST r 3' Combined L3", elapsed: "14 s" },
    ]);
    expect(view.slowNote).toBe("MAST answers queries it has not seen recently slowly (25 to 80 s measured); repeats take 1 to 3 s.");
  });

  it("skips the lookup for coordinates and programmes and adds the slow note only after 8 s", () => {
    const coords: RunLike = { ...base, phase: "searching", kind: "coordinates", text: "274.688 -13.792", name: null };
    expect(searchProgress(coords, { ...ctx, now: 7_999 })).toEqual({ lines: [{ key: "search", text: "Searching JWST r 3' Combined L3", elapsed: "8.0 s" }], slowNote: null });
    const programme: RunLike = { ...coords, kind: "programme", text: "2739" };
    expect(searchProgress(programme, { ...ctx, now: 8_000 }).lines[0].text).toBe("Searching JWST programme 2739 Combined L3");
    expect(searchProgress(programme, { ...ctx, now: 8_000 }).slowNote).not.toBeNull();
    const dataset: RunLike = { ...coords, kind: "dataset", text: "ICK909030" };
    expect(searchProgress(dataset, { ...ctx, missionLabel: "HST", filters: {}, now: 10 }).lines[0].text).toBe("Searching HST dataset ICK909030");
  });
});

describe("emptyView", () => {
  const jwst = missionById("jwst");

  it("offers a wider radius, All levels and Try on HST for an empty JWST cone with Combined L3", () => {
    const results = mergePage(null, page([], { radiusArcmin: 1, searchPosition: { raDeg: 10, decDeg: -80 } }));
    const query = positionQuery(1, { where: { kind: "position", raDeg: 10, decDeg: -80, radiusArcmin: 1, label: null } });
    const v = emptyView({ query, results, missions: MISSIONS, mission: jwst });
    expect(v.title).toBe("No JWST datasets within 1' of 10.00000 -80.00000");
    expect(v.chips).toEqual([{ id: "level", label: "Combined L3" }]);
    expect(v.widenTo).toBe(3);
    expect(v.allLevels).toBe(true);
    expect(v.clearFilters).toBe(false);
    expect(v.tryOn).toEqual({ mission: "hst", label: "Try on HST" });
  });

  it("widens through 3', 10' and 30' and stops at 30'", () => {
    const at = (r: number) => {
      const results = mergePage(null, page([], { radiusArcmin: r }));
      return emptyView({ query: positionQuery(r), results, missions: MISSIONS, mission: jwst }).widenTo;
    };
    expect(at(0.5)).toBe(3);
    expect(at(3)).toBe(10);
    expect(at(10)).toBe(30);
    expect(at(30)).toBeNull();
  });

  it("hides Try on HST when a filter has no HST column, and offers Clear filters", () => {
    const results = mergePage(null, page([], { radiusArcmin: 3 }));
    const query = positionQuery(3, { filters: { level: "combined", instrument: "MIRI", publicOnly: true } });
    const v = emptyView({ query, results, missions: MISSIONS, mission: jwst });
    expect(v.chips.map((c) => c.label)).toEqual(["Combined L3", "MIRI", "Public only"]);
    expect(v.clearFilters).toBe(true);
    expect(v.tryOn).toBeNull();
  });

  it("offers Try on JWST from HST and no level action there", () => {
    const results = mergePage(null, page([], { mission: "hst", radiusArcmin: 3 }));
    const query = positionQuery(3, { mission: "hst", filters: { instrument: "WFC3", dateFrom: "2014-01-01" } });
    const v = emptyView({ query, results, missions: MISSIONS, mission: missionById("hst") });
    expect(v.title).toBe("No HST datasets within 3' of 274.68800 -13.79200");
    expect(v.chips).toEqual([{ id: "instrument", label: "WFC3" }, { id: "dateFrom", label: "From 2014-01-01" }]);
    expect(v.allLevels).toBe(false);
    expect(v.tryOn).toEqual({ mission: "jwst", label: "Try on JWST" });
  });

  it("titles programme and dataset queries without a radius", () => {
    const results = mergePage(null, page([], { radiusArcmin: null, searchPosition: null }));
    expect(emptyView({ query: positionQuery(3, { where: { kind: "none" }, program: 9999 }), results, missions: MISSIONS, mission: jwst })).toMatchObject({
      title: "No JWST datasets in programme 9999",
      widenTo: null,
    });
    expect(emptyView({ query: positionQuery(3, { where: { kind: "none" }, datasetId: "jw09999-o001_t001_nircam" }), results, missions: MISSIONS, mission: jwst }).title).toBe(
      "No JWST dataset called jw09999-o001_t001_nircam",
    );
  });
});

describe("notResolvedView", () => {
  it("names the object and the lookup time and offers a programme search only for a number", () => {
    expect(notResolvedView("Pilars of Creation", 22_500)).toEqual({
      title: 'MAST name lookup found no object called "Pilars of Creation" (23 s)',
      programmeOffer: false,
    });
    expect(notResolvedView("123456", 800).programmeOffer).toBe(true);
  });
});

describe("footer MAST text", () => {
  it("is idle with nothing in flight and counts thumbnails and the queue otherwise", () => {
    expect(footerMastText(0, 0, 0)).toBe("MAST idle");
    expect(footerMastText(2, 0, 0)).toBe("MAST 2 in flight");
    expect(footerMastText(6, 4, 9)).toBe("MAST 6 in flight (4 thumbnails, 9 queued)");
    expect(footerMastText(1, 1, 0)).toBe("MAST 1 in flight (1 thumbnail, 0 queued)");
  });

  it("counts queued thumbnails as wanted, not started and not finished", () => {
    let q = setDemand(createThumbQueue(), { selected: ["a"], visible: ["a", "b", "c", "d", "e", "f"], ahead: ["g"] });
    q = markStarted(q, ["a", "b"]);
    q = markReady(q, "a");
    expect(queuedThumbs(q)).toBe(5);
  });
});

describe("table helpers", () => {
  const jwstDefaults: MastColumn[] = [
    { name: "fileSetName", label: "Dataset", kind: "text", sortable: true, unit: null },
    { name: "productLevel", label: "Lvl", kind: "text", sortable: true, unit: null },
    { name: "missing_col", label: "Missing", kind: "text", sortable: true, unit: null },
    { name: "date_obs", label: "Date obs (UTC)", kind: "datetime", sortable: false, unit: null },
    { name: "_prog_obs", label: "Prog.Obs", kind: "text", sortable: false, unit: null },
  ];
  const mission: MastMission = { ...missionById("jwst"), defaultColumns: jwstDefaults };

  it("shows the mission's default columns that the answer has, sortable as MAST reported", () => {
    const results = mergePage(null, page(TWO));
    const cols = tableColumns(mission, results);
    expect(cols.map((c) => c.name)).toEqual(["fileSetName", "productLevel", "date_obs", "_prog_obs"]);
    expect(cols.find((c) => c.name === "date_obs")?.sortable).toBe(true);
    expect(cols.find((c) => c.name === "date_obs")?.label).toBe("Date obs (UTC)");
  });

  it("falls back to the answer's columns when the mission has no defaults", () => {
    const results = mergePage(null, page(TWO));
    expect(tableColumns(missionById("iue"), results).map((c) => c.name)).toEqual(COLUMNS.map((c) => c.name));
  });

  it("hides columns from the right to fit and always keeps the dataset column", () => {
    const cols = COLUMNS;
    const all = cols.reduce((sum, c, i) => sum + columnWidth(c, i === 0), 0);
    expect(fitColumns(cols, all).map((c) => c.name)).toEqual(cols.map((c) => c.name));
    expect(fitColumns(cols, all - 1).length).toBe(cols.length - 1);
    expect(fitColumns(cols, 10).map((c) => c.name)).toEqual(["fileSetName"]);
  });

  it("formats cells by kind", () => {
    const date = COLUMNS.find((c) => c.name === "date_obs") as MastColumn;
    const exp = COLUMNS.find((c) => c.name === "effexptm") as MastColumn;
    const prog = COLUMNS.find((c) => c.name === "program") as MastColumn;
    expect(cellText("2023-09-04T00:02:31.123", date)).toBe("2023-09-04 00:02");
    expect(cellText(1825.312, exp)).toBe("1825.3");
    expect(cellText(2739, prog)).toBe("2739");
    expect(cellText(null, prog)).toBe("");
    expect(cellText(true, prog)).toBe("yes");
  });

  it("keeps small and fractional floats readable instead of rounding them to one decimal", () => {
    const float: Pick<MastColumn, "kind"> = { kind: "float" };
    expect(cellText(0.0029, float)).toBe("0.0029");
    expect(cellText(0.04571, float)).toBe("0.04571");
    expect(cellText(8.43, float)).toBe("8.43");
    expect(cellText(-0.45, float)).toBe("-0.45");
    expect(cellText(708.6, float)).toBe("708.6");
    expect(cellText(96.6339, float)).toBe("96.63");
    expect(cellText(12345.678, float)).toBe("12345.7");
    expect(cellText(3, float)).toBe("3");
  });

  it("sorts the displayed rows locally and describes the order", () => {
    const rows = [
      row("a", { values: { date_obs: "2023-04-06T10:12:00", fileSetName: "a" } }),
      row("b", { values: { date_obs: "2023-09-04T00:02:00", fileSetName: "b" } }),
    ];
    const results = mergePage(null, page(rows));
    const sort = { keys: [{ column: "date_obs", desc: true }], by: "local" as const };
    expect(displayRows(results, sort).map((r) => r.datasetId)).toEqual(["b", "a"]);
    expect(displayRows(results, { keys: [], by: "local" }).map((r) => r.datasetId)).toEqual(["a", "b"]);
    expect(displayRows(null, sort)).toEqual([]);
    expect(sortText(sort, COLUMNS, results)).toBe("Sorted locally by DATE OBS (UTC) desc (all 2 rows loaded)");
    expect(sortText({ keys: [{ column: "date_obs", desc: false }], by: "server" }, COLUMNS, results)).toBe("Sorted by MAST: DATE OBS (UTC) asc");
    expect(sortText({ keys: [], by: "local" }, COLUMNS, results)).toBe("");
  });

  it("counts the visible rows", () => {
    const rows = Array.from({ length: 14 }, (_, i) => row(`r${i}`));
    expect(rowsText(1, 14, mergePage(null, page(rows)))).toBe("Rows 1-14 of 14");
    expect(rowsText(1, 14, mergePage(null, page(rows, { hasMore: true, totalReported: 450 })))).toBe("Rows 1-14 of 14+");
    expect(rowsText(0, 0, mergePage(null, page([])))).toBe("No rows");
  });
});
