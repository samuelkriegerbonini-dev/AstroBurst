import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ResultsRegion from "../ResultsRegion";
import { archiveReducer, createInitialState, selectedRow, type ArchiveAction, type ArchiveState } from "../state/archiveReducer";
import { createArchiveController, type ArchiveController, type ArchiveServices } from "../state/archiveController";
import { createArchiveStore } from "../state/archiveStore";
import { createInflight } from "../state/inflight";
import { rovingRow, rowKeyMap, rowKeys } from "../logic/resultsView";
import { tableMove } from "../logic/gridNav";
import { MISSIONS, page, row } from "./fixtures";
import type { MastQuery, MastRow } from "../../../shared/types/mast";

const QUERY: MastQuery = {
  mission: "iue",
  where: { kind: "position", raDeg: 274.688, decDeg: -13.792, radiusArcmin: 3, label: null },
  program: null,
  datasetId: null,
  filters: {},
  sort: [],
  offset: 0,
  limit: 200,
};

function spectrum(datasetId: string, aperture: string): MastRow {
  return row(datasetId, { instrument: null, filter: null, filters: [], values: { iue_data_id: datasetId, iue_aper: aperture } });
}

const LWP15521_LARGE = spectrum("LWP15521", "LARGE");
const LWP15521_SMALL = spectrum("LWP15521", "SMALL");
const ROWS = [spectrum("SWP16973", "LARGE"), LWP15521_LARGE, LWP15521_SMALL, spectrum("LWP15526", "LARGE")];

function stubController(): ArchiveController {
  const fns: Record<string | symbol, unknown> = {};
  return new Proxy(fns, { get: (target, prop) => (target[prop] ??= vi.fn()) }) as unknown as ArchiveController;
}

function run(state: ArchiveState, ...actions: ArchiveAction[]): ArchiveState {
  return actions.reduce(archiveReducer, state);
}

function iueTable(rows: MastRow[], selected: string, rowKey: string | null = null): ArchiveState {
  const start = run(createInitialState(), { type: "missionsLoaded", missions: MISSIONS }, { type: "tokenStatus", status: { present: false, authorized: null } });
  return run(
    { ...start, form: { ...start.form, mission: "iue", text: "274.688 -13.792" } },
    { type: "searchStarted", generation: 1, text: "274.688 -13.792", kind: "coordinates", name: null, query: QUERY, at: 0 },
    { type: "pageLoaded", generation: 1, page: page(rows, { mission: "iue" }) },
    { type: "viewSet", view: "table" },
    { type: "selected", datasetId: selected, rowKey },
  );
}

function jwstGallery(rows: MastRow[], selected: string, rowKey: string | null = null): ArchiveState {
  const start = run(createInitialState(), { type: "missionsLoaded", missions: MISSIONS }, { type: "tokenStatus", status: { present: false, authorized: null } });
  return run(
    { ...start, form: { ...start.form, text: "274.688 -13.792" } },
    { type: "searchStarted", generation: 1, text: "274.688 -13.792", kind: "coordinates", name: null, query: { ...QUERY, mission: "jwst" }, at: 0 },
    { type: "pageLoaded", generation: 1, page: page(rows) },
    { type: "selected", datasetId: selected, rowKey },
  );
}

function idleController(store: ReturnType<typeof createArchiveStore>): ArchiveController {
  const services = new Proxy({}, { get: () => vi.fn(() => new Promise(() => {})) }) as unknown as ArchiveServices;
  return createArchiveController({ store, services, clock: { now: () => 0, setTimeout: () => 0, clearTimeout: () => {} }, storage: null, inflight: createInflight() });
}

function cardTags(markup: string): string[] {
  return markup.match(/<div[^>]*data-testid="mast-card"[^>]*>/g) ?? [];
}

function rowTags(markup: string): string[] {
  return markup.match(/<div[^>]*data-testid="mast-row"[^>]*>/g) ?? [];
}

const DOWN = { key: "ArrowDown", ctrlKey: false, metaKey: false, shiftKey: false, altKey: false };

