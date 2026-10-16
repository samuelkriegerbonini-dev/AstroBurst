import { describe, expect, it } from "vitest";
import {
  archiveReducer,
  createInitialState,
  detailEntry,
  detailMode,
  detailShown,
  detailTabOf,
  effectiveView,
  largeEntry,
  resultsState,
  romanPanelState,
  selectedRow,
  unavailableInfo,
  visibleBanner,
  type ArchiveAction,
  type ArchiveState,
} from "../state/archiveReducer";
import { MISSIONS, hstSet, jwstL3Set, page, productList, row } from "./fixtures";
import type { MastError, MastQuery, MastResolved } from "../../../shared/types/mast";

const QUERY: MastQuery = {
  mission: "jwst",
  where: { kind: "position", raDeg: 274.688, decDeg: -13.792, radiusArcmin: 3, label: "M 16" },
  program: null,
  datasetId: null,
  filters: { level: "combined" },
  sort: [],
  offset: 0,
  limit: 200,
};

const M16: MastResolved = { query: "M16", canonicalName: "M 16", resolver: "SIMBAD", raDeg: 274.688, decDeg: -13.792, objectType: null, elapsedMs: 870, fromCache: false };

function run(state: ArchiveState, ...actions: ArchiveAction[]): ArchiveState {
  return actions.reduce(archiveReducer, state);
}

function base(): ArchiveState {
  return run(createInitialState(), { type: "missionsLoaded", missions: MISSIONS });
}

const ROWS = [row("jw02739-o001_t001_nircam"), row("jw02739-o002_t001_miri", { instrument: "MIRI", filters: ["F770W", "F1130W"] })];

function searched(state = base()): ArchiveState {
  return run(
    state,
    { type: "searchStarted", generation: 1, text: "M16", kind: "name", name: "M16", query: null, at: 0 },
    { type: "nameResolved", generation: 1, resolved: M16, query: QUERY, at: 900 },
    { type: "pageLoaded", generation: 1, page: page(ROWS) },
  );
}

const netErr: MastError = { code: "mast_network", message: "connection refused", context: { operation: "search/jwst/api/v0.1/search" } };

describe("search states", () => {
  it("starts idle on JWST with nothing selected", () => {
    const s = base();
    expect(resultsState(s)).toBe("idle");
    expect(s.form.mission).toBe("jwst");
    expect(s.selectedId).toBeNull();
  });

  it("goes resolving, searching, ready and announces the result", () => {
    let s = run(base(), { type: "searchStarted", generation: 1, text: "M16", kind: "name", name: "M16", query: null, at: 0 });
    expect(resultsState(s)).toBe("resolving");
    s = run(s, { type: "nameResolved", generation: 1, resolved: M16, query: QUERY, at: 900 });
    expect(resultsState(s)).toBe("searching");
    expect(s.run?.resolved).toEqual(M16);
    s = run(s, { type: "pageLoaded", generation: 1, page: page(ROWS) });
    expect(resultsState(s)).toBe("ready");
    expect(s.live.text).toBe("Search finished: 2 datasets");
    expect(s.resolved).toEqual(M16);
  });

  it("selects the first result automatically", () => {
    expect(searched().selectedId).toBe("jw02739-o001_t001_nircam");
  });

  it("skips the lookup line for coordinates, programmes and dataset ids", () => {
    const s = run(base(), { type: "searchStarted", generation: 1, text: "2739", kind: "programme", name: null, query: { ...QUERY, where: { kind: "none" }, program: 2739 }, at: 0 });
    expect(resultsState(s)).toBe("searching");
  });

  it("shows the empty state for zero rows and announces it", () => {
    const s = run(
      base(),
      { type: "searchStarted", generation: 1, text: "10 -80", kind: "coordinates", name: null, query: QUERY, at: 0 },
      { type: "pageLoaded", generation: 1, page: page([], { totalReported: 0 }) },
    );
    expect(resultsState(s)).toBe("empty");
    expect(s.live.text).toBe("No results");
  });

  it("shows not-resolved without searching and announces it", () => {
    const s = run(
      searched(),
      { type: "searchStarted", generation: 2, text: "XQZ", kind: "name", name: "XQZ", query: null, at: 0 },
      { type: "nameNotResolved", generation: 2, name: "XQZ", elapsedMs: 22500 },
    );
    expect(resultsState(s)).toBe("not-resolved");
    expect(s.notResolved).toEqual({ name: "XQZ", elapsedMs: 22500 });
    expect(s.results).toBeNull();
    expect(s.live.text).toBe("Name not found");
  });

  it("blocks with an error when nothing is on screen", () => {
    const s = run(
      base(),
      { type: "searchStarted", generation: 1, text: "M16", kind: "name", name: "M16", query: null, at: 0 },
      { type: "searchFailed", generation: 1, error: netErr, at: 10 },
    );
    expect(resultsState(s)).toBe("error");
    expect(s.error).toEqual(netErr);
    expect(s.form.text).toBe("");
  });

  it("keeps the results and raises a banner when a later search fails", () => {
    const s = run(
      searched(),
      { type: "searchStarted", generation: 2, text: "M17", kind: "name", name: "M17", query: null, at: 0 },
      { type: "searchFailed", generation: 2, error: { code: "mast_server", message: "x", context: { status: 504 } }, at: 10 },
    );
    expect(resultsState(s)).toBe("ready");
    expect(s.results?.rows).toHaveLength(2);
    expect(visibleBanner(s, 10)?.kind).toBe("server");
  });

  it("puts a rejected query inline and leaves the region as it was", () => {
    const s = run(
      base(),
      { type: "searchStarted", generation: 1, text: "M16", kind: "name", name: "M16", query: null, at: 0 },
      { type: "searchFailed", generation: 1, error: { code: "mast_rejected", message: "400", context: { detail: "Radius greater than 30 arcminutes is not supported" } }, at: 1 },
    );
    expect(resultsState(s)).toBe("idle");
    expect(s.queryError).toEqual({ text: "MAST rejected the query: Radius greater than 30 arcminutes is not supported.", field: "radius" });
  });

  it("ignores a cancelled search and restores the previous results", () => {
    const before = searched();
    const s = run(
      before,
      { type: "searchStarted", generation: 2, text: "M17", kind: "name", name: "M17", query: null, at: 0 },
      { type: "searchCancelled", generation: 2 },
    );
    expect(resultsState(s)).toBe("ready");
    expect(s.results).toBe(before.results);
    expect(s.run).toBeNull();
    const silent = run(before, { type: "searchStarted", generation: 2, text: "M17", kind: "name", name: "M17", query: null, at: 0 }, { type: "searchFailed", generation: 2, error: { code: "mast_cancelled", message: "", context: {} }, at: 1 });
    expect(silent.run).toBeNull();
    expect(visibleBanner(silent, 1)).toBeNull();
  });

  it("ignores answers from an older generation", () => {
    const s = run(
      base(),
      { type: "searchStarted", generation: 2, text: "M16", kind: "name", name: "M16", query: null, at: 0 },
      { type: "pageLoaded", generation: 1, page: page(ROWS) },
    );
    expect(resultsState(s)).toBe("resolving");
  });

  it("blocks Search with a local hint", () => {
    const s = run(base(), { type: "queryBlocked", field: "query", message: "Add a target, coordinates, programme or dataset id" });
    expect(s.queryError).toEqual({ text: "Add a target, coordinates, programme or dataset id", field: "query" });
    expect(run(s, { type: "formChanged", patch: { text: "M" } }).queryError).toBeNull();
  });

  it("remembers recent searches for the session, newest first and without repeats", () => {
    let s = base();
    for (const text of ["M16", "2739", "M16"]) s = run(s, { type: "searchStarted", generation: 1, text, kind: "name", name: text, query: null, at: 0 });
    expect(s.recent).toEqual(["M16", "2739"]);
  });
});

