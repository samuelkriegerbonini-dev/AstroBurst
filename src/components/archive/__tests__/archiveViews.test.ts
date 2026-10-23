import { describe, expect, it, vi } from "vitest";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import QueryBar from "../QueryBar";
import MissionTabs from "../MissionTabs";
import QuickFilters, { ViewToggle } from "../QuickFilters";
import { ActiveFilters, FilterPanel, FiltersToggle } from "../ObjectFilters";
import { BlockingError } from "../StateViews";
import SummaryLine from "../SummaryLine";
import ResultsRegion from "../ResultsRegion";
import Banner from "../Banner";
import ArchiveToast from "../ArchiveToast";
import ArchiveLiveRegion from "../ArchiveLiveRegion";
import { FooterStatusView } from "../ArchiveFooterStatus";
import ArchiveRoot from "../ArchiveRoot";
import { archiveReducer, createInitialState, type ArchiveAction, type ArchiveState } from "../state/archiveReducer";
import { chipAfterRemoval } from "../logic/objectFilters";
import type { ArchiveController } from "../state/archiveController";
import { COLUMNS, MISSIONS, page, row } from "./fixtures";
import type { MastColumn, MastError, MastFilterValues, MastPreset, MastQuery, MastResolved } from "../../../shared/types/mast";

const M16: MastResolved = { query: "M16", canonicalName: "M 16", resolver: "SIMBAD", raDeg: 274.688, decDeg: -13.792, objectType: null, elapsedMs: 870, fromCache: false };
const QUERY: MastQuery = {
  mission: "jwst",
  where: { kind: "position", raDeg: 274.688, decDeg: -13.792, radiusArcmin: 3, label: "M 16" },
  program: null,
  datasetId: null,
  filters: { level: "combined" },
  preset: null,
  sort: [],
  offset: 0,
  limit: 200,
};
const ROWS = [row("jw02739-o001_t001_nircam"), row("jw02739-o002_t001_miri", { instrument: "MIRI", filters: ["F770W", "F1130W"], access: "PROPRIETARY", releaseDate: "2027-03-02T00:00:00" })];

function stubController(): ArchiveController {
  const fns: Record<string | symbol, unknown> = {};
  return new Proxy(fns, { get: (target, prop) => (target[prop] ??= vi.fn()) }) as unknown as ArchiveController;
}

function run(state: ArchiveState, ...actions: ArchiveAction[]): ArchiveState {
  return actions.reduce(archiveReducer, state);
}

function base(): ArchiveState {
  return run(createInitialState(), { type: "missionsLoaded", missions: MISSIONS }, { type: "tokenStatus", status: { present: false, authorized: null } });
}

function searched(state = base(), rows = ROWS): ArchiveState {
  return run(
    { ...state, form: { ...state.form, text: "M16" } },
    { type: "searchStarted", generation: 1, text: "M16", kind: "name", name: "M16", query: null, at: 0 },
    { type: "nameResolved", generation: 1, resolved: M16, query: QUERY, at: 900 },
    { type: "pageLoaded", generation: 1, page: page(rows) },
  );
}

function html<P extends object>(component: ComponentType<P>, props: P): string {
  return renderToStaticMarkup(createElement(component, props));
}

function tag(markup: string, testid: string, extra = ""): string {
  return new RegExp(`<[a-z]+[^>]*data-testid="${testid}"${extra}[^>]*>`).exec(markup)?.[0] ?? "";
}

function count(markup: string, testid: string): number {
  return markup.split(`data-testid="${testid}"`).length - 1;
}

const ctl = stubController();

describe("query bar", () => {
  it("renders the field, the kind chip, the radius and a Search button that is disabled while empty", () => {
    const markup = html(QueryBar, { state: base(), ctl });
    expect(tag(markup, "mast-query")).toContain('placeholder="Target, coordinates, programme or dataset id"');
    expect(markup).not.toContain('data-testid="mast-query-kind"');
    expect(tag(markup, "mast-radius")).toContain('value="3"');
    const search = tag(markup, "mast-search");
    expect(search).toContain("disabled");
    expect(search).toContain('title="Add a target, coordinates, programme or dataset id"');
    expect(markup).toContain('role="search"');
  });

  it("names the interpreted kind exactly", () => {
    const s = base();
    const kind = (text: string) => /data-testid="mast-query-kind"[^>]*>([^<]*)</.exec(html(QueryBar, { state: { ...s, form: { ...s.form, text } }, ctl }))?.[1];
    expect(kind("2739")).toBe("programme");
    expect(kind("ICK909030")).toBe("dataset");
    expect(kind("274.688 -13.792")).toBe("coordinates");
    expect(kind("M16")).toBe("name");
  });

  it("shows Cancel with the Esc hint while a search runs", () => {
    const s = run({ ...base(), form: { ...base().form, text: "M16" } }, { type: "searchStarted", generation: 1, text: "M16", kind: "name", name: "M16", query: null, at: 0 });
    const markup = html(QueryBar, { state: s, ctl });
    expect(tag(markup, "mast-search")).toContain('data-running="true"');
    expect(markup).toMatch(/Cancel[\s\S]*Esc/);
  });

  it("puts an inline error under the field and marks the field invalid", () => {
    const s = run(base(), { type: "queryBlocked", field: "query", message: "Add a target, coordinates, programme or dataset id" });
    const markup = html(QueryBar, { state: s, ctl });
    const field = tag(markup, "mast-query");
    expect(field).toContain('aria-invalid="true"');
    const describedBy = /aria-describedby="([^"]+)"/.exec(field)?.[1];
    expect(describedBy).toBeTruthy();
    expect(tag(markup, "mast-query-error")).toContain(`id="${describedBy}"`);
    expect(markup).toContain("Add a target, coordinates, programme or dataset id</");
  });

  it("marks the radius invalid for a radius error", () => {
    const s = run(base(), { type: "queryBlocked", field: "radius", message: "Radius: type a number of arcminutes, for example 3" });
    const markup = html(QueryBar, { state: s, ctl });
    expect(tag(markup, "mast-radius")).toContain('aria-invalid="true"');
    expect(tag(markup, "mast-query")).not.toContain('aria-invalid="true"');
  });

  it("disables Search for Roman without a token and says why", () => {
    const s = run({ ...base(), form: { ...base().form, text: "M16" } }, { type: "missionSwitched", form: { ...base().form, mission: "roman", text: "M16", filters: {} }, generation: 1 });
    expect(tag(html(QueryBar, { state: s, ctl }), "mast-search")).toContain('title="Roman needs an authorized MAST token"');
  });
});

