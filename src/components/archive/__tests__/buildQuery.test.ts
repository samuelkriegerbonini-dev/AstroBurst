import { describe, expect, it, vi } from "vitest";
import {
  DEFAULT_FORM,
  EMPTY_QUERY_HINT,
  LEVEL_STORAGE_KEY,
  applicableFilters,
  loadLevel,
  parseRadius,
  planSearch,
  saveLevel,
  switchMission,
  type ArchiveForm,
} from "../logic/buildQuery";
import type { MastResolved } from "../../../shared/types/mast";

const JWST_FILTERS = ["level", "instrument", "filter", "date", "public"] as const;
const HST_FILTERS = ["instrument", "filter", "date"] as const;

function form(over: Partial<ArchiveForm> = {}): ArchiveForm {
  return { ...DEFAULT_FORM, ...over, filters: { ...DEFAULT_FORM.filters, ...(over.filters ?? {}) } };
}

const M16: MastResolved = {
  query: "M16",
  canonicalName: "M 16",
  resolver: "SIMBAD",
  raDeg: 274.688,
  decDeg: -13.792,
  objectType: "open cluster",
  elapsedMs: 870,
  fromCache: false,
};

describe("planSearch", () => {
  it("starts on JWST, Combined L3, radius 3", () => {
    expect(DEFAULT_FORM.mission).toBe("jwst");
    expect(DEFAULT_FORM.filters.level).toBe("combined");
    expect(DEFAULT_FORM.radiusText).toBe("3");
  });

  it("blocks an empty query with the exact hint", () => {
    expect(planSearch(form({ text: "  " }), [...JWST_FILTERS])).toEqual({ kind: "blocked", field: "query", message: EMPTY_QUERY_HINT });
    expect(EMPTY_QUERY_HINT).toBe("Add a target, coordinates, programme or dataset id");
  });

  it("blocks a query with only filters set", () => {
    const plan = planSearch(form({ text: "", filters: { instrument: "MIRI", dateFrom: "2023-01-01" } }), [...JWST_FILTERS]);
    expect(plan.kind).toBe("blocked");
  });

  it("asks for a name lookup before searching a name", () => {
    expect(planSearch(form({ text: "M16" }), [...JWST_FILTERS])).toEqual({ kind: "resolve", name: "M16" });
  });

  it("searches a resolved name by position with the canonical label and the radius kept", () => {
    const plan = planSearch(form({ text: "M16", radiusText: "4.5" }), [...JWST_FILTERS], { resolved: M16 });
    expect(plan).toEqual({
      kind: "search",
      query: {
        mission: "jwst",
        where: { kind: "position", raDeg: 274.688, decDeg: -13.792, radiusArcmin: 4.5, label: "M 16" },
        program: null,
        datasetId: null,
        filters: { level: "combined" },
        sort: [],
        offset: 0,
        limit: 200,
      },
    });
  });

  it("sends the name to MAST when the lookup service failed", () => {
    const plan = planSearch(form({ text: "M16" }), [...JWST_FILTERS], { serverResolve: true });
    expect(plan.kind === "search" && plan.query.where).toEqual({ kind: "name", name: "M16", radiusArcmin: 3 });
  });

  it("builds a position query from coordinates without a lookup", () => {
    const plan = planSearch(form({ text: "10.0 -80.0", radiusText: "1" }), [...JWST_FILTERS]);
    expect(plan.kind === "search" && plan.query.where).toEqual({ kind: "position", raDeg: 10, decDeg: -80, radiusArcmin: 1, label: null });
  });

  it("builds a programme query with the level filter and no position", () => {
    const plan = planSearch(form({ text: "2739" }), [...JWST_FILTERS]);
    expect(plan.kind === "search" && plan.query).toMatchObject({ where: { kind: "none" }, program: 2739, datasetId: null, filters: { level: "combined" } });
  });

  it("builds a dataset query without filters, since one id names one dataset", () => {
    const plan = planSearch(form({ mission: "hst", text: "ICK909030", filters: { instrument: "WFC3" } }), [...HST_FILTERS]);
    expect(plan.kind === "search" && plan.query).toMatchObject({ mission: "hst", where: { kind: "none" }, program: null, datasetId: "ICK909030", filters: {} });
  });

  it("maps only the filters the mission offers and drops empty values", () => {
    const plan = planSearch(
      form({ mission: "hst", text: "M16", filters: { level: "combined", instrument: "WFC3", filter: " ", dateFrom: "2014-01-01", dateTo: "", publicOnly: true } }),
      [...HST_FILTERS],
      { resolved: M16 },
    );
    expect(plan.kind === "search" && plan.query.filters).toEqual({ instrument: "WFC3", dateFrom: "2014-01-01" });
  });

  it("keeps every JWST filter and trims the free-text filter", () => {
    const plan = planSearch(
      form({ text: "M16", filters: { level: "all", instrument: "NIRCAM", filter: " f444w ", dateFrom: "2022-01-01", dateTo: "2022-12-31", publicOnly: true } }),
      [...JWST_FILTERS],
      { resolved: M16 },
    );
    expect(plan.kind === "search" && plan.query.filters).toEqual({
      level: "all",
      instrument: "NIRCAM",
      filter: "F444W",
      dateFrom: "2022-01-01",
      dateTo: "2022-12-31",
      publicOnly: true,
    });
  });

  it("passes the page offset and a server sort", () => {
    const plan = planSearch(form({ text: "2739" }), [...JWST_FILTERS], { offset: 400, sort: [{ column: "date_obs", desc: true }] });
    expect(plan.kind === "search" && plan.query).toMatchObject({ offset: 400, limit: 200, sort: [{ column: "date_obs", desc: true }] });
  });

  it("blocks interpretation errors with the field message", () => {
    expect(planSearch(form({ text: "400 10" }), [...JWST_FILTERS])).toEqual({ kind: "blocked", field: "query", message: "RA must be at least 0 and below 360 degrees" });
  });

  it("blocks an unreadable radius for position searches only", () => {
    expect(planSearch(form({ text: "M16", radiusText: "abc" }), [...JWST_FILTERS])).toEqual({
      kind: "blocked",
      field: "radius",
      message: "Radius: type a number of arcminutes, for example 3",
    });
    expect(planSearch(form({ text: "2739", radiusText: "abc" }), [...JWST_FILTERS]).kind).toBe("search");
  });

  it("honours the manual kind override", () => {
    expect(planSearch(form({ text: "2739", kindOverride: "name" }), [...JWST_FILTERS])).toEqual({ kind: "resolve", name: "2739" });
  });
});