describe("paging", () => {
  it("appends the next page in the same generation", () => {
    const first = run(base(), { type: "searchStarted", generation: 1, text: "M16", kind: "coordinates", name: null, query: QUERY, at: 0 }, {
      type: "pageLoaded",
      generation: 1,
      page: page(Array.from({ length: 200 }, (_, i) => row(`d${i}`)), { hasMore: true, totalReported: 300 }),
    });
    const loading = run(first, { type: "pageRequested", generation: 1, offset: 200 });
    expect(loading.loadingMore).toBe(200);
    expect(loading.generation).toBe(first.generation);
    const done = run(loading, { type: "pageLoaded", generation: 1, page: page(Array.from({ length: 100 }, (_, i) => row(`e${i}`)), { offset: 200, totalReported: 300 }) });
    expect(done.results?.rows).toHaveLength(300);
    expect(done.loadingMore).toBeNull();
    expect(done.selectedId).toBe("d0");
  });
});

describe("views and selection", () => {
  it("keeps the selection across the gallery and table toggle", () => {
    let s = run(searched(), { type: "selected", datasetId: "jw02739-o002_t001_miri" });
    s = run(s, { type: "viewToggled" });
    expect(effectiveView(s)).toBe("table");
    expect(s.selectedId).toBe("jw02739-o002_t001_miri");
    s = run(s, { type: "viewToggled" });
    expect(effectiveView(s)).toBe("gallery");
    expect(s.selectedId).toBe("jw02739-o002_t001_miri");
  });

  it("forces the table on a mission without images", () => {
    const s = run(base(), { type: "missionSwitched", form: { ...base().form, mission: "iue", filters: {} }, generation: 1 });
    expect(effectiveView(s)).toBe("table");
    expect(resultsState(s)).toBe("no-images");
  });

  it("clears results, selection and filters' results on a mission switch and bumps the generation", () => {
    const s = run(searched(), { type: "missionSwitched", form: { ...base().form, mission: "hst", filters: {} }, generation: 2 });
    expect(s.form.mission).toBe("hst");
    expect(s.generation).toBe(2);
    expect(s.results).toBeNull();
    expect(s.selectedId).toBeNull();
    expect(resultsState(s)).toBe("idle");
  });

  it("locks Roman and marks undeployed missions unavailable", () => {
    expect(resultsState(run(base(), { type: "missionSwitched", form: { ...base().form, mission: "roman", filters: {} }, generation: 1 }))).toBe("locked");
    expect(resultsState(run(base(), { type: "missionSwitched", form: { ...base().form, mission: "tess", filters: {} }, generation: 1 }))).toBe("unavailable");
  });

  it("opens Roman results once access is confirmed", () => {
    const s = run(
      base(),
      { type: "tokenStatus", status: { present: true, authorized: true } },
      { type: "missionSwitched", form: { ...base().form, mission: "roman", filters: {} }, generation: 1 },
    );
    expect(resultsState(s)).toBe("idle");
  });
});