describe("mission tabs", () => {
  it("lists JWST, HST and a locked Roman as tabs with the current one selected", () => {
    const markup = html(MissionTabs, { state: base(), ctl });
    expect(markup).toContain('role="tablist"');
    expect(count(markup, "mast-mission-tab")).toBe(3);
    expect(tag(markup, "mast-mission-tab", ' data-mission="jwst"')).toContain('aria-selected="true"');
    expect(tag(markup, "mast-mission-tab", ' data-mission="hst"')).toContain('aria-selected="false"');
    const roman = tag(markup, "mast-mission-tab", ' data-mission="roman"');
    expect(roman).toContain('data-lock="locked"');
    expect(markup).toContain("Roman, locked");
  });

  it("adds the current More mission as a fourth tab", () => {
    const s = run(base(), { type: "missionSwitched", form: { ...base().form, mission: "iue", filters: {} }, generation: 1 });
    const markup = html(MissionTabs, { state: s, ctl });
    expect(tag(markup, "mast-mission-tab", ' data-mission="iue"')).toContain('aria-selected="true"');
  });
});

describe("quick filters and view toggle", () => {
  it("renders the JWST filters with Combined L3 active", () => {
    const markup = html(QuickFilters, { state: base(), ctl });
    for (const id of ["level", "instrument", "filter", "date", "public"]) expect(markup).toContain(`data-testid="mast-filter-${id}"`);
    expect(markup).toMatch(/data-active="true"[^>]*>Combined L3</);
    expect(markup).toMatch(/<option value=""[^>]*>any<\/option>/);
    expect(markup).toMatch(/<option value="MIRI"[^>]*>MIRI<\/option>/);
  });

  it("renders only the HST filters and disables them while searching", () => {
    let s = run(base(), { type: "missionSwitched", form: { ...base().form, mission: "hst", filters: {} }, generation: 1 });
    expect(html(QuickFilters, { state: s, ctl })).not.toContain('data-testid="mast-filter-level"');
    s = run({ ...s, form: { ...s.form, text: "M16" } }, { type: "searchStarted", generation: 2, text: "M16", kind: "name", name: "M16", query: null, at: 0 });
    expect(tag(html(QuickFilters, { state: s, ctl }), "mast-filter-instrument")).toContain("disabled");
  });

  it("marks the active view and disables Gallery for a mission without images", () => {
    const markup = html(ViewToggle, { state: base(), ctl });
    expect(markup).toMatch(/data-testid="mast-view-toggle"/);
    expect(markup).toMatch(/data-view="gallery"[^>]*aria-pressed="true"/);
    expect(markup).toMatch(/data-view="table"[^>]*aria-pressed="false"/);
    const iue = run(base(), { type: "missionSwitched", form: { ...base().form, mission: "iue", filters: {} }, generation: 1 });
    const off = html(ViewToggle, { state: iue, ctl });
    expect(off).toMatch(/data-view="gallery"[^>]*aria-pressed="false"[^>]*aria-disabled="true"/);
    expect(off).toMatch(/data-view="table"[^>]*aria-pressed="true"/);
    expect(off).toContain("serves no thumbnails or previews");
  });
});

