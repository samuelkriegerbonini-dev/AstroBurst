import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ResultsRegion from "../ResultsRegion";
import { archiveReducer, createInitialState, type ArchiveAction } from "../state/archiveReducer";
import type { ArchiveController } from "../state/archiveController";
import { MISSIONS, page, row } from "./fixtures";
import type { MastQuery } from "../../../shared/types/mast";

const QUERY: MastQuery = {
  mission: "jwst",
  where: { kind: "position", raDeg: 274.688, decDeg: -13.792, radiusArcmin: 3, label: "M 16" },
  program: null,
  datasetId: null,
  filters: {},
  preset: null,
  sort: [],
  offset: 0,
  limit: 200,
};

function stubController(): ArchiveController {
  const fns: Record<string | symbol, unknown> = {};
  return new Proxy(fns, { get: (target, prop) => (target[prop] ??= vi.fn()) }) as unknown as ArchiveController;
}

describe("results cut at 2,000 rows", () => {
  it("offers Narrow by date when the mission has a date filter", () => {
    const rows = Array.from({ length: 2000 }, (_, i) => row(`ds-${String(i).padStart(4, "0")}`));
    const actions: ArchiveAction[] = [
      { type: "missionsLoaded", missions: MISSIONS },
      { type: "searchStarted", generation: 1, text: "274.688 -13.792", kind: "coordinates", name: null, query: QUERY, at: 0 },
      { type: "pageLoaded", generation: 1, page: page(rows, { hasMore: true, totalReported: 5000 }) },
    ];
    const state = actions.reduce(archiveReducer, createInitialState());
    expect(state.results?.truncated).toBe(true);
    const html = renderToStaticMarkup(createElement(ResultsRegion, { state, ctl: stubController(), around: null }));
    expect(html).toContain("Showing the first 2,000 rows.");
    expect(html).toContain('data-testid="mast-narrow-date"');
    const noDate = { ...state, missions: state.missions.map((m) => (m.id === "jwst" ? { ...m, quickFilters: m.quickFilters.filter((f) => f.id !== "date") } : m)) };
    expect(renderToStaticMarkup(createElement(ResultsRegion, { state: noDate, ctl: stubController(), around: null }))).not.toContain("mast-narrow-date");
  });
});