describe("detail images", () => {
  it("defaults to the representative entry and follows group and image choices", () => {
    let s = run(searched(), { type: "imageListLoaded", key: "jwst:jw02739-o001_t001_nircam", set: jwstL3Set() });
    expect(detailEntry(s)?.filter).toBe("F187N");
    s = run(s, { type: "imageSelected", imageKey: jwstL3Set().entries[3].key });
    expect(detailEntry(s)?.filter).toBe("F335M");
    s = run(s, { type: "selected", datasetId: "jw02739-o002_t001_miri" });
    expect(s.detail.imageKey).toBeNull();
  });

  it("cycles product groups with G", () => {
    let s = run(searched(), { type: "imageListLoaded", key: "jwst:jw02739-o001_t001_nircam", set: hstSet("jw02739-o001_t001_nircam") });
    expect(detailEntry(s)?.group).toBe("combined");
    s = run(s, { type: "groupCycled" });
    expect(detailEntry(s)?.group).toBe("calibrated");
    s = run(s, { type: "groupCycled" }, { type: "groupCycled" }, { type: "groupCycled" });
    expect(detailEntry(s)?.group).toBe("combined");
  });
});

describe("Large view", () => {
  it("walks datasets and keeps the group and filter when they exist", () => {
    const l3 = jwstL3Set();
    const miri = { ...jwstL3Set("jw02739-o002_t001_miri"), representative: 0 };
    miri.entries = miri.entries.slice(0, 2).map((e, i) => ({ ...e, key: `miri-${i}`, label: i === 0 ? "F770W" : "F1130W", filter: i === 0 ? "F770W" : "F1130W" }));
    let s = run(
      searched(),
      { type: "imageListLoaded", key: "jwst:jw02739-o001_t001_nircam", set: l3 },
      { type: "largeOpened", datasetId: "jw02739-o001_t001_nircam", imageKey: l3.entries[1].key },
    );
    expect(largeEntry(s)?.filter).toBe("F187N");
    s = run(s, { type: "largeImage", imageKey: l3.entries[2].key });
    expect(largeEntry(s)?.filter).toBe("F200W");
    s = run(s, { type: "largeDataset", datasetId: "jw02739-o002_t001_miri" });
    expect(s.large.datasetId).toBe("jw02739-o002_t001_miri");
    expect(s.selectedId).toBe("jw02739-o002_t001_miri");
    expect(largeEntry(s)).toBeNull();
    s = run(s, { type: "imageListLoaded", key: "jwst:jw02739-o002_t001_miri", set: miri });
    expect(largeEntry(s)?.group).toBe("i2d");
    expect(largeEntry(s)?.label).toBe("F770W");
    s = run(s, { type: "largeClosed" });
    expect(s.large.open).toBe(false);
    expect(s.selectedId).toBe("jw02739-o002_t001_miri");
  });

  it("drops the detail image choice when it opens on another dataset", () => {
    const s = run(
      searched(),
      { type: "imageListLoaded", key: "jwst:jw02739-o001_t001_nircam", set: hstSet("jw02739-o001_t001_nircam") },
      { type: "groupSelected", group: "raw" },
      { type: "largeOpened", datasetId: "jw02739-o002_t001_miri", imageKey: null },
    );
    expect(s.selectedId).toBe("jw02739-o002_t001_miri");
    expect(s.detail.group).toBeNull();
    expect(s.detail.imageKey).toBeNull();
  });

  it("keeps the same filter when the next dataset has it", () => {
    const a = jwstL3Set("jw02739-o001_t001_nircam");
    const b = jwstL3Set("jw02739-o002_t001_miri");
    const s = run(
      searched(),
      { type: "imageListLoaded", key: "jwst:jw02739-o001_t001_nircam", set: a },
      { type: "imageListLoaded", key: "jwst:jw02739-o002_t001_miri", set: b },
      { type: "largeOpened", datasetId: "jw02739-o001_t001_nircam", imageKey: a.entries[4].key },
      { type: "largeDataset", datasetId: "jw02739-o002_t001_miri" },
    );
    expect(largeEntry(s)?.key).toBe(b.entries[4].key);
  });
});

