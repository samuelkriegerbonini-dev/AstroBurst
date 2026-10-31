import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { CatalogDatasetReport, CatalogStatus } from "../../services/catalogStore";
import {
  DEFAULT_SEARCH_RADIUS_TEXT,
  DEFAULT_SOLVE_OPTIONS,
  SIP_ORDER_CHOICES,
  SOLVE_OPTIONS_KEY,
  loadSolveOptions,
  offlineCatalogueNames,
  offlineSearchRadius,
  optionsSolver,
  saveSolveOptions,
  searchRadiusError,
  searchRadiusPlaceholder,
  sipOrderCap,
  sipOrderChoice,
  solveOptionsStorage,
  solveOptionsSummary,
  twomassMissing,
  type SolveOptions,
} from "../plateSolveOptions";

function memoryStorage(initial: Record<string, string> = {}) {
  const items = new Map(Object.entries(initial));
  return {
    items,
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
  };
}

const brokenStorage = {
  getItem: (): string | null => {
    throw new Error("SecurityError");
  },
  setItem: (): void => {
    throw new Error("QuotaExceededError");
  },
};

function report(id: string, status: CatalogStatus): CatalogDatasetReport {
  return {
    id,
    title: id,
    kind: "hosted",
    status,
    size_bytes: null,
    installable: true,
    compressed: "none",
    licence: "",
    credit: "",
    url_host: null,
    folder: "C:/catalogs",
    folder_bytes: 0,
    free_space_bytes: null,
  };
}

const TYCHO2 = report("tycho2-l6-v1", { state: "installed", path: "C:/res/tycho2.bin", verified: true });
const SIRIL = report("siril-gaia-astro-v1", { state: "external", path: "D:/siril/cat.dat", valid: true, problem: null });
const TWOMASS = report("2mass-l8-v1", { state: "installed", path: "C:/catalogs/2mass.bin", verified: true });

describe("Solve options storage", () => {
  it("starts with online Gaia, 2MASS and the astrometry.net offer on, the automatic SIP order, the backend radius and the header hints", () => {
    expect(DEFAULT_SOLVE_OPTIONS).toEqual({
      onlineGaia: true,
      offerNova: true,
      useTwomass: true,
      maxSipOrder: "auto",
      searchRadius: "",
      hints: "header",
    });
    expect(loadSolveOptions(null)).toEqual({ options: DEFAULT_SOLVE_OPTIONS, typedHints: {} });
    expect(loadSolveOptions(memoryStorage())).toEqual({ options: DEFAULT_SOLVE_OPTIONS, typedHints: {} });
  });

  it("keeps the options and the typed hints for the next session", () => {
    const storage = memoryStorage();
    const options: SolveOptions = { onlineGaia: false, offerNova: false, useTwomass: false, maxSipOrder: 3, searchRadius: "5", hints: "typed" };
    saveSolveOptions(storage, options, { centerRa: "10.5", centerDec: "-20.25" });
    expect(JSON.parse(storage.items.get(SOLVE_OPTIONS_KEY)!)).toEqual({
      onlineGaia: false,
      offerNova: false,
      useTwomass: false,
      maxSipOrder: 3,
      searchRadius: "5",
      hints: "typed",
      typedHints: { centerRa: "10.5", centerDec: "-20.25" },
    });
    expect(loadSolveOptions(storage)).toEqual({ options, typedHints: { centerRa: "10.5", centerDec: "-20.25" } });
  });

  it("reads the options saved before the 2MASS switch and the SIP order existed, with those two at their defaults", () => {
    const storage = memoryStorage({
      [SOLVE_OPTIONS_KEY]: JSON.stringify({
        onlineGaia: false,
        offerNova: false,
        searchRadius: "5",
        hints: "typed",
        typedHints: { centerRa: "10.5" },
      }),
    });
    expect(loadSolveOptions(storage)).toEqual({
      options: { onlineGaia: false, offerNova: false, useTwomass: true, maxSipOrder: "auto", searchRadius: "5", hints: "typed" },
      typedHints: { centerRa: "10.5" },
    });
  });

  it("falls back to the default of each stored value it cannot use", () => {
    const storage = memoryStorage({
      [SOLVE_OPTIONS_KEY]: JSON.stringify({
        onlineGaia: "no",
        offerNova: false,
        useTwomass: "off",
        maxSipOrder: 5,
        searchRadius: "20",
        hints: "typed",
        typedHints: { centerRa: "10.5", scaleUnits: "furlongs", radius: 5, bogus: "x" },
      }),
    });
    expect(loadSolveOptions(storage)).toEqual({
      options: { onlineGaia: true, offerNova: false, useTwomass: true, maxSipOrder: "auto", searchRadius: "", hints: "typed" },
      typedHints: { centerRa: "10.5" },
    });
  });

  it("reads back each SIP order cap it saves and nothing else", () => {
    for (const choice of SIP_ORDER_CHOICES) {
      const storage = memoryStorage();
      saveSolveOptions(storage, { ...DEFAULT_SOLVE_OPTIONS, maxSipOrder: choice }, {});
      expect(loadSolveOptions(storage).options.maxSipOrder, String(choice)).toBe(choice);
    }
    for (const stored of [0, 1, 5, 3.5, "four", null, true, [3]]) {
      const storage = memoryStorage({ [SOLVE_OPTIONS_KEY]: JSON.stringify({ maxSipOrder: stored }) });
      expect(loadSolveOptions(storage).options.maxSipOrder, String(stored)).toBe("auto");
    }
  });

  it("forgets typed hints stored under the header policy", () => {
    const storage = memoryStorage({ [SOLVE_OPTIONS_KEY]: JSON.stringify({ hints: "header", typedHints: { centerRa: "10.5" } }) });
    expect(loadSolveOptions(storage).typedHints).toEqual({});
  });

  it("uses the defaults when the stored text is not JSON or storage refuses", () => {
    expect(loadSolveOptions(memoryStorage({ [SOLVE_OPTIONS_KEY]: "{not json" })).options).toEqual(DEFAULT_SOLVE_OPTIONS);
    expect(loadSolveOptions(brokenStorage).options).toEqual(DEFAULT_SOLVE_OPTIONS);
    expect(() => saveSolveOptions(brokenStorage, DEFAULT_SOLVE_OPTIONS, {})).not.toThrow();
  });

  it("has no storage outside a browser window", () => {
    expect(solveOptionsStorage()).toBeNull();
  });
});

