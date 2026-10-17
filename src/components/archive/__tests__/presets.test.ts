import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import PresetPicker from "../PresetPicker";
import SummaryLine from "../SummaryLine";
import { DEFAULT_FORM, planSearch, switchMission, type ArchiveForm } from "../logic/buildQuery";
import { conditionText, droppedText, presetById, presetProducts, presetTitle, presetsFor } from "../logic/presetView";
import { summaryView } from "../logic/resultsView";
import { productView } from "../logic/productView";
import { mergePage } from "../logic/paging";
import { archiveReducer, createInitialState, type ArchiveState } from "../state/archiveReducer";
import type { ArchiveController } from "../state/archiveController";
import { MISSIONS, page, row } from "./fixtures";
import type { MastPreset, MastProduct, MastProductClass, MastProductList, MastQuery, MastResolved } from "../../../shared/types/mast";

const JWST_FILTERS = ["level", "instrument", "filter", "date", "public"] as const;
const TSO_TYPES = "NRS_BRIGHTOBJ,NIS_SOSS,MIR_LRS-SLITLESS,NRC_TSGRISM";

const PRESETS: MastPreset[] = [
  {
    id: "tso",
    mission: "jwst",
    label: "TSO",
    description: "Time-series observations: NIRSpec bright object, NIRISS SOSS, MIRI slitless LRS, NIRCam grism",
    conditions: [
      { column: "tsovisit", value: "t" },
      { column: "exp_type", value: TSO_TYPES },
    ],
    products: ["x1dints", "calints"],
  },
  {
    id: "spectra_1d",
    mission: "jwst",
    label: "1D spectra",
    description: "Extracted 1D spectra of any mode",
    conditions: [],
    products: ["x1d"],
  },
  {
    id: "pipeline_catalogs",
    mission: "jwst",
    label: "Pipeline catalogues",
    description: "Source catalogues and segmentation maps of combined products",
    conditions: [{ column: "productLevel", value: "3" }],
    products: ["cat", "segm"],
  },
];

const WASP39: MastResolved = { query: "WASP-39", canonicalName: "WASP-39", resolver: "SIMBAD", raDeg: 217.3266, decDeg: -3.4445, objectType: null, elapsedMs: 700, fromCache: false };

function form(over: Partial<ArchiveForm> = {}): ArchiveForm {
  return { ...DEFAULT_FORM, ...over, filters: { ...DEFAULT_FORM.filters, ...(over.filters ?? {}) } };
}

function product(filename: string, suffix: string, cls: MastProductClass, sizeBytes = 1_000_000): MastProduct {
  return {
    datasetId: "jw01366-o001_t001_nirspec_clear-prism-s1600a1-sub512",
    filename,
    sizeBytes,
    category: cls === "guidestar" ? "GUIDESTAR" : "SCIENCE",
    type: "science",
    suffix,
    cls,
    opensAs: cls === "image" || cls === "cube" || cls === "spectrum" || cls === "ramp" || cls === "catalog" ? cls : null,
    access: "PUBLIC",
    mastUri: `mast:JWST/product/${filename}`,
    downloadUrl: `https://mast.stsci.edu/api/v0.1/Download/file?uri=mast:JWST/product/${filename}`,
    badges: [],
  };
}

function tsoProducts(): MastProductList {
  const products = [
    product("a_x1dints.fits", "x1dints", "spectrum", 40_000_000),
    product("b_calints.fits", "calints", "image", 900_000_000),
    product("a_x1dints.jpg", "x1dints", "preview", 30_000),
    product("c_rateints.fits", "rateints", "image", 800_000_000),
    product("a_whtlt.ecsv", "whtlt", "catalog", 20_000),
    product("gs_calints.fits", "calints", "guidestar", 5_000_000),
  ];
  return {
    mission: "jwst",
    datasetIds: ["jw01366-o001_t001_nirspec_clear-prism-s1600a1-sub512"],
    products,
    groups: [],
    totalCount: products.length,
    totalBytes: products.reduce((sum, p) => sum + p.sizeBytes, 0),
    elapsedMs: 4000,
    fromCache: false,
    cachedUntil: null,
  };
}

function withPresets(over: Partial<ArchiveForm> = {}): ArchiveState {
  const loaded = archiveReducer(archiveReducer(createInitialState(), { type: "missionsLoaded", missions: MISSIONS }), { type: "presetsLoaded", presets: PRESETS });
  return archiveReducer(loaded, { type: "formChanged", patch: over });
}