describe("object filters", () => {
  const IFU: MastPreset = {
    id: "ifu_cubes",
    mission: "jwst",
    label: "IFU cubes",
    description: "NIRSpec IFU and MIRI MRS combined cubes",
    conditions: [
      { column: "exp_type", value: "NRS_IFU,MIR_MRS" },
      { column: "productLevel", value: "3" },
    ],
    products: ["s3d", "x1d"],
  };
  const PURE_PARALLEL: MastPreset = { id: "pure_parallel", mission: "jwst", label: "Pure parallel", description: "Pure-parallel visits", conditions: [{ column: "visitype", value: "PARALLEL_PURE" }], products: ["cal"] };

  function withFilters(filters: MastFilterValues, state = base()): ArchiveState {
    return { ...state, form: { ...state.form, filters } };
  }

  function on(mission: "iue" | "classy" | "hst"): ArchiveState {
    return run(base(), { type: "missionSwitched", form: { ...base().form, mission, filters: {} }, generation: 1 });
  }

  function levelButtons(markup: string): string[] {
    const start = markup.indexOf('data-testid="mast-filter-level"');
    return [...markup.slice(start, markup.indexOf("</div>", start)).matchAll(/<button[^>]*>/g)].map((m) => m[0]);
  }

  it("renders the Filters disclosure with its state and the number of active filters in its name", () => {
    const closed = tag(html(FiltersToggle, { state: base(), open: false, onToggle: vi.fn() }), "mast-filters-toggle");
    expect(closed).toContain('aria-expanded="false"');
    expect(closed).toContain('aria-controls="mast-filter-panel"');
    expect(closed).toContain('aria-label="Filters"');
    const open = tag(html(FiltersToggle, { state: withFilters({ level: "combined", categories: ["galaxies", "ism"], objectTypes: ["exoplanets"] }), open: true, onToggle: vi.fn() }), "mast-filters-toggle");
    expect(open).toContain('aria-expanded="true"');
    expect(open).toContain('aria-label="Filters, 3 active"');
    expect(html(FiltersToggle, { state: on("classy"), open: false, onToggle: vi.fn() })).toBe("");
  });

  it("renders each group as a fieldset described by its info text, with pressed options that stay enabled during a run", () => {
    const running = run(withFilters({ level: "combined", categories: ["galaxies"] }, { ...base(), form: { ...base().form, text: "M16" } }), {
      type: "searchStarted",
      generation: 1,
      text: "M16",
      kind: "name",
      name: "M16",
      query: null,
      at: 0,
    });
    const markup = html(FilterPanel, { state: { ...running, form: { ...running.form, filters: { level: "combined", categories: ["galaxies"] } } }, ctl, open: true });
    expect(tag(markup, "mast-filter-panel")).toContain('id="mast-filter-panel"');
    expect(tag(markup, "mast-filter-group-category")).toMatch(/^<fieldset[^>]*aria-describedby="mast-filter-info-category"/);
    expect(tag(markup, "mast-filter-group-object_type")).toMatch(/^<fieldset[^>]*aria-describedby="mast-filter-info-object_type"/);
    expect(markup).toMatch(/<legend[^>]*>[\s\S]*?Category[\s\S]*?<\/legend>/);
    expect(markup).toMatch(/id="mast-filter-info-category"[^>]*>From the proposal: the target category chosen in APT \(TARGCAT\).</);
    expect(tag(markup, "mast-filter-group-category")).toContain('aria-labelledby="mast-filter-label-category"');
    expect(markup).toMatch(/<span[^>]*id="mast-filter-label-category"[^>]*>Category<\/span>/);
    expect(markup).toMatch(/<button[^>]*aria-label="About Category"/);
    expect(markup).not.toMatch(/<button[^>]*aria-describedby=/);
    expect(count(markup, "mast-filter-option-category-galaxies")).toBe(1);
    expect(markup.split('data-testid="mast-filter-option-category-').length - 1).toBe(8);
    expect(markup.split('data-testid="mast-filter-option-object_type-').length - 1).toBe(8);
    const galaxies = tag(markup, "mast-filter-option-category-galaxies");
    expect(galaxies).toContain('aria-pressed="true"');
    expect(galaxies).toContain('title="targcat Galaxies"');
    expect(galaxies).not.toContain("disabled");
    expect(tag(markup, "mast-filter-option-category-ism")).toContain('aria-pressed="false"');
    expect(markup).not.toContain('data-testid="mast-preset-object-note"');
  });

  it("keeps the controlled panel in place but hides it while closed", () => {
    const markup = html(FilterPanel, { state: base(), ctl, open: false });
    expect(tag(markup, "mast-filter-panel")).toContain("hidden");
    expect(markup).not.toContain("mast-filter-option-");
    expect(html(FilterPanel, { state: on("classy"), ctl, open: true })).toBe("");
  });

  it("lists active filters as a list of removable chips with Clear, and nothing without them", () => {
    const markup = html(ActiveFilters, { state: withFilters({ level: "combined", categories: ["galaxies"], objectTypes: ["planetary_nebulae"] }), ctl });
    expect(tag(markup, "mast-filter-chips")).toContain('role="list"');
    expect(tag(markup, "mast-filter-chip-category-galaxies")).toContain('aria-label="Remove filter Galaxies"');
    expect(tag(markup, "mast-filter-chip-object_type-planetary_nebulae")).toContain('aria-label="Remove filter Planetary nebulae"');
    expect(markup.indexOf("mast-filter-chip-category-galaxies")).toBeLessThan(markup.indexOf("mast-filter-chip-object_type-planetary_nebulae"));
    expect(tag(markup, "mast-filters-reset")).toContain('aria-label="Clear object filters"');
    expect(markup).toContain(">Clear<");
    expect(html(ActiveFilters, { state: base(), ctl })).toBe("");
  });

  it("moves focus after a chip is removed to the next chip, else the previous chip, else nowhere so the Filters button takes it", () => {
    const chips = ["galaxies", "ism", "exoplanets"];
    expect(chipAfterRemoval(chips, 0)).toBe("ism");
    expect(chipAfterRemoval(chips, 1)).toBe("exoplanets");
    expect(chipAfterRemoval(chips, 2)).toBe("ism");
    expect(chipAfterRemoval(["galaxies"], 0)).toBeNull();
  });

  it("puts the Filters button between the preset and the view toggle, with the active row and panel under the filter row", () => {
    const s = withFilters({ level: "combined", categories: ["galaxies"] }, run(base(), { type: "presetsLoaded", presets: [IFU] }));
    const markup = html(QueryBar, { state: s, ctl });
    const preset = markup.indexOf('data-testid="mast-preset"');
    const toggle = markup.indexOf('data-testid="mast-filters-toggle"');
    const view = markup.indexOf('data-testid="mast-view-toggle"');
    expect(preset).toBeGreaterThan(0);
    expect(toggle).toBeGreaterThan(preset);
    expect(view).toBeGreaterThan(toggle);
    expect(markup.indexOf('data-testid="mast-filter-chips"')).toBeGreaterThan(view);
    expect(markup.indexOf('data-testid="mast-filter-panel"')).toBeGreaterThan(view);
  });

  it("marks the query bar while the filter panel is open, so a short window keeps the active chips to one row beside it", () => {
    const s = withFilters({ level: "combined", categories: ["galaxies"] });
    const bar = (stored: string | null) => {
      vi.stubGlobal("localStorage", { getItem: () => stored, setItem: vi.fn() });
      try {
        return /<div role="search"[^>]*>/.exec(html(QueryBar, { state: s, ctl }))?.[0] ?? "";
      } finally {
        vi.unstubAllGlobals();
      }
    };
    expect(bar("1")).toContain('data-filters-open="true"');
    expect(bar("0")).toContain('data-filters-open="false"');
  });

  it("shows IUE's Filters button and five categories, without the 'No quick filters' text", () => {
    const iue = on("iue");
    expect(html(QuickFilters, { state: iue, ctl })).not.toContain("No quick filters");
    expect(html(QueryBar, { state: iue, ctl })).toContain('data-testid="mast-filters-toggle"');
    const panel = html(FilterPanel, { state: iue, ctl, open: true });
    expect(panel.split('data-testid="mast-filter-option-category-').length - 1).toBe(5);
    expect(panel).not.toContain("mast-filter-group-object_type");
    expect(html(QuickFilters, { state: on("classy"), ctl })).toContain("No quick filters for this mission");
  });

  it("notes in the panel that pure-parallel visits carry no category", () => {
    const s = run(base(), { type: "presetsLoaded", presets: [PURE_PARALLEL] });
    const markup = html(FilterPanel, { state: { ...s, form: { ...s.form, preset: "pure_parallel" } }, ctl, open: true });
    expect(tag(markup, "mast-preset-object-note")).not.toBe("");
    expect(markup).toContain("Pure-parallel visits carry no proposal category, so Category and Object type match nothing.");
  });

  it("offers three levels and remembers Exposures L1-2 as active", () => {
    const markup = html(QuickFilters, { state: withFilters({ level: "exposures" }), ctl });
    const buttons = levelButtons(markup);
    expect(buttons).toHaveLength(3);
    expect(markup).toMatch(/>Combined L3</);
    expect(markup).toMatch(/data-active="true"[^>]*>Exposures L1-2</);
    expect(markup).toMatch(/>All levels</);
    expect(markup).not.toContain('data-testid="mast-level-lock"');
    for (const b of buttons) expect(b).not.toContain('aria-disabled="true"');
  });

  it("locks the Level control with a visible, focusable reason when a preset sets the level", () => {
    const s = run(base(), { type: "presetsLoaded", presets: [IFU] });
    const markup = html(QuickFilters, { state: { ...s, form: { ...s.form, preset: "ifu_cubes", filters: { level: "exposures" } } }, ctl });
    const buttons = levelButtons(markup);
    expect(buttons).toHaveLength(3);
    for (const b of buttons) {
      expect(b).toContain('aria-disabled="true"');
      expect(b).toContain('aria-describedby="mast-level-lock"');
      expect(b).not.toMatch(/\sdisabled=""/);
    }
    expect(tag(markup, "mast-level-lock")).toContain('id="mast-level-lock"');
    expect(markup).toContain(">Level 3 set by preset IFU cubes<");
  });
});

