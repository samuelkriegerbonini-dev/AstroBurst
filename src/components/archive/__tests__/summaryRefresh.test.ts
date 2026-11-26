import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import SummaryLine from "../SummaryLine";
import { archiveReducer, createInitialState, hasCountdown, type ArchiveAction, type ArchiveState } from "../state/archiveReducer";
import { elementsOf } from "../../../test/elementTree";
import { MISSIONS, page, row } from "./fixtures";
import type { MastError, MastQuery, MastResolved } from "../../../shared/types/mast";

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
const M16: MastResolved = { query: "M16", canonicalName: "M 16", resolver: "SIMBAD", raDeg: 274.688, decDeg: -13.792, objectType: null, elapsedMs: 870, fromCache: false };
const NET: MastError = { code: "mast_network", message: "connection refused", context: { host: "mast.stsci.edu" } };

function searched(): ArchiveState {
  const actions: ArchiveAction[] = [
    { type: "missionsLoaded", missions: MISSIONS },
    { type: "searchStarted", generation: 1, text: "M16", kind: "name", name: "M16", query: null, at: 0 },
    { type: "nameResolved", generation: 1, resolved: M16, query: QUERY, at: 900 },
    { type: "pageLoaded", generation: 1, page: page([row("jw02739-o001_t001_nircam")]) },
  ];
  return actions.reduce(archiveReducer, createInitialState());
}

describe("summary line", () => {
  it("refreshes the results shown through the controller, whatever the field holds", () => {
    const ctl = { refresh: vi.fn(async () => {}), clearObjectFilters: vi.fn(async () => {}) };
    const state = { ...searched(), form: { ...searched().form, text: "M1" } };
    const tree = SummaryLine({ state, ctl });
    const button = elementsOf<{ "aria-label"?: string; onClick?: () => void }>(tree).find((e) => e.props["aria-label"] === "Refresh");
    button?.props.onClick?.();
    expect(ctl.refresh).toHaveBeenCalledTimes(1);
  });

  it("marks the results as previous after a newer search failed", () => {
    const failed = [
      { type: "searchStarted", generation: 2, text: "NGC 628", kind: "name", name: "NGC 628", query: null, at: 1 },
      { type: "searchFailed", generation: 2, error: NET, at: 2 },
    ] as ArchiveAction[];
    const state = failed.reduce(archiveReducer, searched());
    const html = renderToStaticMarkup(SummaryLine({ state, ctl: { refresh: vi.fn(), clearObjectFilters: vi.fn() } }) as never);
    expect(html).toContain('data-stale="true"');
    expect(html).toContain("Previous results: ");
  });
});

describe("countdown timer of the results region", () => {
  it("stops once the rate-limit banner has expired", () => {
    const limited = [
      { type: "pageRequested", generation: 1, offset: 200 },
      { type: "pageFailed", generation: 1, error: { code: "mast_rate_limited", message: "429", context: { retryAfterSec: 10, status: 429 } }, at: 1_000 },
    ] as ArchiveAction[];
    const state = limited.reduce(archiveReducer, searched());
    expect(hasCountdown(state, 5_000)).toBe(true);
    expect(hasCountdown(state, 11_001)).toBe(false);
  });
});