describe("Roman token panel", () => {
  const roman = (s: ArchiveState) => run(s, { type: "missionSwitched", form: { ...s.form, mission: "roman", filters: {} }, generation: s.generation + 1 });

  it("shows none without a token", () => {
    expect(romanPanelState(run(roman(base()), { type: "tokenStatus", status: { present: false, authorized: null } }))).toBe("none");
  });

  it("walks checking, then authorized, and announces it", () => {
    let s = run(roman(base()), { type: "tokenCheckStarted", step: "info" });
    expect(romanPanelState(s)).toBe("checking");
    s = run(s, { type: "tokenStep", step: "auth" });
    expect(s.token.step).toBe("auth");
    s = run(s, { type: "tokenSaved", status: { present: true, authorized: true }, at: 5 });
    expect(romanPanelState(s)).toBe("authorized");
    expect(s.live.text).toBe("Roman access confirmed.");
  });

  it("shows not-authorized when MAST accepts the token without Roman access", () => {
    const s = run(roman(base()), { type: "tokenCheckStarted", step: "info" }, { type: "tokenSaved", status: { present: true, authorized: false }, at: 5 });
    expect(romanPanelState(s)).toBe("not-authorized");
  });

  it("shows rejected with nothing saved", () => {
    const s = run(roman(base()), { type: "tokenCheckStarted", step: "info" }, {
      type: "tokenSaveFailed",
      error: { code: "mast_token_rejected", message: "anon", context: {} },
      status: { present: false, authorized: null },
    });
    expect(romanPanelState(s)).toBe("rejected");
    expect(s.token.message).toBe("MAST did not accept this token (mistyped, expired or revoked). Nothing was saved.");
  });

  it("shows unreachable with the right copy for /info and for the Roman check", () => {
    const info = run(roman(base()), { type: "tokenCheckStarted", step: "info" }, { type: "tokenSaveFailed", error: netErr, status: { present: false, authorized: null } });
    expect(romanPanelState(info)).toBe("unreachable");
    expect(info.token.status).toEqual({ present: false, authorized: null });
    expect(info.token.message).toBe("Could not reach MAST to check the token. Nothing was saved.");
    const auth = run(roman(base()), { type: "tokenCheckStarted", step: "info" }, { type: "tokenSaveFailed", error: netErr, status: { present: true, authorized: null } });
    expect(romanPanelState(auth)).toBe("unreachable");
    expect(auth.token.message).toBe("Token saved. MAST could not be reached to check Roman access.");
  });

  it("shows expired after a rejected recheck or a Roman search answering token rejected", () => {
    const authorized = run(roman(base()), { type: "tokenStatus", status: { present: true, authorized: true } });
    const viaCheck = run(authorized, { type: "tokenCheckStarted", step: "info" }, { type: "tokenCheckFailed", error: { code: "mast_token_rejected", message: "", context: {} } });
    expect(romanPanelState(viaCheck)).toBe("expired");
    const viaSearch = run(
      authorized,
      { type: "searchStarted", generation: 9, text: "M16", kind: "coordinates", name: null, query: { ...QUERY, mission: "roman" }, at: 0 },
      { type: "searchFailed", generation: 9, error: { code: "mast_token_rejected", message: "", context: {} }, at: 1 },
    );
    expect(romanPanelState(viaSearch)).toBe("expired");
    expect(resultsState(viaSearch)).toBe("locked");
  });

  it("marks Roman not authorized after a Roman 401", () => {
    const s = run(
      roman(run(base(), { type: "tokenStatus", status: { present: true, authorized: true } })),
      { type: "searchStarted", generation: 9, text: "M16", kind: "coordinates", name: null, query: { ...QUERY, mission: "roman" }, at: 0 },
      { type: "searchFailed", generation: 9, error: { code: "mast_unauthorized", message: "", context: { mission: "roman", status: 401 } }, at: 1 },
    );
    expect(romanPanelState(s)).toBe("not-authorized");
  });

  it("goes back to none after Remove", () => {
    const s = run(roman(base()), { type: "tokenStatus", status: { present: true, authorized: false } }, { type: "tokenCleared" });
    expect(romanPanelState(s)).toBe("none");
  });
});

describe("Roman token panel focus, replace and manage", () => {
  const roman = (s: ArchiveState) => run(s, { type: "missionSwitched", form: { ...s.form, mission: "roman", filters: {} }, generation: s.generation + 1 });
  const lastFocus = (s: ArchiveState) => s.token.focus?.target ?? null;

  it("asks for focus on the outcome after a save or a check, and on the field after a rejected token", () => {
    const saved = run(roman(base()), { type: "tokenCheckStarted", step: "info" }, { type: "tokenSaved", status: { present: true, authorized: false }, at: 5 });
    expect(lastFocus(saved)).toBe("outcome");
    const rejected = run(roman(base()), { type: "tokenCheckStarted", step: "info" }, {
      type: "tokenSaveFailed",
      error: { code: "mast_token_rejected", message: "anon", context: {} },
      status: { present: false, authorized: null },
    });
    expect(lastFocus(rejected)).toBe("field");
    const badFormat = run(roman(base()), {
      type: "tokenSaveFailed",
      error: { code: "mast_bad_input", message: "This is too short for a MAST token (at least 16 characters).", context: {} },
      status: { present: false, authorized: null },
    });
    expect(romanPanelState(badFormat)).toBe("rejected");
    expect(badFormat.token.message).toBe("This is too short for a MAST token (at least 16 characters).");
    expect(lastFocus(badFormat)).toBe("field");
    const checked = run(roman(base()), { type: "tokenCheckStarted", step: "info" }, { type: "tokenChecked", status: { present: true, authorized: true }, at: 7 });
    expect(romanPanelState(checked)).toBe("authorized");
    expect(checked.token.checkedAt).toBe(7);
    expect(checked.live.text).toBe("Roman access confirmed.");
    expect(lastFocus(checked)).toBe("outcome");
  });

  it("clears a focus request once it is done, and only that one", () => {
    const s = run(roman(base()), { type: "tokenSaved", status: { present: true, authorized: false }, at: 5 });
    const seq = s.token.focus?.seq ?? -1;
    expect(run(s, { type: "tokenFocusDone", seq: seq + 1 }).token.focus).not.toBeNull();
    expect(run(s, { type: "tokenFocusDone", seq }).token.focus).toBeNull();
  });

  it("uses the saved flag when the controller knows which check failed", () => {
    const replaced = run(roman(run(base(), { type: "tokenStatus", status: { present: true, authorized: true } })), { type: "tokenCheckStarted", step: "info" }, {
      type: "tokenSaveFailed",
      error: netErr,
      status: { present: true, authorized: true },
      saved: false,
    });
    expect(replaced.token.message).toBe("Could not reach MAST to check the token. Nothing was saved.");
  });

  it("opens and closes the replace form and the management view", () => {
    let s = run(roman(base()), { type: "tokenStatus", status: { present: true, authorized: true } });
    s = run(s, { type: "tokenPanel", open: true });
    expect(s.token.expanded).toBe(true);
    expect(lastFocus(s)).toBe("panel");
    s = run(s, { type: "tokenReplace", open: true });
    expect(s.token.replacing).toBe(true);
    expect(lastFocus(s)).toBe("field");
    s = run(s, { type: "tokenSaved", status: { present: true, authorized: true }, at: 9 });
    expect(s.token.replacing).toBe(false);
    s = run(s, { type: "tokenPanel", open: false });
    expect(s.token.expanded).toBe(false);
    expect(lastFocus(s)).toBe("outcome");
  });

  it("forgets the panel layout and pending focus when another mission is chosen", () => {
    let s = run(roman(base()), { type: "tokenStatus", status: { present: true, authorized: true } }, { type: "tokenPanel", open: true }, { type: "tokenReplace", open: true });
    s = run(s, { type: "missionSwitched", form: { ...s.form, mission: "jwst", filters: {} }, generation: s.generation + 1 });
    expect(s.token).toMatchObject({ expanded: false, replacing: false, focus: null, status: { present: true, authorized: true } });
  });

  it("announces a removed token and puts focus on the empty field", () => {
    const s = run(roman(base()), { type: "tokenStatus", status: { present: true, authorized: false } }, { type: "tokenCleared" });
    expect(s.live.text).toBe("Token removed from this computer.");
    expect(lastFocus(s)).toBe("field");
  });
});