function stubController(): ArchiveController {
  const fns: Record<string | symbol, unknown> = {};
  return new Proxy(fns, { get: (target, prop) => (target[prop] ??= vi.fn()) }) as unknown as ArchiveController;
}

describe("presets in the query", () => {
  it("starts without a preset", () => {
    expect(DEFAULT_FORM.preset).toBeNull();
    const plan = planSearch(form({ text: "2739" }), [...JWST_FILTERS]);
    expect(plan.kind === "search" && plan.query.preset).toBeNull();
  });

  it("sends the chosen preset with name, coordinate and programme searches", () => {
    const named = planSearch(form({ text: "WASP-39", preset: "tso" }), [...JWST_FILTERS], { resolved: WASP39 });
    expect(named).toMatchObject({ kind: "search", query: { mission: "jwst", preset: "tso", filters: { level: "combined" } } });
    const coordinates = planSearch(form({ text: "217.3266 -3.4445", preset: "tso" }), [...JWST_FILTERS]);
    expect(coordinates.kind === "search" && coordinates.query.preset).toBe("tso");
    const programme = planSearch(form({ text: "1366", preset: "tso" }), [...JWST_FILTERS]);
    expect(programme).toMatchObject({ kind: "search", query: { program: 1366, preset: "tso" } });
    const server = planSearch(form({ text: "WASP-39", preset: "tso" }), [...JWST_FILTERS], { serverResolve: true });
    expect(server.kind === "search" && server.query.preset).toBe("tso");
  });

  it("leaves the preset out of dataset searches, as it does the quick filters", () => {
    const plan = planSearch(form({ text: "jw01366-o001_t001_nirspec_clear-prism-s1600a1-sub512", preset: "tso" }), [...JWST_FILTERS]);
    expect(plan).toMatchObject({ kind: "search", query: { datasetId: "jw01366-o001_t001_nirspec_clear-prism-s1600a1-sub512", preset: null, filters: {} } });
  });

  it("clears the preset when the mission changes", () => {
    expect(switchMission(form({ text: "M16", preset: "tso" }), "hst", null).preset).toBeNull();
    expect(switchMission(form({ mission: "hst", text: "M16", preset: null }), "jwst", null).preset).toBeNull();
  });
});

describe("presetView", () => {
  it("lists the presets of a mission and finds one by id", () => {
    expect(presetsFor(PRESETS, "jwst").map((p) => p.id)).toEqual(["tso", "spectra_1d", "pipeline_catalogs"]);
    expect(presetsFor(PRESETS, "hst")).toEqual([]);
    expect(presetById(PRESETS, "tso")?.label).toBe("TSO");
    expect(presetById(PRESETS, "missing")).toBeNull();
    expect(presetById(PRESETS, null)).toBeNull();
  });

  it("describes the MAST conditions and the files a preset keeps", () => {
    expect(conditionText(PRESETS[0])).toBe(`tsovisit = t, exp_type = ${TSO_TYPES}`);
    expect(conditionText(PRESETS[1])).toBe("no MAST condition");
    expect(presetTitle(PRESETS[0])).toBe(`${PRESETS[0].description}. MAST conditions: tsovisit = t, exp_type = ${TSO_TYPES}. Files: x1dints, calints`);
  });

  it("names the conditions MAST left out of its answer", () => {
    expect(droppedText([])).toBeNull();
    expect(droppedText(["tsovisit"])).toBe("MAST ignored tsovisit");
    expect(droppedText(["tsovisit", "exp_type"])).toBe("MAST ignored tsovisit, exp_type");
  });

  it("keeps the files whose suffix the preset names, without previews or guide-star files", () => {
    expect(presetProducts(PRESETS[0], tsoProducts().products).map((p) => p.filename)).toEqual(["a_x1dints.fits", "b_calints.fits"]);
    expect(presetProducts(PRESETS[2], tsoProducts().products)).toEqual([]);
  });
});