describe("Solve options applied to a solve", () => {
  it("solves offline only when online Gaia is off", () => {
    expect(optionsSolver(DEFAULT_SOLVE_OPTIONS)).toBe("auto");
    expect(optionsSolver({ ...DEFAULT_SOLVE_OPTIONS, onlineGaia: false })).toBe("local");
  });

  it("accepts a blank radius or 0 to 15 degrees written with a dot", () => {
    for (const text of ["", " ", "0", "3", "15", "7.5"]) expect(searchRadiusError({ searchRadius: text }), text).toBeNull();
    expect(searchRadiusError({ searchRadius: "15.5" })).toBe("Use a value from 0 to 15");
    expect(searchRadiusError({ searchRadius: "-1" })).toBe("Use a value from 0 to 15");
    expect(searchRadiusError({ searchRadius: "2,5" })).toBe("Use a dot (.) as the decimal separator");
    expect(searchRadiusError({ searchRadius: "wide" })).toBe("Not a number");
  });

  it("lets a radius in the hint fields win, capped at 15, and otherwise sends the options radius", () => {
    expect(offlineSearchRadius(16.9706, { searchRadius: "5" })).toBe(15);
    expect(offlineSearchRadius(2, { searchRadius: "5" })).toBe(2);
    expect(offlineSearchRadius(null, { searchRadius: "5" })).toBe(5);
    expect(offlineSearchRadius(null, { searchRadius: "0" })).toBe(0);
    expect(offlineSearchRadius(null, { searchRadius: "" })).toBeUndefined();
    expect(offlineSearchRadius(null, { searchRadius: "20" })).toBeUndefined();
  });

  it("shows the radius a blank hint field falls back to", () => {
    expect(DEFAULT_SEARCH_RADIUS_TEXT).toBe("3");
    expect(searchRadiusPlaceholder({ searchRadius: "" })).toBe("3");
    expect(searchRadiusPlaceholder({ searchRadius: "5" })).toBe("5");
    expect(searchRadiusPlaceholder({ searchRadius: "20" })).toBe("3");
  });

  it("leaves the SIP order to the solver's own rule on Auto and sends the cap otherwise", () => {
    expect(sipOrderCap(DEFAULT_SOLVE_OPTIONS)).toBeUndefined();
    expect(sipOrderCap({ maxSipOrder: 2 })).toBe(2);
    expect(sipOrderCap({ maxSipOrder: 4 })).toBe(4);
  });

  it("reads the SIP order the select picks", () => {
    expect(sipOrderChoice("auto")).toBe("auto");
    expect(sipOrderChoice("2")).toBe(2);
    expect(sipOrderChoice("3")).toBe(3);
    expect(sipOrderChoice("4")).toBe(4);
    expect(sipOrderChoice("5")).toBe("auto");
  });

  it("offers Auto and exactly the SIP orders plate_solve_cmd accepts", () => {
    const rust = readFileSync(fileURLToPath(new URL("../../../src-tauri/src/cmd/astrometry.rs", import.meta.url)), "utf8");
    const bound = (name: string) => Number(new RegExp(`const ${name}: u8 = (\\d+);`).exec(rust)?.[1]);
    const accepted = Array.from({ length: bound("MAX_SIP_ORDER_CAP") - bound("MIN_SIP_ORDER_CAP") + 1 }, (_, i) => bound("MIN_SIP_ORDER_CAP") + i);
    expect(accepted).toEqual([2, 3, 4]);
    expect(SIP_ORDER_CHOICES).toEqual(["auto", ...accepted]);
  });
});