describe("mission probes", () => {
  const at = new Date(2026, 9, 16, 10, 42).getTime();
  const missionOf = (id: string) => MISSIONS.find((m) => m.id === id) ?? MISSIONS[0];
  const tess = missionOf("tess");
  const iue = missionOf("iue");

  it("marks a probe in flight, then records MAST's answer with its time", () => {
    let s = run(base(), { type: "missionProbeStarted", mission: "tess", at });
    expect(s.probes.tess).toEqual({ stage: "checking", at, error: null });
    s = run(s, { type: "missionProbed", mission: { ...tess, checkedOn: "2026-10-16" }, at: at + 500 });
    expect(s.probes.tess).toEqual({ stage: "done", at: at + 500, error: null });
    expect(s.live.text).toBe("MAST does not serve the TESS Mission Search API right now (checked 10:42).");
  });

  it("puts a mission MAST serves again back in place and says so", () => {
    const s = run(
      base(),
      { type: "missionProbeStarted", mission: "tess", at },
      { type: "missionProbed", mission: { ...tess, availability: "open", capabilities: { search: true, images: false, products: false } }, at },
    );
    expect(s.missions.find((m) => m.id === "tess")?.availability).toBe("open");
    expect(s.live.text).toBe("TESS answers at MAST now.");
  });

  it("keeps a failed probe with its error and announces it", () => {
    const s = run(base(), { type: "missionProbeStarted", mission: "tess", at }, { type: "missionProbeFailed", mission: "tess", error: netErr, at });
    expect(s.probes.tess).toEqual({ stage: "failed", at, error: netErr });
    expect(s.live.text).toBe("Could not check TESS again: Could not reach MAST (mast.stsci.edu): connection refused.");
  });

  it("records when and how a search found the mission missing, and the later of a probe and a search wins", () => {
    let s = run(base(), { type: "missionSwitched", form: { ...base().form, mission: "iue", filters: {} }, generation: 1 });
    s = run(
      s,
      { type: "searchStarted", generation: 2, text: "HD 93129A", kind: "name", name: null, query: { ...QUERY, mission: "iue" }, at: 0 },
      { type: "searchFailed", generation: 2, error: { code: "mast_not_deployed", message: "404", context: { mission: "iue", operation: "search/iue/api/v0.1/search", status: 404 } }, at },
    );
    expect(resultsState(s)).toBe("unavailable");
    expect(unavailableInfo(s)).toEqual({ mission: "iue", label: "IUE", operation: "search/iue/api/v0.1/search", at, status: 404, checkedOn: "2026-10-16" });
    const later = run(
      s,
      { type: "missionProbeStarted", mission: "iue", at: at + 60_000 },
      { type: "missionProbed", mission: { ...iue, availability: "not_deployed", capabilities: { search: false, images: false, products: false } }, at: at + 60_000 },
    );
    expect(unavailableInfo(later)).toMatchObject({ mission: "iue", at: at + 60_000, status: 404 });
    const back = run(s, { type: "missionProbed", mission: iue, at: at + 60_000 });
    expect(resultsState(back)).toBe("no-images");
    expect(unavailableInfo(back)).toBeNull();
  });

  it("leaves the not-deployed panel when a new search runs on that mission, and shows what MAST answers", () => {
    const notDeployed: MastError = { code: "mast_not_deployed", message: "404", context: { mission: "jwst", operation: "search/jwst/api/v0.1/search", status: 404 } };
    let s = run(
      base(),
      { type: "searchStarted", generation: 1, text: "274.688 -13.792", kind: "coordinates", name: null, query: QUERY, at: 0 },
      { type: "searchFailed", generation: 1, error: notDeployed, at },
    );
    expect(resultsState(s)).toBe("unavailable");
    s = run(s, { type: "searchStarted", generation: 2, text: "274.688 -13.792", kind: "coordinates", name: null, query: QUERY, at: at + 60_000 });
    expect(resultsState(s)).toBe("searching");
    expect(unavailableInfo(s)).toBeNull();
    const answered = run(s, { type: "pageLoaded", generation: 2, page: page(ROWS) });
    expect(resultsState(answered)).toBe("ready");
    expect(answered.results?.rows).toHaveLength(2);
    const again = run(s, { type: "searchFailed", generation: 2, error: notDeployed, at: at + 61_000 });
    expect(resultsState(again)).toBe("unavailable");
    expect(unavailableInfo(again)).toMatchObject({ mission: "jwst", at: at + 61_000, status: 404 });
  });

  it("keeps another mission's not-deployed answer when a search starts elsewhere", () => {
    let s = run(base(), { type: "missionSwitched", form: { ...base().form, mission: "iue", filters: {} }, generation: 1 });
    s = run(
      s,
      { type: "searchStarted", generation: 2, text: "HD 93129A", kind: "name", name: null, query: { ...QUERY, mission: "iue" }, at: 0 },
      { type: "searchFailed", generation: 2, error: { code: "mast_not_deployed", message: "404", context: { mission: "iue", status: 404 } }, at },
    );
    s = run(s, { type: "missionSwitched", form: { ...s.form, mission: "jwst", filters: { level: "combined" } }, generation: 3 });
    s = run(s, { type: "searchStarted", generation: 4, text: "274.688 -13.792", kind: "coordinates", name: null, query: QUERY, at: at + 1000 });
    expect(s.unavailable?.mission).toBe("iue");
  });

  it("describes a mission that is not deployed even before any probe", () => {
    const s = run(base(), { type: "missionSwitched", form: { ...base().form, mission: "tess", filters: {} }, generation: 1 });
    expect(unavailableInfo(s)).toEqual({ mission: "tess", label: "TESS", operation: undefined, at: null, status: null, checkedOn: "2026-10-16" });
  });
});