describe("summary line", () => {
  it("exposes the position, the total and the applied conditions", () => {
    const markup = html(SummaryLine, { state: searched(), ctl });
    const summary = tag(markup, "mast-summary");
    expect(summary).toContain('data-ra="274.688"');
    expect(summary).toContain('data-dec="-13.792"');
    expect(summary).toContain('data-total="2"');
    expect(summary).toContain('data-applied="productLevel = 3"');
    expect(markup).toContain("M 16");
  });
});

describe("results region", () => {
  it("shows the first run with suggestions and no Around button without a solved image", () => {
    const markup = html(ResultsRegion, { state: base(), ctl, around: null });
    const region = tag(markup, "mast-results");
    expect(region).toContain('data-state="idle"');
    expect(region).toContain('data-view="gallery"');
    expect(count(markup, "mast-suggestion")).toBeGreaterThanOrEqual(3);
    expect(markup).toContain('data-query="M16"');
    expect(markup).not.toContain('data-testid="mast-around"');
    expect(markup).toContain("Explore the sky through Webb and Hubble");
  });

  it("adds Around <file> with its centre and radius when the selected file has a WCS", () => {
    const markup = html(ResultsRegion, { state: base(), ctl, around: { raDeg: 274.71, decDeg: -13.83, radiusArcmin: 4.2, label: "673nmos.fits" } });
    const around = tag(markup, "mast-around");
    expect(around).toContain('data-ra="274.71"');
    expect(around).toContain('data-dec="-13.83"');
    expect(around).toContain('data-radius="4.2"');
    expect(markup).toContain("Around 673nmos.fits");
  });

  it("renders one card per dataset with selection, thumbnail state and an accessible name", () => {
    const markup = html(ResultsRegion, { state: searched(), ctl, around: null });
    expect(tag(markup, "mast-results")).toContain('data-state="ready"');
    expect(count(markup, "mast-card")).toBe(2);
    const first = tag(markup, "mast-card", ' data-dataset="jw02739-o001_t001_nircam"');
    expect(first).toContain('aria-selected="true"');
    expect(first).toContain('data-thumb="pending"');
    expect(first).toContain('aria-label="M 16, JWST NIRCam, level 3, 6 filters, 2022-08-14, public"');
    expect(first).toContain('tabindex="0"');
    const second = tag(markup, "mast-card", ' data-dataset="jw02739-o002_t001_miri"');
    expect(second).toContain('aria-selected="false"');
    expect(second).toContain('tabindex="-1"');
    expect(markup).toContain("Exclusive until 2027-03-02");
    expect(markup).toContain('role="grid"');
    expect(markup).toContain('data-testid="mast-summary"');
  });

  it("shows a ready thumbnail as an asset image with empty alt text and a failed one with its cause", () => {
    const rows = [ROWS[0], row("jw02739-o003_t001_nircam")];
    let s = searched(base(), rows);
    s = run(
      s,
      { type: "thumbDemand", selected: [], visible: ["jw02739-o001_t001_nircam", "jw02739-o003_t001_nircam"], ahead: [] },
      { type: "thumbsStarted", keys: ["jw02739-o001_t001_nircam", "jw02739-o003_t001_nircam"], at: 0 },
      { type: "thumbReady", key: "jw02739-o001_t001_nircam", ref: { url: "http://asset.localhost/C%3A%5Ct.jpg", width: 128, height: 75, imageCount: 6, filename: "t_thumb.jpg" } },
      { type: "thumbFailed", key: "jw02739-o003_t001_nircam", error: { code: "mast_not_image", message: "response is not an image", context: { operation: "search/jwst/api/v0.1/get_image_thumbnail" } }, at: 900 },
    );
    const markup = html(ResultsRegion, { state: s, ctl, around: null });
    expect(tag(markup, "mast-card", ' data-dataset="jw02739-o001_t001_nircam"')).toContain('data-thumb="ready"');
    expect(markup).toMatch(/<img[^>]*src="http:\/\/asset\.localhost\/C%3A%5Ct\.jpg"[^>]*>/);
    expect(markup).toMatch(/<img[^>]*alt=""/);
    expect(tag(markup, "mast-card", ' data-dataset="jw02739-o003_t001_nircam"')).toContain('data-thumb="failed"');
    expect(tag(markup, "mast-card", ' data-dataset="jw02739-o003_t001_nircam"')).toContain('data-error-code="mast_not_image"');
    expect(markup).toContain(">MAST sent something that is not an image<");
    expect(markup).not.toContain("No preview");
    expect(markup).toMatch(/>6</);
  });

  describe("a thumbnail that failed shows MAST's answer where the image would be", () => {
    const A = "jw02739-o001_t001_nircam";
    const B = "jw02739-o003_t001_nircam";
    const unavailable: MastError = { code: "mast_server", message: "HTTP 503", context: { status: 503, detail: "Service Unavailable", operation: "search/jwst/api/v0.1/get_image_thumbnail" } };
    const slow: MastError = { code: "mast_timeout", message: "no answer within 30 s", context: { timeoutSec: 30, operation: "search/jwst/api/v0.1/get_image_thumbnail" } };

    function failedOn(error: MastError, waitedMs = 2000): ArchiveState {
      return run(
        searched(base(), [row(A), row(B)]),
        { type: "thumbDemand", selected: [], visible: [A, B], ahead: [] },
        { type: "thumbsStarted", keys: [A], at: 1000 },
        { type: "thumbFailed", key: A, error, at: 1000 + waitedMs },
      );
    }

    function card(markup: string, id: string): string {
      const start = markup.indexOf(`data-dataset="${id}"`);
      const open = markup.lastIndexOf("<div", start);
      const next = markup.indexOf('data-testid="mast-card"', start);
      return markup.slice(open, next < 0 ? undefined : markup.lastIndexOf("<div", next));
    }

    it("shows the title, the HTTP status, MAST's text and a Retry button on the card, never 'No preview'", () => {
      const markup = html(ResultsRegion, { state: failedOn(unavailable), ctl, around: null });
      const tile = card(markup, A);
      const open = tag(markup, "mast-card", ` data-dataset="${A}"`);
      expect(open).toContain('data-thumb="failed"');
      expect(open).toContain('data-error-code="mast_server"');
      expect(open).toContain('data-error-status="503"');
      expect(open).toContain('data-error-waited-ms="2000"');
      expect(tile).toContain(">MAST server error<");
      expect(tile).toContain(">HTTP 503<");
      expect(tile).toContain(">Service Unavailable<");
      expect(tile).toMatch(/<button[^>]*tabindex="-1"[^>]*>Retry/);
      expect(tile).not.toContain("No preview");
      const described = /aria-describedby="([^"]+)"/.exec(open)?.[1];
      expect(described).toBeTruthy();
      expect(tile).toContain(`id="${described}"`);
      expect(tile).toContain('data-tone="server"');
    });

    it("shows a timeout with the limit and the time the card waited", () => {
      const markup = html(ResultsRegion, { state: failedOn(slow, 41_000), ctl, around: null });
      const tile = card(markup, A);
      expect(tile).toContain(">MAST did not answer<");
      expect(tile).toContain("no answer within 30 s · waited 41 s");
      expect(tag(markup, "mast-card", ` data-dataset="${A}"`)).toContain('data-error-waited-ms="41000"');
      expect(tile).toContain('data-tone="transient"');
    });

    it("shows a mono badge with the status in the table cell, and the cause in its tooltip", () => {
      const s = run(failedOn(unavailable), { type: "viewSet", view: "table" }, { type: "tableThumbsToggled" });
      const markup = html(ResultsRegion, { state: s, ctl, around: null });
      const cell = /<div role="gridcell"[^>]*data-error-code="mast_server"[^>]*>[\s\S]*?<\/div>/.exec(markup)?.[0] ?? "";
      expect(cell).toContain('data-error-status="503"');
      expect(cell).toMatch(/>503</);
      expect(cell).toMatch(/title="[^"]*HTTP 503[^"]*Service Unavailable[^"]*"/);
      expect(cell).toContain("Press R to retry");
    });

    it("shows the queue paused by MAST with its countdown on the card, and WAIT in the table instead of a shimmer", () => {
      const now = Date.now();
      const paused = run(
        searched(base(), [row(A), row(B)]),
        { type: "thumbDemand", selected: [], visible: [A, B], ahead: [] },
        { type: "thumbsStarted", keys: [A], at: now },
        { type: "thumbRateLimited", key: A, at: now, retryAfterSec: 30, consecutive: 1, status: 429 },
      );
      const gallery = html(ResultsRegion, { state: paused, ctl, around: null });
      const tile = card(gallery, A);
      expect(tag(gallery, "mast-card", ` data-dataset="${A}"`)).toContain('data-thumb="paused"');
      expect(tile).toContain(">Paused by MAST<");
      expect(tile).toMatch(/HTTP 429 · resumes in (29|30) s/);
      const table = html(ResultsRegion, { state: run(paused, { type: "viewSet", view: "table" }, { type: "tableThumbsToggled" }), ctl, around: null });
      const cells = table.match(/<div role="gridcell" class="ab-mast-td ab-mast-td-thumb"[\s\S]*?<\/div>/g) ?? [];
      expect(cells).toHaveLength(2);
      for (const c of cells) {
        expect(c).toMatch(/>WAIT</);
        expect(c).not.toContain("ab-mast-shimmer");
      }
    });

    it("says Queued before a request starts and how long a slow one has been loading", () => {
      const s = run(searched(base(), [row(A), row(B)]), { type: "thumbDemand", selected: [], visible: [A, B], ahead: [] }, { type: "thumbsStarted", keys: [A], at: Date.now() - 12_400 });
      const markup = html(ResultsRegion, { state: s, ctl, around: null });
      expect(card(markup, A)).toMatch(/Loading · 1[23] s/);
      expect(card(markup, A)).toContain("ab-mast-shimmer");
      expect(card(markup, B)).toContain("Queued");
      expect(card(markup, B)).not.toContain("ab-mast-shimmer");
    });
  });

  it("shows a lock tile instead of a thumbnail for an exclusive row of a mission that takes no token", () => {
    const markup = html(ResultsRegion, { state: searched(), ctl, around: null });
    const locked = tag(markup, "mast-card", ' data-dataset="jw02739-o002_t001_miri"');
    expect(locked).toContain('data-thumb="locked"');
    expect(markup).toMatch(/ab-mast-thumb-locked[^>]*>.*EXCL\./);
    expect(tag(markup, "mast-card", ' data-dataset="jw02739-o001_t001_nircam"')).toContain('data-thumb="pending"');
  });

  it("mounts at most 200 cards for a 2,000-row result", () => {
    const many = Array.from({ length: 2000 }, (_, i) => row(`ds-${String(i).padStart(4, "0")}`));
    const markup = html(ResultsRegion, { state: searched(base(), many), ctl, around: null });
    expect(count(markup, "mast-card")).toBeGreaterThan(0);
    expect(count(markup, "mast-card")).toBeLessThanOrEqual(200);
    expect(markup).toContain('data-loaded="2000"');
  });

  it("renders the table view with rows, sortable headers and the row count", () => {
    const s = run(searched(), { type: "viewSet", view: "table" });
    const markup = html(ResultsRegion, { state: s, ctl, around: null });
    expect(tag(markup, "mast-results")).toContain('data-view="table"');
    expect(count(markup, "mast-row")).toBe(2);
    expect(tag(markup, "mast-row", ' data-dataset="jw02739-o001_t001_nircam"')).toContain('aria-selected="true"');
    expect(markup).toContain('aria-sort="none"');
    expect(markup).toContain("Rows 1-2 of 2");
    expect(markup).toContain("Thumbnails");
  });

  it("shows the empty state with real query edits", () => {
    const s = run(
      { ...base(), form: { ...base().form, text: "10.0 -80.0", radiusText: "1" } },
      { type: "searchStarted", generation: 1, text: "10.0 -80.0", kind: "coordinates", name: null, query: { ...QUERY, where: { kind: "position", raDeg: 10, decDeg: -80, radiusArcmin: 1, label: null } }, at: 0 },
      { type: "pageLoaded", generation: 1, page: page([], { radiusArcmin: 1, searchPosition: { raDeg: 10, decDeg: -80 } }) },
    );
    const markup = html(ResultsRegion, { state: s, ctl, around: null });
    expect(tag(markup, "mast-results")).toContain('data-state="empty"');
    expect(markup).toContain("No JWST datasets within 1&#x27; of 10.00000 -80.00000");
    expect(markup).toContain("Widen to 3&#x27;");
    expect(markup).toContain("Try on HST");
    expect(markup).toContain("All levels");
  });

  it("shows object chips with Remove, the filters in the title and where the class comes from", () => {
    const query: MastQuery = { ...QUERY, filters: { level: "combined", categories: ["galaxies"], objectTypes: ["planetary_nebulae"] } };
    const s = run(
      { ...base(), form: { ...base().form, text: "M16", filters: query.filters } },
      { type: "searchStarted", generation: 1, text: "M16", kind: "name", name: "M16", query, at: 0 },
      { type: "pageLoaded", generation: 1, page: page([]) },
    );
    const markup = html(ResultsRegion, { state: s, ctl, around: null });
    expect(markup).toContain("No JWST datasets within 3&#x27; of 274.68800 -13.79200 with Galaxies, Planetary nebulae");
    expect(markup).toContain('aria-label="Remove filter Galaxies"');
    expect(markup).toContain('aria-label="Remove filter Planetary nebulae"');
    expect(markup).toContain("Category and Object type come from what each proposal declared; a field can be filed under another class.");
    expect(markup).toContain(">Clear filters<");
  });

  it("shows the not-resolved state", () => {
    const s = run(
      { ...base(), form: { ...base().form, text: "Pilars of Creation" } },
      { type: "searchStarted", generation: 1, text: "Pilars of Creation", kind: "name", name: "Pilars of Creation", query: null, at: 0 },
      { type: "nameNotResolved", generation: 1, name: "Pilars of Creation", elapsedMs: 22_500 },
    );
    const markup = html(ResultsRegion, { state: s, ctl, around: null });
    expect(tag(markup, "mast-results")).toContain('data-state="not-resolved"');
    expect(markup).toContain("MAST name lookup found no object called &quot;Pilars of Creation&quot;");
    expect(markup).toContain("Edit query");
  });

  it("offers Search all levels next to Try again when a level search timed out", () => {
    const timeout: MastError = { code: "mast_timeout", message: "timed out", context: { timeoutSec: 120 } };
    const level = html(BlockingError, { error: timeout, missionLabel: "JWST", ctl, levelSearch: true });
    expect(level).toContain("MAST is sometimes slow with a level condition; All levels usually answers in a few seconds.");
    expect(tag(level, "mast-timeout-all-levels")).not.toBe("");
    expect(level.indexOf('data-testid="mast-try-again"')).toBeLessThan(level.indexOf('data-testid="mast-timeout-all-levels"'));
    expect(html(BlockingError, { error: timeout, missionLabel: "HST", ctl })).not.toContain("mast-timeout-all-levels");
    const network: MastError = { code: "mast_network", message: "refused", context: {} };
    expect(html(BlockingError, { error: network, missionLabel: "JWST", ctl, levelSearch: true })).not.toContain("mast-timeout-all-levels");
  });

  it("shows the blocking network error with Try again and the details", () => {
    const err: MastError = { code: "mast_network", message: "connection refused", context: { operation: "search/jwst/api/v0.1/search", host: "mast.stsci.edu" } };
    const s = run(
      { ...base(), form: { ...base().form, text: "M16" } },
      { type: "searchStarted", generation: 1, text: "M16", kind: "name", name: "M16", query: null, at: 0 },
      { type: "searchFailed", generation: 1, error: err, at: 0 },
    );
    const markup = html(ResultsRegion, { state: s, ctl, around: null });
    expect(tag(markup, "mast-results")).toContain('data-state="error"');
    expect(tag(markup, "mast-error")).toContain('data-code="mast_network"');
    expect(tag(markup, "mast-error")).toContain('data-kind="network"');
    expect(markup).toContain("Could not reach MAST (mast.stsci.edu)");
    expect(markup).toContain("The connection failed: connection refused. Your query is kept.");
    expect(markup).toContain('data-testid="mast-try-again"');
    expect(markup).toContain("Operation: search/jwst/api/v0.1/search");
  });

  it("keeps the previous results dimmed and inert while a new search runs", () => {
    const s = run(searched(), { type: "searchStarted", generation: 2, text: "M16", kind: "name", name: "M16", query: null, at: 0 });
    const markup = html(ResultsRegion, { state: s, ctl, around: null });
    expect(tag(markup, "mast-results")).toContain('data-state="resolving"');
    expect(markup).toContain("Resolving &quot;M16&quot; with MAST name lookup");
    expect(markup).toMatch(/<div[^>]*inert=""[^>]*>/);
    expect(count(markup, "mast-card")).toBe(2);
  });

  it("shows a skeleton row on the first search", () => {
    const s = run({ ...base(), form: { ...base().form, text: "2739" } }, { type: "searchStarted", generation: 1, text: "2739", kind: "programme", name: null, query: { ...QUERY, program: 2739 }, at: 0 });
    const markup = html(ResultsRegion, { state: s, ctl, around: null });
    expect(tag(markup, "mast-results")).toContain('data-state="searching"');
    expect(markup).toContain("Searching JWST programme 2739");
    expect(markup).toContain("ab-mast-skeleton");
  });

  it("puts a non-blocking banner above the results", () => {
    const err: MastError = { code: "mast_server", message: "bad gateway", context: { status: 504 } };
    const s = run(
      searched(),
      { type: "searchStarted", generation: 2, text: "M16", kind: "name", name: "M16", query: null, at: 0 },
      { type: "searchFailed", generation: 2, error: err, at: 0 },
    );
    const markup = html(ResultsRegion, { state: s, ctl, around: null });
    expect(tag(markup, "mast-banner")).toContain('data-kind="server"');
    expect(markup).toContain("HTTP 504");
  });
});