describe("parseRadius", () => {
  it.each([["3", 3], ["3'", 3], [" 1.5 ", 1.5], ["60", 60]])("reads %s", (text, value) => {
    expect(parseRadius(text)).toBe(value);
  });

  it.each(["", "0", "-1", "abc", "3''"])("refuses %s", (text) => {
    expect(parseRadius(text)).toBeNull();
  });
});

describe("applicableFilters", () => {
  it("keeps the date range as two values and drops a false public flag", () => {
    expect(applicableFilters({ dateFrom: "2014-01-01", dateTo: "2014-12-31", publicOnly: false }, [...JWST_FILTERS])).toEqual({
      dateFrom: "2014-01-01",
      dateTo: "2014-12-31",
    });
  });
});

describe("switchMission", () => {
  it("keeps the query, the kind override, the radius and the date range, and resets instrument, filter and public", () => {
    const before = form({
      text: "M16",
      kindOverride: "name",
      radiusText: "5",
      filters: { level: "all", instrument: "NIRCAM", filter: "F444W", dateFrom: "2022-01-01", dateTo: "2023-01-01", publicOnly: true },
    });
    const after = switchMission(before, "hst", null);
    expect(after).toEqual({
      mission: "hst",
      text: "M16",
      kindOverride: "name",
      radiusText: "5",
      filters: { dateFrom: "2022-01-01", dateTo: "2023-01-01" },
    });
  });

  it("restores the remembered JWST level when switching back", () => {
    const storage = { getItem: vi.fn(() => "all"), setItem: vi.fn() };
    const after = switchMission(form({ mission: "hst", filters: {} }), "jwst", storage);
    expect(after.filters.level).toBe("all");
    expect(storage.getItem).toHaveBeenCalledWith(LEVEL_STORAGE_KEY);
  });
});

describe("level memory", () => {
  it("defaults to combined without storage, with an unknown value or when storage throws", () => {
    expect(loadLevel(null)).toBe("combined");
    expect(loadLevel({ getItem: () => "weird" })).toBe("combined");
    expect(loadLevel({ getItem: () => { throw new Error("blocked"); } })).toBe("combined");
  });

  it("reads and writes the level under its key, swallowing storage errors", () => {
    const store = new Map<string, string>();
    const storage = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) };
    saveLevel(storage, "all");
    expect(store.get(LEVEL_STORAGE_KEY)).toBe("all");
    expect(loadLevel(storage)).toBe("all");
    expect(() => saveLevel({ setItem: () => { throw new Error("quota"); } }, "combined")).not.toThrow();
    expect(() => saveLevel(null, "combined")).not.toThrow();
  });
});