describe("banners", () => {
  it("shows one banner, by precedence: rate limit over network over server over cache", () => {
    let s = run(searched(), { type: "bannerRaised", banner: { kind: "cache", text: "c", retry: null } });
    s = run(s, { type: "bannerRaised", banner: { kind: "server", text: "s", retry: "search" } });
    expect(visibleBanner(s, 0)?.kind).toBe("server");
    s = run(s, { type: "bannerRaised", banner: { kind: "network", text: "n", retry: "search" } });
    expect(visibleBanner(s, 0)?.kind).toBe("network");
    s = run(s, { type: "thumbDemand", selected: ["a"], visible: [], ahead: [] }, { type: "thumbsStarted", keys: ["a"] }, { type: "thumbRateLimited", key: "a", at: 0, retryAfterSec: 42, consecutive: 1, status: 429 });
    const top = visibleBanner(s, 0);
    expect(top?.kind).toBe("rate-limited");
    expect(top?.text).toBe("MAST is limiting requests (HTTP 429). Thumbnails are paused and resume in 42 s.");
    expect(visibleBanner(s, 40_500)?.text).toBe("MAST is limiting requests (HTTP 429). Thumbnails are paused and resume in 2 s.");
    s = run(s, { type: "bannerDismissed", kind: "network" });
    s = run(s, { type: "thumbsResumed" });
    expect(visibleBanner(s, 0)?.kind).toBe("server");
  });

  it("summarises thumbnail network failures with a retry", () => {
    const s = run(searched(), { type: "thumbDemand", selected: [], visible: ["a", "b"], ahead: [] }, { type: "thumbsStarted", keys: ["a", "b"] }, { type: "thumbFailed", key: "a", reason: "network" });
    expect(visibleBanner(s, 0)).toEqual({ kind: "network", text: "Some images could not be loaded (network).", retry: "thumbs" });
  });

  it("turns the pause into an error banner once the breaker gives up", () => {
    const s = run(searched(), { type: "thumbDemand", selected: ["a"], visible: [], ahead: [] }, { type: "thumbsStarted", keys: ["a"] }, { type: "thumbRateLimited", key: "a", at: 0, retryAfterSec: 60, consecutive: 3, status: 429 });
    expect(visibleBanner(s, 0)).toEqual({ kind: "rate-limited", text: "MAST kept limiting requests, so thumbnails stay paused.", retry: "thumbs" });
  });
});

describe("previews and progress", () => {
  it("tracks download, decode and ready for the matching request id only", () => {
    let s = run(searched(), { type: "previewStarted", key: "k", requestId: "g1:preview:a:b.jpg:pane", at: 0 });
    expect(s.previews.k.stage).toBe("download");
    s = run(s, { type: "previewProgress", requestId: "g1:preview:a:b.jpg:pane", received: 10, total: 100, stage: "download" });
    expect(s.previews.k).toMatchObject({ received: 10, total: 100 });
    s = run(s, { type: "previewProgress", requestId: "other", received: 99, total: 100, stage: "decode" });
    expect(s.previews.k.received).toBe(10);
    s = run(s, { type: "previewProgress", requestId: "g1:preview:a:b.jpg:pane", received: 100, total: 100, stage: "decode" });
    expect(s.previews.k.stage).toBe("decode");
  });
});

describe("toast and live region", () => {
  it("bumps the sequence so the same text is announced twice", () => {
    const a = run(base(), { type: "toast", text: "Copied a" });
    const b = run(a, { type: "toast", text: "Copied a" });
    expect(b.toast?.seq).toBe((a.toast?.seq ?? 0) + 1);
    const c = run(b, { type: "announce", text: "x" }, { type: "announce", text: "x" });
    expect(c.live.seq).toBe(b.live.seq + 2);
  });
});