describe("results whose dataset id repeats (IUE lists one row per aperture)", () => {
  it("gives every row a distinct React key, so a later result never inherits orphaned rows", () => {
    expect(rowKeys(ROWS)).toEqual(["SWP16973", "LWP15521", "LWP15521#2", "LWP15526"]);
    const tricky = [row("A"), row("A"), row("A#2"), row("A")];
    const keys = rowKeys(tricky);
    expect(new Set(keys).size).toBe(tricky.length);
    const unique = [row("jw02739-o001_t001_nircam"), row("jw02739-o002_t001_miri")];
    expect(rowKeys(unique)).toEqual(unique.map((r) => r.datasetId));
  });

  it("keeps the keyboard on the row the user is on, so ArrowDown walks through both rows of a repeated dataset", () => {
    const keyOf = rowKeyMap(ROWS);
    expect(rowKeyMap(ROWS)).toBe(keyOf);
    expect(keyOf.get(LWP15521_SMALL)).toBe("LWP15521#2");
    expect(rovingRow(ROWS, keyOf, "LWP15521", "LWP15521#2")).toBe(2);
    expect(rovingRow(ROWS, keyOf, "LWP15521", null)).toBe(1);
    expect(rovingRow(ROWS, keyOf, "LWP15521", "LWP15526")).toBe(1);
    expect(rovingRow(ROWS, keyOf, null, "LWP15521#2")).toBe(0);
    expect(rovingRow(ROWS, keyOf, "LWP99999", null)).toBe(0);
    const first = tableMove(rovingRow(ROWS, keyOf, "LWP15521", "LWP15521"), DOWN, { count: ROWS.length, pageRows: 10 });
    expect(first?.index).toBe(2);
    const target = ROWS[first!.index];
    const next = tableMove(rovingRow(ROWS, keyOf, target.datasetId, keyOf.get(target) ?? null), DOWN, { count: ROWS.length, pageRows: 10 });
    expect(next?.index).toBe(3);
  });

  it("shows the row the user picked in the detail, not the first row of its dataset", () => {
    const small = iueTable(ROWS, "LWP15521", "LWP15521#2");
    expect(selectedRow(small)?.values.iue_aper).toBe("SMALL");
    expect(selectedRow(iueTable(ROWS, "LWP15521"))?.values.iue_aper).toBe("LARGE");
    const back = run(small, { type: "selected", datasetId: "LWP15521", rowKey: "LWP15521" });
    expect(selectedRow(back)?.values.iue_aper).toBe("LARGE");
    const tags = rowTags(renderToStaticMarkup(createElement(ResultsRegion, { state: small, ctl: stubController(), around: null })));
    const marked = tags.filter((t) => t.includes('aria-selected="true"'));
    expect(marked).toHaveLength(1);
    expect(marked[0]).toContain('aria-rowindex="4"');
  });

  it("keeps the detail state when the other row of the same dataset is picked, and resets it for another dataset", () => {
    const s = run(iueTable(ROWS, "LWP15521"), { type: "detailTab", tab: "metadata" }, { type: "groupSelected", group: "x" });
    const sameDataset = run(s, { type: "selected", datasetId: "LWP15521", rowKey: "LWP15521#2" });
    expect(sameDataset.selectedKey).toBe("LWP15521#2");
    expect(sameDataset.detail.group).toBe("x");
    const other = run(sameDataset, { type: "selected", datasetId: "LWP15526", rowKey: "LWP15526" });
    expect(other.detail.group).toBeNull();
    expect(other.selectedKey).toBe("LWP15526");
  });

  it("selects through the controller with the row key it was given", () => {
    const store = createArchiveStore(iueTable(ROWS, "SWP16973"));
    const ctl = idleController(store);
    ctl.select("LWP15521", "LWP15521#2");
    expect(store.getState().selectedKey).toBe("LWP15521#2");
    expect(selectedRow(store.getState())?.values.iue_aper).toBe("SMALL");
    ctl.dispose();
  });
  it("marks one row selected and one row in the tab order when the selected dataset has two rows", () => {
    const markup = renderToStaticMarkup(createElement(ResultsRegion, { state: iueTable(ROWS, "LWP15521"), ctl: stubController(), around: null }));
    const tags = rowTags(markup);
    expect(tags).toHaveLength(4);
    const selected = tags.filter((t) => t.includes('aria-selected="true"'));
    expect(selected).toHaveLength(1);
    expect(selected[0]).toContain('aria-rowindex="3"');
    expect(tags.filter((t) => t.includes('tabindex="0"'))).toHaveLength(1);
  });
});

describe("gallery cards whose dataset id repeats", () => {
  const A = row("jw02739-o001_t001_nircam");
  const A2 = row("jw02739-o001_t001_nircam", { values: { ...A.values, effexptm: 1.5 } });
  const B = row("jw02739-o002_t001_miri");

  afterEach(() => vi.restoreAllMocks());

  it("gives every card its own key and marks only the picked card selected", () => {
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    const markup = renderToStaticMarkup(createElement(ResultsRegion, { state: jwstGallery([A, A2, B], A.datasetId, `${A.datasetId}#2`), ctl: stubController(), around: null }));
    const cards = cardTags(markup);
    expect(cards).toHaveLength(3);
    expect(cards.map((c) => /data-row-key="([^"]+)"/.exec(c)?.[1])).toEqual([A.datasetId, `${A.datasetId}#2`, B.datasetId]);
    const marked = cards.filter((c) => c.includes('aria-selected="true"'));
    expect(marked).toHaveLength(1);
    expect(marked[0]).toContain(`data-row-key="${A.datasetId}#2"`);
    expect(cards.filter((c) => c.includes('tabindex="0"'))).toHaveLength(1);
    expect(errors.mock.calls.some((c) => String(c[0]).includes("same key"))).toBe(false);
  });

  it("steps the Large view to the next dataset, never to the other row of the same one", () => {
    const store = createArchiveStore(jwstGallery([A, A2, B], A.datasetId));
    const ctl = idleController(store);
    ctl.openLarge(A.datasetId);
    ctl.largeStepDataset(1);
    expect(store.getState().large.datasetId).toBe(B.datasetId);
    ctl.largeStepDataset(-1);
    expect(store.getState().large.datasetId).toBe(A.datasetId);
    ctl.dispose();
  });

});
