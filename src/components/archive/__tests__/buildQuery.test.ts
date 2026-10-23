import { describe, expect, it, vi } from "vitest";
import {
  DEFAULT_FORM,
  EMPTY_QUERY_HINT,
  FILTERS_OPEN_STORAGE_KEY,
  LEVEL_STORAGE_KEY,
  applicableFilters,
  loadFiltersOpen,
  loadLevel,
  parseRadius,
  planSearch,
  saveFiltersOpen,
  saveLevel,
  switchMission,
  type ArchiveForm,
} from "../logic/buildQuery";
import type { MastResolved } from "../../../shared/types/mast";
import { HST_GROUPS, IUE_GROUPS, JWST_GROUPS } from "./fixtures";

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
        preset: null,
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

  it("keeps the category and object type ids the mission offers, in option order, once each, and drops empty lists", () => {
    expect(applicableFilters({ level: "combined", categories: ["stars", "galaxy_clusters", "galaxies", "stars"], objectTypes: ["exoplanets", "planetary_nebulae"] }, [...JWST_FILTERS], JWST_GROUPS)).toEqual({
      level: "combined",
      categories: ["galaxies", "galaxy_clusters", "stars"],
      objectTypes: ["planetary_nebulae", "exoplanets"],
    });
    expect(applicableFilters({ categories: ["galaxy_clusters", "stars"], objectTypes: ["exoplanets"] }, [], IUE_GROUPS)).toEqual({ categories: ["stars"] });
    expect(applicableFilters({ categories: ["galaxy_clusters"], objectTypes: [] }, [], IUE_GROUPS)).toEqual({});
    expect(applicableFilters({ categories: ["galaxies"], objectTypes: ["exoplanets"] }, [...JWST_FILTERS])).toEqual({});
  });
});

describe("object filters in a planned search", () => {
  it("sends the offered category and object type ids with a name, coordinates or programme query", () => {
    const f = form({ text: "M16", filters: { level: "combined", categories: ["ism", "galaxies"], objectTypes: ["planetary_nebulae"] } });
    const named = planSearch(f, [...JWST_FILTERS], { resolved: M16 }, JWST_GROUPS);
    expect(named.kind === "search" && named.query.filters).toEqual({ level: "combined", categories: ["galaxies", "ism"], objectTypes: ["planetary_nebulae"] });
    const programme = planSearch({ ...f, text: "2739" }, [...JWST_FILTERS], {}, JWST_GROUPS);
    expect(programme.kind === "search" && programme.query.filters).toEqual({ level: "combined", categories: ["galaxies", "ism"], objectTypes: ["planetary_nebulae"] });
  });

  it("drops object filters from a dataset query and when no group is passed", () => {
    const f = form({ text: "jw02739-o001_t001_nircam", filters: { level: "combined", categories: ["ism"] } });
    const dataset = planSearch(f, [...JWST_FILTERS], {}, JWST_GROUPS);
    expect(dataset.kind === "search" && dataset.query.filters).toEqual({});
    const without = planSearch({ ...f, text: "2739" }, [...JWST_FILTERS]);
    expect(without.kind === "search" && without.query.filters).toEqual({ level: "combined" });
  });

  it("still blocks a query that has only object filters", () => {
    expect(planSearch(form({ text: "", filters: { categories: ["galaxies"] } }), [...JWST_FILTERS], {}, JWST_GROUPS)).toEqual({ kind: "blocked", field: "query", message: EMPTY_QUERY_HINT });
  });
});

describe("filters panel memory", () => {
  it("is closed by default, with an unknown value, without storage or when storage throws", () => {
    expect(FILTERS_OPEN_STORAGE_KEY).toBe("astroburst.mast.filtersOpen");
    expect(loadFiltersOpen(null)).toBe(false);
    expect(loadFiltersOpen({ getItem: () => "yes" })).toBe(false);
    expect(loadFiltersOpen({ getItem: () => { throw new Error("blocked"); } })).toBe(false);
  });

  it("writes 1 and 0 under its key and swallows storage errors", () => {
    const store = new Map<string, string>();
    const storage = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) };
    saveFiltersOpen(storage, true);
    expect(store.get(FILTERS_OPEN_STORAGE_KEY)).toBe("1");
    expect(loadFiltersOpen(storage)).toBe(true);
    saveFiltersOpen(storage, false);
    expect(store.get(FILTERS_OPEN_STORAGE_KEY)).toBe("0");
    expect(loadFiltersOpen(storage)).toBe(false);
    expect(() => saveFiltersOpen({ setItem: () => { throw new Error("quota"); } }, true)).not.toThrow();
    expect(() => saveFiltersOpen(null, true)).not.toThrow();
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
      preset: null,
    });
  });

  it("keeps the category and object type ids the target mission offers", () => {
    const before = form({ text: "M16", filters: { level: "exposures", dateFrom: "2022-01-01", categories: ["galaxies", "galaxy_clusters", "stars"], objectTypes: ["exoplanets"] } });
    expect(switchMission(before, "hst", null, HST_GROUPS).filters).toEqual({ dateFrom: "2022-01-01", categories: ["galaxies", "galaxy_clusters", "stars"], objectTypes: ["exoplanets"] });
    expect(switchMission(before, "iue", null, IUE_GROUPS).filters).toEqual({ dateFrom: "2022-01-01", categories: ["galaxies", "stars"] });
    expect(switchMission(before, "classy", null, []).filters).toEqual({ dateFrom: "2022-01-01" });
    expect(switchMission(before, "hst", null).filters).toEqual({ dateFrom: "2022-01-01" });
  });

  it("restores the remembered JWST level when switching back", () => {
    const storage = { getItem: vi.fn(() => "all"), setItem: vi.fn() };
    const after = switchMission(form({ mission: "hst", filters: {} }), "jwst", storage);
    expect(after.filters.level).toBe("all");
    expect(storage.getItem).toHaveBeenCalledWith(LEVEL_STORAGE_KEY);
  });
});

describe("level memory", () => {
  it("remembers Exposures L1-2 as its own level", () => {
    const store = new Map<string, string>();
    const storage = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) };
    saveLevel(storage, "exposures");
    expect(store.get(LEVEL_STORAGE_KEY)).toBe("exposures");
    expect(loadLevel(storage)).toBe("exposures");
    expect(switchMission(form({ mission: "hst", filters: {} }), "jwst", storage).filters.level).toBe("exposures");
  });

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