describe("results shell state", () => {
  const REF = { url: "http://asset.localhost/t.jpg", width: 128, height: 75, imageCount: 6, filename: "t_thumb.jpg" };

  it("keeps the query of the shown results and the thumbnail refs until new results replace them", () => {
    let s = searched();
    expect(s.resultsQuery).toEqual(QUERY);
    s = run(s, { type: "thumbDemand", selected: [], visible: ["jw02739-o001_t001_nircam"], ahead: [] }, { type: "thumbsStarted", keys: ["jw02739-o001_t001_nircam"] });
    s = run(s, { type: "thumbReady", key: "jw02739-o001_t001_nircam", ref: REF });
    expect(s.thumbRefs["jw02739-o001_t001_nircam"]).toEqual(REF);
    s = run(
      s,
      { type: "searchStarted", generation: 2, text: "M16", kind: "name", name: "M16", query: null, at: 0 },
      { type: "nameResolved", generation: 2, resolved: M16, query: { ...QUERY, filters: {} }, at: 1 },
    );
    expect(s.thumbRefs["jw02739-o001_t001_nircam"]).toEqual(REF);
    expect(s.resultsQuery).toEqual(QUERY);
    s = run(s, { type: "pageLoaded", generation: 2, page: page(ROWS) });
    expect(s.thumbRefs).toEqual({});
    expect(s.resultsQuery).toEqual({ ...QUERY, filters: {} });
  });

  it("moves a name search to the searching phase when MAST resolves it server-side", () => {
    let s = run(base(), { type: "searchStarted", generation: 1, text: "M16", kind: "name", name: "M16", query: null, at: 0 });
    expect(resultsState(s)).toBe("resolving");
    const byName: MastQuery = { ...QUERY, where: { kind: "name", name: "M16", radiusArcmin: 3 } };
    s = run(s, { type: "serverResolve", generation: 2, query: byName, at: 5 });
    expect(resultsState(s)).toBe("resolving");
    s = run(s, { type: "serverResolve", generation: 1, query: byName, at: 5 });
    expect(resultsState(s)).toBe("searching");
    expect(s.run).toMatchObject({ query: byName, resolved: null, searchStartedAt: 5 });
  });

  it("sorts locally, resets the order on new results and takes a server order from the applied echo", () => {
    let s = run(searched(), { type: "sortSet", sort: { keys: [{ column: "date_obs", desc: true }], by: "local" } });
    expect(s.sort).toEqual({ keys: [{ column: "date_obs", desc: true }], by: "local" });
    s = run(s, { type: "searchStarted", generation: 2, text: "2739", kind: "programme", name: null, query: { ...QUERY, sort: [{ column: "effexptm", desc: false }] }, at: 0 });
    s = run(s, { type: "pageLoaded", generation: 2, page: page(ROWS, { applied: { conditions: [], sortBy: ["effexptm"], sortDesc: [false] } }) });
    expect(s.sort).toEqual({ keys: [{ column: "effexptm", desc: false }], by: "server" });
    s = run(s, { type: "searchStarted", generation: 3, text: "2739", kind: "programme", name: null, query: QUERY, at: 0 }, { type: "pageLoaded", generation: 3, page: page(ROWS) });
    expect(s.sort).toEqual({ keys: [], by: "local" });
  });

  it("toggles table thumbnails and keeps the choice across searches", () => {
    let s = run(base(), { type: "tableThumbsToggled" });
    expect(s.tableThumbs).toBe(true);
    s = searched(s);
    expect(s.tableThumbs).toBe(true);
    expect(run(s, { type: "tableThumbsToggled" }).tableThumbs).toBe(false);
  });
});