describe("Solve options summary", () => {
  it("names the offline catalogues the solve uses, in the order it tries them", () => {
    expect(offlineCatalogueNames([TWOMASS, SIRIL, TYCHO2])).toEqual(["Tycho-2", "Gaia DR3 (Siril extract)", "2MASS"]);
    expect(offlineCatalogueNames([TYCHO2, TWOMASS])).toEqual(["Tycho-2", "2MASS"]);
    expect(offlineCatalogueNames(null)).toBeNull();
  });

  it("leaves out a missing or partial catalogue, a broken external file and other datasets", () => {
    expect(
      offlineCatalogueNames([
        report("tycho2-l6-v1", { state: "missing" }),
        report("siril-gaia-astro-v1", { state: "external", path: "D:/x.dat", valid: false, problem: "bad header" }),
        report("2mass-l8-v1", { state: "partial", received: 10, total: 100 }),
        report("vizier-cache", { state: "cache", bytes: 1, files: 1, cap: 10 }),
      ]),
    ).toEqual([]);
  });

  it("says in one line what the next solve uses", () => {
    expect(solveOptionsSummary(DEFAULT_SOLVE_OPTIONS, ["Tycho-2", "2MASS"])).toBe(
      "Offline: Tycho-2 + 2MASS, then online Gaia DR3, astrometry.net offered after a failure",
    );
    expect(solveOptionsSummary({ ...DEFAULT_SOLVE_OPTIONS, onlineGaia: false, offerNova: false }, ["Tycho-2", "2MASS"])).toBe(
      "Offline: Tycho-2 + 2MASS, online off, no astrometry.net",
    );
  });

  it("says when no offline catalogue is installed, and does not guess while the list is unknown", () => {
    expect(solveOptionsSummary({ ...DEFAULT_SOLVE_OPTIONS, onlineGaia: false }, [])).toBe(
      "No offline catalogue, online off, astrometry.net offered after a failure",
    );
    expect(solveOptionsSummary(DEFAULT_SOLVE_OPTIONS, null)).toBe(
      "Offline catalogues, then online Gaia DR3, astrometry.net offered after a failure",
    );
  });

  it("names 2MASS off and the SIP order cap only when they differ from the defaults", () => {
    expect(solveOptionsSummary({ ...DEFAULT_SOLVE_OPTIONS, useTwomass: false }, ["Tycho-2", "2MASS"])).toBe(
      "Offline: Tycho-2, 2MASS off, then online Gaia DR3, astrometry.net offered after a failure",
    );
    expect(solveOptionsSummary({ ...DEFAULT_SOLVE_OPTIONS, maxSipOrder: 2 }, ["Tycho-2", "2MASS"])).toBe(
      "Offline: Tycho-2 + 2MASS, then online Gaia DR3, SIP up to 2, astrometry.net offered after a failure",
    );
    expect(
      solveOptionsSummary({ ...DEFAULT_SOLVE_OPTIONS, onlineGaia: false, offerNova: false, useTwomass: false, maxSipOrder: 4 }, ["Tycho-2", "2MASS"]),
    ).toBe("Offline: Tycho-2, 2MASS off, online off, SIP up to 4, no astrometry.net");
    expect(solveOptionsSummary({ ...DEFAULT_SOLVE_OPTIONS, useTwomass: false }, ["2MASS"])).toBe(
      "No offline catalogue, 2MASS off, then online Gaia DR3, astrometry.net offered after a failure",
    );
    expect(solveOptionsSummary({ ...DEFAULT_SOLVE_OPTIONS, useTwomass: false }, null)).toBe(
      "Offline catalogues, 2MASS off, then online Gaia DR3, astrometry.net offered after a failure",
    );
  });

  it("knows 2MASS is missing only once the catalogue list says so", () => {
    expect(twomassMissing(offlineCatalogueNames([TYCHO2]))).toBe(true);
    expect(twomassMissing(offlineCatalogueNames([TYCHO2, TWOMASS]))).toBe(false);
    expect(twomassMissing(offlineCatalogueNames([report("2mass-l8-v1", { state: "partial", received: 10, total: 100 })]))).toBe(true);
    expect(twomassMissing(null)).toBe(false);
  });
});