describe("categories in the results", () => {
  const CATEGORY: MastColumn = { name: "_category", label: "Category", kind: "text", sortable: false, unit: null };
  const filed = row("jw02739-o001_t001_nircam", { values: { ...row("x").values, _category: "Nebulae and ISM" }, targetDescription: "Molecular clouds; Nebulae; Protostars" });
  const unknown = row("jw02739-o002_t001_nircam", { level: "2bs, 3", levelClass: "exposure", values: { ...row("y").values, _category: null } });

  function filtered(filters: MastFilterValues, over: Parameters<typeof page>[1] = {}): ArchiveState {
    const query: MastQuery = { ...QUERY, filters };
    return run(
      { ...base(), form: { ...base().form, text: "M16", filters } },
      { type: "searchStarted", generation: 1, text: "M16", kind: "name", name: "M16", query, at: 0 },
      { type: "pageLoaded", generation: 1, page: page([filed, unknown], { columns: [...COLUMNS, CATEGORY], ...over }) },
    );
  }

  it("puts the category line on the card with the raw description as its title, and the L3 badge on 2bs, 3 rows", () => {
    const markup = html(ResultsRegion, { state: filtered({ level: "combined" }), ctl, around: null });
    expect(markup).toMatch(/<span class="ab-mast-card-cat" title="Molecular clouds; Nebulae; Protostars">Nebulae and ISM<\/span>/);
    expect(tag(markup, "mast-card", ' data-dataset="jw02739-o001_t001_nircam"')).toContain("category Nebulae and ISM");
    const second = tag(markup, "mast-card", ' data-dataset="jw02739-o002_t001_nircam"');
    expect(second).toContain("category unknown");
    expect(markup.split("ab-mast-card-cat").length - 1).toBe(1);
    expect(markup.split('class="ab-mast-badge">L3<').length - 1).toBe(2);
  });

  it("reads unknown in muted text in the table", () => {
    const s = run(filtered({ level: "combined" }), { type: "viewSet", view: "table" });
    const markup = html(ResultsRegion, { state: s, ctl, around: null });
    expect(markup).toMatch(/<span class="ab-mast-td-unknown">unknown<\/span>/);
    expect(markup).toContain(">Nebulae and ISM<");
  });

  it("explains unknown categories under the summary with the cached count and a way back", () => {
    const markup = html(SummaryLine, { state: filtered({ level: "all", categories: ["ism"] }, { unfilteredTotal: 1913 }), ctl });
    const note = tag(markup, "mast-summary-unknown");
    expect(note).not.toBe("");
    expect(markup).toContain("Showing 2 of 1,913 datasets from the same search without Category or Object type.");
    expect(tag(markup, "mast-summary-unknown-clear")).not.toBe("");
    expect(markup).toContain(">Remove object filters<");
    expect(html(SummaryLine, { state: filtered({ level: "all" }), ctl })).not.toContain("mast-summary-unknown");
  });

  it("gives the unknown note its whole text as a title, for a short window that cuts it to one line", () => {
    const markup = html(SummaryLine, { state: filtered({ level: "all", categories: ["ism"] }, { unfilteredTotal: 1913 }), ctl });
    expect(markup).toMatch(/<span class="ab-mast-summary-unknown-text" title="Showing 2 of 1,913 datasets from the same search without Category or Object type\. Datasets with no proposal category[^"]*never match\.">/);
  });
});