describe("detail pane state", () => {
  const ID = "jw02739-o001_t001_nircam";
  const KEY = `jwst:${ID}`;
  const err: MastError = { code: "mast_server", message: "bad gateway", context: { status: 504 } };

  it("resizes within 360 to 640 px and switches to a drawer in a narrow window", () => {
    let s = run(searched(), { type: "detailResized", width: 300 });
    expect(s.detail.width).toBe(360);
    s = run(s, { type: "detailResized", width: 900 });
    expect(s.detail.width).toBe(640);
    expect(detailMode(s)).toBe("pane");
    s = run(s, { type: "narrowSet", narrow: true });
    expect(detailMode(s)).toBe("drawer");
    expect(base().detail.width).toBe(420);
  });

  it("shows the selected row in the pane, in the drawer only while it is open, and never under the Large view", () => {
    let s = searched();
    expect(selectedRow(s)?.datasetId).toBe(ID);
    expect(detailShown(s)).toBe(true);
    expect(detailShown(run(s, { type: "detailToggled" }))).toBe(false);
    s = run(s, { type: "narrowSet", narrow: true });
    expect(detailShown(s)).toBe(false);
    s = run(s, { type: "drawerSet", open: true });
    expect(detailShown(s)).toBe(true);
    s = run(s, { type: "largeOpened", datasetId: ID, imageKey: null });
    expect(detailShown(s)).toBe(false);
    expect(detailShown(base())).toBe(false);
  });

  it("falls back to Metadata for a mission without previews", () => {
    const iue = run(base(), { type: "missionSwitched", form: { ...base().form, mission: "iue", filters: {} }, generation: 1 });
    const s = run(
      iue,
      { type: "searchStarted", generation: 1, text: "HD 93129A", kind: "coordinates", name: null, query: { ...QUERY, mission: "iue" }, at: 0 },
      { type: "pageLoaded", generation: 1, page: page([row("SWP01234")], { mission: "iue" }) },
    );
    expect(s.detail.tab).toBe("previews");
    expect(detailTabOf(s)).toBe("metadata");
    expect(detailTabOf(run(s, { type: "detailTab", tab: "products" }))).toBe("products");
    expect(detailTabOf(searched())).toBe("previews");
  });

  it("tracks an image list for its own request only, and lets a failure be retried", () => {
    let s = run(searched(), { type: "imageListStarted", key: KEY, requestId: "g1:list:a", at: 5 });
    expect(s.lists[KEY]).toEqual({ requestId: "g1:list:a", startedAt: 5, error: null });
    s = run(s, { type: "imageListFailed", key: KEY, requestId: "g0:list:a", error: err });
    expect(s.lists[KEY].error).toBeNull();
    s = run(s, { type: "imageListFailed", key: KEY, requestId: "g1:list:a", error: err });
    expect(s.lists[KEY].error).toEqual(err);
    s = run(s, { type: "imageListRetried", key: KEY });
    expect(s.lists[KEY]).toBeUndefined();
    s = run(s, { type: "imageListStarted", key: KEY, requestId: "g1:list:b", at: 9 }, { type: "imageListCancelled", key: KEY, requestId: "g1:list:b" });
    expect(s.lists[KEY]).toBeUndefined();
    s = run(s, { type: "imageListStarted", key: KEY, requestId: "g1:list:c", at: 9 }, { type: "imageListLoaded", key: KEY, set: jwstL3Set() });
    expect(s.lists[KEY]).toBeUndefined();
    expect(s.imageSets[KEY].entries).toHaveLength(6);
  });

  it("stores the asset URL of a loaded preview and lets a failed one be retried", () => {
    const ref = {
      requestId: "p",
      path: "C:\\c\\a.2048.jpg",
      width: 2048,
      height: 1226,
      bytes: 9,
      filename: "a.2048.jpg",
      wcs: null,
      imageCount: 6,
      elapsedMs: 1,
      fromCache: false,
      size: "pane" as const,
      channels: 1 as const,
      source: { width: 14341, height: 8583, bytes: 5_800_000 },
    };
    let s = run(searched(), { type: "previewStarted", key: "k", requestId: "p", at: 0 }, { type: "previewLoaded", key: "k", requestId: "p", ref, url: "http://asset.localhost/a.jpg" });
    expect(s.previews.k).toMatchObject({ stage: "ready", url: "http://asset.localhost/a.jpg", ref });
    s = run(s, { type: "previewStarted", key: "f", requestId: "q", at: 0 }, { type: "previewFailed", key: "f", requestId: "q", error: err });
    expect(s.previews.f.stage).toBe("failed");
    s = run(s, { type: "previewRetried", key: "f" }, { type: "previewRetried", key: "k" });
    expect(s.previews.f).toBeUndefined();
    expect(s.previews.k.stage).toBe("ready");
  });

  it("keeps products per dataset through loading, ready, failure, a cancel by the user and a retry", () => {
    let s = run(searched(), { type: "productsStarted", key: KEY, requestId: "g1:products:a", at: 3 });
    expect(s.products[KEY]).toMatchObject({ stage: "loading", startedAt: 3, list: null, error: null });
    s = run(s, { type: "productsLoaded", key: KEY, requestId: "other", list: productList() });
    expect(s.products[KEY].stage).toBe("loading");
    s = run(s, { type: "productsLoaded", key: KEY, requestId: "g1:products:a", list: productList() });
    expect(s.products[KEY].stage).toBe("ready");
    expect(run(s, { type: "productsRetried", key: KEY }).products[KEY].stage).toBe("ready");
    const other = "jwst:jw02739-o002_t001_miri";
    s = run(s, { type: "productsStarted", key: other, requestId: "g1:products:b", at: 4 }, { type: "productsFailed", key: other, requestId: "g1:products:b", error: err });
    expect(s.products[other]).toMatchObject({ stage: "failed", error: err });
    s = run(s, { type: "productsRetried", key: other });
    expect(s.products[other]).toBeUndefined();
    s = run(s, { type: "productsStarted", key: other, requestId: "g1:products:c", at: 4 }, { type: "productsCancelled", key: other, requestId: "g1:products:c", byUser: true });
    expect(s.products[other].stage).toBe("cancelled");
    s = run(s, { type: "productsStarted", key: other, requestId: "g1:products:d", at: 4 }, { type: "productsCancelled", key: other, requestId: "g1:products:d", byUser: false });
    expect(s.products[other]).toBeUndefined();
  });

  it("switches the product view and the hidden files", () => {
    let s = searched();
    expect([s.productMode, s.productHidden]).toEqual(["opens", false]);
    s = run(s, { type: "productModeSet", mode: "everything" }, { type: "productHiddenToggled" });
    expect([s.productMode, s.productHidden]).toEqual(["everything", true]);
  });

  it("clears lists, previews and products when new results arrive", () => {
    let s = run(
      searched(),
      { type: "imageListStarted", key: KEY, requestId: "g1:list:a", at: 0 },
      { type: "previewStarted", key: "k", requestId: "p", at: 0 },
      { type: "productsStarted", key: KEY, requestId: "g1:products:a", at: 0 },
    );
    s = run(s, { type: "searchStarted", generation: 2, text: "2739", kind: "programme", name: null, query: QUERY, at: 0 }, { type: "pageLoaded", generation: 2, page: page(ROWS) });
    expect([s.lists, s.previews, s.products]).toEqual([{}, {}, {}]);
  });

  it("asks the results to focus a dataset, once per request", () => {
    const a = run(searched(), { type: "focusRequested", datasetId: ID });
    const b = run(a, { type: "focusRequested", datasetId: ID });
    expect(a.focusRequest?.datasetId).toBe(ID);
    expect(b.focusRequest?.seq).toBe((a.focusRequest?.seq ?? 0) + 1);
  });

  it("returns the detail to the image last shown in the Large view", () => {
    const l3 = jwstL3Set();
    const s = run(
      searched(),
      { type: "imageListLoaded", key: KEY, set: l3 },
      { type: "largeOpened", datasetId: ID, imageKey: l3.entries[1].key },
      { type: "largeImage", imageKey: l3.entries[2].key },
      { type: "largeClosed" },
    );
    expect(s.large.open).toBe(false);
    expect(detailEntry(s)?.filter).toBe("F200W");
  });
});