describe("preset results", () => {
  const query: MastQuery = {
    mission: "jwst",
    where: { kind: "position", raDeg: 217.3266, decDeg: -3.4445, radiusArcmin: 3, label: "WASP-39" },
    program: null,
    datasetId: null,
    filters: { level: "combined" },
    preset: "tso",
    sort: [],
    offset: 0,
    limit: 200,
  };

  it("puts the preset name in the summary and keeps what MAST dropped across pages", () => {
    const first = mergePage(null, page([row("jw01366-o001_t001_nirspec_clear-prism-s1600a1-sub512")], { hasMore: true, totalReliable: false, droppedConditions: ["tsovisit"] }));
    expect(first.droppedConditions).toEqual(["tsovisit"]);
    const next = mergePage(first, page([row("jw01366-o002_t001_niriss_clear-gr700xd-substrip256")], { offset: 200, droppedConditions: [] }));
    expect(next.droppedConditions).toEqual(["tsovisit"]);
    const v = summaryView({ results: first, resolved: WASP39, query, missionLabel: "JWST", presets: PRESETS });
    expect(v.text).toContain(' 217.32660 -3.44450 · Preset TSO · r 3\' · ');
    expect(v.preset).toBe("tso");
    expect(v.ignored).toBe("MAST ignored tsovisit");
    const plain = summaryView({ results: mergePage(null, page([row("a")])), resolved: null, query: { ...query, preset: null }, missionLabel: "JWST", presets: PRESETS });
    expect(plain.text).not.toContain("Preset");
    expect(plain.preset).toBeNull();
    expect(plain.ignored).toBeNull();
  });

  it("shows the preset name and the dropped conditions on the summary line", () => {
    const state = withPresets();
    const loaded: ArchiveState = { ...state, results: mergePage(null, page([row("a")], { droppedConditions: ["tsovisit", "exp_type"] })), resultsQuery: query };
    const html = renderToStaticMarkup(createElement(SummaryLine, { state: loaded, ctl: stubController() }));
    expect(html).toContain('data-preset="tso"');
    expect(html).toContain("Preset TSO");
    expect(html).toContain('data-testid="mast-summary-ignored"');
    expect(html).toContain("MAST ignored tsovisit, exp_type");
  });

  it("lists the preset files first and folds the other groups away", () => {
    const view = productView(tsoProducts(), "opens", false, PRESETS[0]);
    expect(view.sections.map((s) => [s.key, s.count, s.expanded])).toEqual([
      ["preset", 2, true],
      ["opens", 1, false],
      ["catalog", 1, false],
      ["preview", 1, false],
    ]);
    expect(view.sections[0]).toMatchObject({ title: "TSO files", detail: "2 files, 940 MB" });
    expect(view.visible.map((p) => p.filename)).toEqual(["a_x1dints.fits", "b_calints.fits"]);
    expect(view.presetNote).toBeNull();
    const everything = productView(tsoProducts(), "everything", false, PRESETS[0]);
    expect(everything.sections.every((s) => s.expanded)).toBe(true);
    const none = productView(tsoProducts(), "opens", false, PRESETS[2]);
    expect(none.sections.map((s) => s.key)).toEqual(["opens", "catalog", "preview"]);
    expect(none.presetNote).toBe("No Pipeline catalogues files (cat, segm) in this dataset");
    expect(productView(tsoProducts(), "opens", false).presetNote).toBeNull();
  });
});

describe("PresetPicker", () => {
  it("offers none and the JWST presets, with the active one selected", () => {
    const html = renderToStaticMarkup(createElement(PresetPicker, { state: withPresets({ preset: "tso" }), ctl: stubController() }));
    expect(html).toContain('data-testid="mast-preset"');
    expect(html.match(/<option/g)).toHaveLength(4);
    expect(html).toContain('<option value="">none</option>');
    expect(html).toMatch(/<option value="tso"[^>]*selected=""[^>]*>TSO<\/option>/);
    expect(html).toContain(`title="${PRESETS[0].description}. MAST conditions: tsovisit = t, exp_type = ${TSO_TYPES}. Files: x1dints, calints"`);
  });

  it("stays hidden on missions without presets and locks while a search runs", () => {
    const hst = withPresets({ mission: "hst" });
    expect(renderToStaticMarkup(createElement(PresetPicker, { state: hst, ctl: stubController() }))).toBe("");
    const empty = archiveReducer(createInitialState(), { type: "missionsLoaded", missions: MISSIONS });
    expect(renderToStaticMarkup(createElement(PresetPicker, { state: empty, ctl: stubController() }))).toBe("");
    const running = { ...withPresets(), run: { generation: 1 } } as unknown as ArchiveState;
    expect(renderToStaticMarkup(createElement(PresetPicker, { state: running, ctl: stubController() }))).toContain("disabled");
  });
});