describe("banner, toast, live region and footer", () => {
  it("renders a banner with its retry action and a close button", () => {
    const markup = html(Banner, { banner: { kind: "rate-limited", text: "MAST is limiting requests (HTTP 429). Thumbnails are paused and resume in 42 s.", retry: "resume" }, ctl });
    expect(tag(markup, "mast-banner")).toContain('data-kind="rate-limited"');
    expect(markup).toContain("resume in 42 s");
    expect(markup).toContain("Resume now");
    expect(markup).toContain('aria-label="Dismiss"');
    const retry = html(Banner, { banner: { kind: "network", text: "Some images could not be loaded (network).", retry: "thumbs" }, ctl });
    expect(retry).toContain("Retry failed");
    expect(retry).not.toContain("mast-banner-all-levels");
  });

  it("offers Search all levels next to Try again on a timed-out level search banner", () => {
    const text = "MAST did not answer within 120 s. MAST is sometimes slow with a level condition; All levels usually answers in a few seconds.";
    const markup = html(Banner, { banner: { kind: "timeout", text, retry: "search", allLevels: true }, ctl });
    expect(markup).toContain("Try again");
    expect(tag(markup, "mast-banner-all-levels")).not.toBe("");
    expect(markup).toContain(">Search all levels<");
    expect(html(Banner, { banner: { kind: "timeout", text, retry: "search" }, ctl })).not.toContain("mast-banner-all-levels");
  });

  it("renders the toast and the polite live region", () => {
    expect(tag(html(ArchiveToast, { toast: { text: "Copied jw02739-o001_t001_nircam", seq: 1 }, ctl }), "mast-toast")).toBeTruthy();
    expect(html(ArchiveToast, { toast: null, ctl })).toBe("");
    const live = html(ArchiveLiveRegion, { live: { text: "Search finished: 2 datasets", seq: 3 } });
    expect(tag(live, "mast-live")).toContain('aria-live="polite"');
    expect(live).toContain("Search finished: 2 datasets");
  });

  it("shows the MAST activity pill, the Roman key status and the processing pill", () => {
    const busy = html(FooterStatusView, { total: 6, thumbs: 4, queued: 9, romanUnlocked: false, processing: { done: 2, total: 4 }, onRoman: vi.fn() });
    const pill = tag(busy, "mast-inflight");
    expect(pill).toContain('data-inflight="6"');
    expect(pill).toContain('data-inflight-thumbs="4"');
    expect(busy).toContain("MAST 6 in flight (4 thumbnails, 9 queued)");
    expect(tag(busy, "mast-roman-status")).toContain('data-state="locked"');
    expect(busy).toContain("Roman locked");
    expect(busy).toContain("Processing 2/4");
    const idle = html(FooterStatusView, { total: 0, thumbs: 0, queued: 0, romanUnlocked: true, processing: null, onRoman: vi.fn() });
    expect(idle).toContain("MAST idle");
    expect(tag(idle, "mast-roman-status")).toContain('data-state="unlocked"');
    expect(idle).not.toContain("Processing");
  });
});

describe("archive root", () => {
  it("fills its parent with the query bar, the results region, the live region and no detail pane yet", () => {
    const markup = html(ArchiveRoot, { active: true, workspaceFile: null, onSwitchToWorkspace: () => {} });
    expect(markup).toContain('data-testid="mast-query"');
    expect(markup).toContain('data-testid="mast-results"');
    expect(markup).toContain('data-testid="mast-live"');
    expect(markup).toContain('role="search"');
  });
});
