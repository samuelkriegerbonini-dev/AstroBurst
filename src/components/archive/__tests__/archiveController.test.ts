import { describe, expect, it, vi } from "vitest";
import { createArchiveController, type ArchiveClock, type ArchiveServices } from "../state/archiveController";
import { createArchiveStore } from "../state/archiveStore";
import { createInflight } from "../state/inflight";
import { createInitialState, detailEntry, resultsState, romanPanelState, visibleBanner } from "../state/archiveReducer";
import { thumbStatus } from "../logic/thumbQueue";
import { SUGGESTIONS } from "../logic/suggestions";
import { EMPTY_QUERY_HINT, LEVEL_STORAGE_KEY } from "../logic/buildQuery";
import { previewKey, stripKey } from "../logic/detailView";
import { MISSIONS, jwstL3Set, page, productList, row } from "./fixtures";
import type {
  MastError,
  MastImageEntry,
  MastImageRef,
  MastMissionId,
  MastPreviewRef,
  MastProductList,
  MastProgressEvent,
  MastResolved,
  MastSearchPage,
} from "../../../shared/types/mast";

const M16: MastResolved = { query: "M16", canonicalName: "M 16", resolver: "SIMBAD", raDeg: 274.688, decDeg: -13.792, objectType: null, elapsedMs: 870, fromCache: false };

interface Deferred<T> {
  promise: Promise<T>;
  resolve(value: T): void;
  reject(err: unknown): void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (err: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function mastErr(code: MastError["code"], context: MastError["context"] = {}, message: string = code): MastError {
  return { code, message, context };
}

function flush(): Promise<void> {
  return new Promise((r) => setTimeout(r, 0));
}

function fakeClock(start = 1_000_000) {
  let now = start;
  let timers: { at: number; fn: () => void; id: number }[] = [];
  let next = 1;
  const clock: ArchiveClock & { advance(ms: number): void } = {
    now: () => now,
    setTimeout: (fn, ms) => {
      const id = next++;
      timers.push({ at: now + ms, fn, id });
      return id;
    },
    clearTimeout: (handle) => {
      timers = timers.filter((t) => t.id !== handle);
    },
    advance(ms) {
      now += ms;
      const due = timers.filter((t) => t.at <= now).sort((a, b) => a.at - b.at);
      timers = timers.filter((t) => t.at > now);
      due.forEach((t) => t.fn());
    },
  };
  return clock;
}

function thumbRef(datasetId: string): MastImageRef {
  return { requestId: `t:${datasetId}`, path: `C:\\cache\\mast\\img\\jwst\\${datasetId}\\thumb.jpg`, width: 128, height: 75, bytes: 3000, filename: `${datasetId}_thumb.jpg`, wcs: null, imageCount: 6, elapsedMs: 900, fromCache: false };
}

function services(over: Partial<ArchiveServices> = {}): ArchiveServices {
  return {
    missions: vi.fn(async () => MISSIONS),
    probeMission: vi.fn(async () => MISSIONS[0]),
    resolve: vi.fn(async () => M16),
    search: vi.fn(async () => page([row("jw02739-o001_t001_nircam"), row("jw02739-o002_t001_miri")])),
    imageList: vi.fn(async () => {
      throw mastErr("mast_not_deployed");
    }),
    thumbnail: vi.fn(async (_id, _m, datasetId) => thumbRef(datasetId)),
    preview: vi.fn(async () => {
      throw mastErr("mast_not_deployed");
    }),
    products: vi.fn(async () => {
      throw mastErr("mast_not_deployed");
    }),
    cancel: vi.fn(async () => 0),
    tokenStatus: vi.fn(async () => ({ present: false, authorized: null })),
    tokenSave: vi.fn(async () => ({ present: false, authorized: null })),
    tokenClear: vi.fn(async () => ({ present: false, authorized: null })),
    cacheInfo: vi.fn(async () => ({ bytes: 0, files: 0 })),
    cacheClear: vi.fn(async () => ({ bytes: 2_500_000, files: 40 })),
    onProgress: vi.fn(async () => () => {}),
    previewUrl: vi.fn(async (path: string) => `http://asset.localhost/${encodeURIComponent(path)}`),
    wcsInfo: vi.fn(async () => {
      throw new Error("no wcs");
    }),
    copyText: vi.fn(async () => {}),
    openUrl: vi.fn(async () => {}),
    ...over,
  };
}

function memoryStorage(initial: Record<string, string> = {}) {
  const data = { ...initial };
  return {
    data,
    getItem: (k: string) => (k in data ? data[k] : null),
    setItem: (k: string, v: string) => {
      data[k] = v;
    },
  };
}

function setup(over: Partial<ArchiveServices> = {}, storage = memoryStorage()) {
  const store = createArchiveStore(createInitialState());
  const svc = services(over);
  const clock = fakeClock();
  const inflight = createInflight();
  const ctl = createArchiveController({ store, services: svc, clock, storage, inflight });
  return { store, svc, clock, inflight, ctl, storage };
}

async function ready(over: Partial<ArchiveServices> = {}, storage = memoryStorage()) {
  const t = setup(over, storage);
  await t.ctl.init();
  return t;
}

function rows(n: number, prefix = "jw02739-o0") {
  return Array.from({ length: n }, (_, i) => row(`${prefix}${String(i).padStart(2, "0")}_t001_nircam`));
}

describe("init", () => {
  it("loads the missions, the stored level and the token status without a network check", async () => {
    const t = await ready({}, memoryStorage({ [LEVEL_STORAGE_KEY]: "all" }));
    const s = t.store.getState();
    expect(s.missions).toHaveLength(10);
    expect(s.form.filters.level).toBe("all");
    expect(s.token.status).toEqual({ present: false, authorized: null });
    expect(t.svc.tokenStatus).toHaveBeenCalledWith("token:check", false);
    await t.ctl.init();
    expect(t.svc.missions).toHaveBeenCalledTimes(1);
  });

  it("raises a decode banner when the missions cannot be loaded", async () => {
    const t = await ready({ missions: vi.fn(async () => Promise.reject(mastErr("mast_decode", {}, "bad answer"))) });
    expect(visibleBanner(t.store.getState(), t.clock.now())?.kind).toBe("decode");
  });
});

describe("search flow", () => {
  it("resolves a name first, then searches by position with generation ids", async () => {
    const t = await ready();
    t.ctl.setText("M16");
    await t.ctl.search();
    expect(t.svc.cancel).toHaveBeenCalledWith({ prefix: "g0:" });
    expect(t.svc.resolve).toHaveBeenCalledWith("g1:resolve", "M16");
    expect(t.svc.search).toHaveBeenCalledWith(
      "g1:search",
      expect.objectContaining({
        mission: "jwst",
        where: { kind: "position", raDeg: 274.688, decDeg: -13.792, radiusArcmin: 3, label: "M 16" },
        filters: { level: "combined" },
        offset: 0,
        limit: 200,
      }),
      false,
    );
    const s = t.store.getState();
    expect(resultsState(s)).toBe("ready");
    expect(s.selectedId).toBe("jw02739-o001_t001_nircam");
    expect(s.resultsQuery?.where).toMatchObject({ kind: "position", label: "M 16" });
    expect(t.inflight.snapshot()).toEqual({ total: 0, thumbs: 0 });
  });

  it("blocks an empty query with the hint and sends nothing", async () => {
    const t = await ready();
    await t.ctl.search();
    expect(t.store.getState().queryError).toEqual({ text: EMPTY_QUERY_HINT, field: "query" });
    expect(t.svc.resolve).not.toHaveBeenCalled();
    expect(t.svc.search).not.toHaveBeenCalled();
    expect(t.svc.cancel).not.toHaveBeenCalled();
  });

  it("stops at the lookup when MAST knows no such name", async () => {
    const t = await ready({ resolve: vi.fn(async () => null) });
    t.ctl.setText("XQZ NOT A TARGET 123");
    await t.ctl.search();
    expect(resultsState(t.store.getState())).toBe("not-resolved");
    expect(t.svc.search).not.toHaveBeenCalled();
  });

  it("lets MAST resolve the name when the lookup service fails", async () => {
    const t = await ready({ resolve: vi.fn(async () => Promise.reject(mastErr("mast_server", { status: 502 }))) });
    t.ctl.setText("M16");
    await t.ctl.search();
    expect(t.svc.search).toHaveBeenCalledWith("g1:search", expect.objectContaining({ where: { kind: "name", name: "M16", radiusArcmin: 3 } }), false);
    expect(resultsState(t.store.getState())).toBe("ready");
  });

  it("skips the lookup for coordinates, programmes and dataset ids", async () => {
    const t = await ready();
    t.ctl.setText("2739");
    await t.ctl.search();
    expect(t.svc.resolve).not.toHaveBeenCalled();
    expect(t.svc.search).toHaveBeenCalledWith("g1:search", expect.objectContaining({ program: 2739, where: { kind: "none" } }), false);
  });

  it("shows a blocking error when the first search fails, and Try again repeats it", async () => {
    const search = vi.fn(async (_id: string, _query: unknown, _refresh: boolean): Promise<MastSearchPage> =>
      Promise.reject(mastErr("mast_network", { operation: "search/jwst/api/v0.1/search" }, "connection refused")),
    );
    const t = await ready({ search });
    t.ctl.setText("274.688 -13.792");
    await t.ctl.search();
    expect(resultsState(t.store.getState())).toBe("error");
    expect(t.store.getState().form.text).toBe("274.688 -13.792");
    t.ctl.retry();
    await flush();
    expect(search).toHaveBeenCalledTimes(2);
    expect(search.mock.calls[1][0]).toBe("g2:search");
    expect(t.inflight.snapshot().total).toBe(0);
  });

  it("cancels a running search by its generation and restores the previous results without a banner", async () => {
    const second = deferred<MastSearchPage>();
    const search = vi.fn().mockResolvedValueOnce(page([row("first-a"), row("first-b")])).mockReturnValueOnce(second.promise);
    const t = await ready({ search });
    t.ctl.setText("274.688 -13.792");
    await t.ctl.search();
    const pending = t.ctl.search();
    await flush();
    expect(resultsState(t.store.getState())).toBe("searching");
    expect(t.svc.cancel).toHaveBeenLastCalledWith({ prefix: "g1:" });
    t.ctl.cancelSearch();
    expect(t.svc.cancel).toHaveBeenLastCalledWith({ prefix: "g2:" });
    second.reject(mastErr("mast_cancelled"));
    await pending;
    const s = t.store.getState();
    expect(resultsState(s)).toBe("ready");
    expect(s.results?.rows.map((r) => r.datasetId)).toEqual(["first-a", "first-b"]);
    expect(visibleBanner(s, t.clock.now())).toBeNull();
    expect(t.inflight.snapshot().total).toBe(0);
  });

  it("loads the next 200 rows with a page id that keeps the generation", async () => {
    const first = page(rows(200), { hasMore: true, totalReported: 450 });
    const search = vi.fn().mockResolvedValueOnce(first).mockResolvedValueOnce(page(rows(200, "jw09999-o0"), { offset: 200, hasMore: true, totalReported: 450 }));
    const t = await ready({ search });
    t.ctl.setText("2739");
    await t.ctl.search();
    await t.ctl.loadMore();
    expect(search).toHaveBeenLastCalledWith("g1:page:200", expect.objectContaining({ program: 2739, offset: 200 }), false);
    expect(t.store.getState().results?.rows).toHaveLength(400);
    expect(t.store.getState().generation).toBe(1);
  });

  it("does not ask MAST again on its own for a page that just failed, only when the user retries", async () => {
    const first = page(rows(200), { hasMore: true, totalReported: 450 });
    const search = vi
      .fn()
      .mockResolvedValueOnce(first)
      .mockRejectedValueOnce(mastErr("mast_rate_limited", { retryAfterSec: 5, status: 429, consecutive: 1, operation: "search/jwst/api/v0.1/search" }))
      .mockResolvedValueOnce(page(rows(200, "jw09999-o0"), { offset: 200, hasMore: true, totalReported: 450 }));
    const t = await ready({ search });
    t.ctl.setText("2739");
    await t.ctl.search();
    await t.ctl.loadMore({ auto: true });
    expect(search).toHaveBeenCalledTimes(2);
    expect(visibleBanner(t.store.getState(), t.clock.now())?.retry).toBe("page");
    await t.ctl.loadMore({ auto: true });
    await t.ctl.loadMore({ auto: true });
    expect(search).toHaveBeenCalledTimes(2);
    await t.ctl.loadMore();
    expect(search).toHaveBeenCalledTimes(3);
    expect(search).toHaveBeenLastCalledWith("g1:page:200", expect.objectContaining({ offset: 200 }), false);
    expect(t.store.getState().results?.rows).toHaveLength(400);
  });

  it("asks for the next page on its own again once a new search replaced the failed one", async () => {
    const search = vi
      .fn()
      .mockResolvedValueOnce(page(rows(200), { hasMore: true, totalReported: 450 }))
      .mockRejectedValueOnce(mastErr("mast_network", { operation: "search/jwst/api/v0.1/search" }, "connection refused"))
      .mockResolvedValueOnce(page(rows(200), { hasMore: true, totalReported: 450 }))
      .mockResolvedValueOnce(page(rows(200, "jw09999-o0"), { offset: 200, hasMore: true, totalReported: 450 }));
    const t = await ready({ search });
    t.ctl.setText("2739");
    await t.ctl.search();
    await t.ctl.loadMore({ auto: true });
    await t.ctl.loadMore({ auto: true });
    expect(search).toHaveBeenCalledTimes(2);
    await t.ctl.search();
    await t.ctl.loadMore({ auto: true });
    expect(search).toHaveBeenCalledTimes(4);
    expect(search).toHaveBeenLastCalledWith("g2:page:200", expect.objectContaining({ offset: 200 }), false);
  });

  it("refreshes past the search cache on request", async () => {
    const t = await ready();
    t.ctl.setText("2739");
    await t.ctl.search({ refresh: true });
    expect(t.svc.search).toHaveBeenCalledWith("g1:search", expect.anything(), true);
  });

  it("never sends a Roman request without an authorized token", async () => {
    const t = await ready();
    t.ctl.setText("M16");
    t.ctl.switchMission("roman");
    expect(resultsState(t.store.getState())).toBe("locked");
    await t.ctl.search();
    expect(t.svc.resolve).not.toHaveBeenCalled();
    expect(t.svc.search).not.toHaveBeenCalled();
  });
});

describe("mission switch and filters", () => {
  it("keeps the query and radius, resets instrument and filter, cancels the old generation and runs again", async () => {
    const t = await ready();
    t.ctl.setText("M16");
    t.ctl.setRadius("5");
    t.ctl.setFilters({ instrument: "NIRCAM", filter: "F444W", dateFrom: "2022-01-01" });
    await t.ctl.search();
    t.ctl.switchMission("hst");
    expect(t.svc.cancel).toHaveBeenCalledWith({ prefix: "g1:" });
    const form = t.store.getState().form;
    expect(form).toMatchObject({ mission: "hst", text: "M16", radiusText: "5" });
    expect(form.filters).toEqual({ dateFrom: "2022-01-01" });
    await flush();
    expect(t.svc.search).toHaveBeenLastCalledWith("g3:search", expect.objectContaining({ mission: "hst", filters: { dateFrom: "2022-01-01" } }), false);
  });

  it("does not search on a switch before any search was made, and ignores disabled missions", async () => {
    const t = await ready();
    t.ctl.setText("M16");
    t.ctl.switchMission("hst");
    await flush();
    expect(t.svc.search).not.toHaveBeenCalled();
    t.ctl.switchMission("tess");
    expect(t.store.getState().form.mission).toBe("hst");
  });

  it("reruns a shown search when a discrete filter changes and remembers the level", async () => {
    const storage = memoryStorage();
    const t = await ready({}, storage);
    t.ctl.setText("M16");
    await t.ctl.search();
    t.ctl.setFilters({ level: "all" }, { rerun: true });
    await flush();
    expect(storage.data[LEVEL_STORAGE_KEY]).toBe("all");
    expect(t.svc.search).toHaveBeenCalledTimes(2);
    expect(t.svc.search).toHaveBeenLastCalledWith("g2:search", expect.objectContaining({ filters: { level: "all" } }), false);
  });

  it("widens the radius, removes a filter chip and tries the other mission as real query edits", async () => {
    const t = await ready({ search: vi.fn(async () => page([])) });
    t.ctl.setText("10.0 -80.0");
    t.ctl.setRadius("1");
    t.ctl.setFilters({ instrument: "MIRI" });
    await t.ctl.search();
    await t.ctl.widen(3);
    expect(t.store.getState().form.radiusText).toBe("3");
    expect(t.svc.search).toHaveBeenLastCalledWith("g2:search", expect.objectContaining({ where: expect.objectContaining({ radiusArcmin: 3 }) }), false);
    await t.ctl.removeFilter("instrument");
    expect(t.store.getState().form.filters.instrument).toBeUndefined();
    expect(t.svc.search).toHaveBeenCalledTimes(3);
    await t.ctl.tryMission("hst");
    expect(t.store.getState().form.mission).toBe("hst");
    expect(t.svc.search).toHaveBeenLastCalledWith(expect.stringMatching(/^g\d+:search$/), expect.objectContaining({ mission: "hst" }), false);
  });

  it("runs a first-run suggestion after filling the form visibly", async () => {
    const t = await ready();
    const hubble = SUGGESTIONS.find((s) => s.mission === "hst");
    expect(hubble).toBeDefined();
    await t.ctl.runSuggestion(hubble!);
    const form = t.store.getState().form;
    expect(form).toMatchObject({ mission: "hst", text: "M16" });
    expect(form.filters).toMatchObject({ instrument: "WFC3", filter: "F502N" });
    expect(t.svc.search).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({ mission: "hst", filters: { instrument: "WFC3", filter: "F502N" } }), false);
  });

  it("searches around a plate-solved workspace image", async () => {
    const t = await ready();
    await t.ctl.searchAround({ raDeg: 274.71, decDeg: -13.83, radiusArcmin: 4.2, label: "673nmos.fits" });
    expect(t.store.getState().form).toMatchObject({ text: "274.71000 -13.83000", radiusText: "4.2" });
    expect(t.svc.search).toHaveBeenCalledWith("g1:search", expect.objectContaining({ where: { kind: "position", raDeg: 274.71, decDeg: -13.83, radiusArcmin: 4.2, label: null } }), false);
  });
});

describe("thumbnail queue", () => {
  async function withResults(n: number, over: Partial<ArchiveServices> = {}) {
    const list = rows(n);
    const t = await ready({ search: vi.fn(async () => page(list)), ...over });
    t.ctl.setText("2739");
    await t.ctl.search();
    return { ...t, list };
  }

  it("starts at most 4 thumbnails in priority order and the next one when one finishes", async () => {
    const pending: Deferred<MastImageRef>[] = [];
    const thumbnail = vi.fn(() => {
      const d = deferred<MastImageRef>();
      pending.push(d);
      return d.promise;
    });
    const t = await withResults(6, { thumbnail });
    const ids = t.list.map((r) => r.datasetId);
    t.ctl.setThumbDemand({ selected: [ids[5]], visible: ids, ahead: [] });
    expect(thumbnail).toHaveBeenCalledTimes(4);
    expect(thumbnail.mock.calls.map((c) => (c as unknown[])[0])).toEqual([`g1:thumb:${ids[5]}`, `g1:thumb:${ids[0]}`, `g1:thumb:${ids[1]}`, `g1:thumb:${ids[2]}`]);
    expect(thumbnail).toHaveBeenCalledWith(`g1:thumb:${ids[5]}`, "jwst", ids[5], "combined", null);
    expect(t.inflight.snapshot()).toEqual({ total: 4, thumbs: 4 });
    pending[0].resolve(thumbRef(ids[5]));
    await flush();
    expect(t.store.getState().thumbRefs[ids[5]]).toMatchObject({ width: 128, height: 75, imageCount: 6 });
    expect(t.store.getState().thumbRefs[ids[5]].url).toMatch(/^http:\/\/asset\.localhost\//);
    expect(thumbnail).toHaveBeenCalledTimes(5);
    expect(t.inflight.snapshot().thumbs).toBe(4);
  });

  it("marks a failed thumbnail with its short reason", async () => {
    const t = await withResults(1, { thumbnail: vi.fn(async () => Promise.reject(mastErr("mast_not_image"))) });
    const id = t.list[0].datasetId;
    t.ctl.setThumbDemand({ selected: [], visible: [id], ahead: [] });
    await flush();
    const q = t.store.getState().thumbs;
    expect(q.done[id]).toBe("failed");
    expect(q.reasons[id]).toBe("not an image");
  });

  it("pauses on a rate limit, shows the queue as paused and resumes after Retry-After", async () => {
    const thumbnail = vi.fn().mockRejectedValueOnce(mastErr("mast_rate_limited", { retryAfterSec: 5, consecutive: 1, status: 429 })).mockImplementation(async (_i: string, _m: string, d: string) => thumbRef(d));
    const t = await withResults(2, { thumbnail });
    const ids = t.list.map((r) => r.datasetId);
    t.ctl.setThumbDemand({ selected: [], visible: [ids[0]], ahead: [] });
    await flush();
    t.ctl.setThumbDemand({ selected: [], visible: ids, ahead: [] });
    const s = t.store.getState();
    expect(thumbStatus(s.thumbs, ids[1], t.clock.now())).toBe("paused");
    expect(visibleBanner(s, t.clock.now())?.kind).toBe("rate-limited");
    expect(thumbnail).toHaveBeenCalledTimes(1);
    t.clock.advance(5_000);
    await flush();
    expect(thumbnail).toHaveBeenCalledTimes(3);
    await flush();
    expect(t.store.getState().thumbs.done).toEqual({ [ids[0]]: "ready", [ids[1]]: "ready" });
  });

  it("starts nothing while Archive is hidden and resumes when it is shown again", async () => {
    const t = await withResults(3);
    t.ctl.setActive(false);
    t.ctl.setThumbDemand({ selected: [], visible: t.list.map((r) => r.datasetId), ahead: [] });
    expect(t.svc.thumbnail).not.toHaveBeenCalled();
    t.ctl.setActive(true);
    expect(t.svc.thumbnail).toHaveBeenCalledTimes(3);
  });

  it("drops a thumbnail that finishes after a newer search replaced the results", async () => {
    const late = deferred<MastImageRef>();
    const thumbnail = vi.fn().mockReturnValueOnce(late.promise).mockImplementation(async (_i: string, _m: string, d: string) => thumbRef(d));
    const search = vi.fn().mockResolvedValueOnce(page([row("old-a")])).mockResolvedValueOnce(page([row("new-a")]));
    const t = await ready({ search, thumbnail });
    t.ctl.setText("2739");
    await t.ctl.search();
    t.ctl.setThumbDemand({ selected: [], visible: ["old-a"], ahead: [] });
    await t.ctl.search();
    late.resolve(thumbRef("old-a"));
    await flush();
    const s = t.store.getState();
    expect(s.thumbRefs["old-a"]).toBeUndefined();
    expect(s.thumbs.done["old-a"]).toBeUndefined();
    expect(t.inflight.snapshot().thumbs).toBe(0);
  });

  it("retries failed thumbnails on request", async () => {
    const thumbnail = vi.fn().mockRejectedValueOnce(mastErr("mast_network", {}, "reset")).mockImplementation(async (_i: string, _m: string, d: string) => thumbRef(d));
    const t = await withResults(1, { thumbnail });
    const id = t.list[0].datasetId;
    t.ctl.setThumbDemand({ selected: [], visible: [id], ahead: [] });
    await flush();
    expect(visibleBanner(t.store.getState(), t.clock.now())?.retry).toBe("thumbs");
    t.ctl.retry();
    await flush();
    expect(t.store.getState().thumbs.done[id]).toBe("ready");
  });

  it("starts no thumbnails while a search runs", async () => {
    const second = deferred<MastSearchPage>();
    const search = vi.fn().mockResolvedValueOnce(page([row("a"), row("b")])).mockReturnValueOnce(second.promise);
    const t = await ready({ search });
    t.ctl.setText("2739");
    await t.ctl.search();
    void t.ctl.search();
    await flush();
    t.ctl.setThumbDemand({ selected: [], visible: ["a", "b"], ahead: [] });
    expect(t.svc.thumbnail).not.toHaveBeenCalled();
    second.resolve(page([row("c")]));
    await flush();
  });
});

describe("table sort, view, copy and cache", () => {
  it("sorts locally when every row is loaded: ascending, then descending, then MAST order", async () => {
    const t = await ready();
    t.ctl.setText("2739");
    await t.ctl.search();
    const date = { name: "date_obs", label: "Date obs (UTC)", kind: "datetime" as const, sortable: false, unit: null };
    t.ctl.sortBy(date, false);
    expect(t.store.getState().sort).toEqual({ keys: [{ column: "date_obs", desc: false }], by: "local" });
    t.ctl.sortBy(date, false);
    expect(t.store.getState().sort.keys).toEqual([{ column: "date_obs", desc: true }]);
    t.ctl.sortBy(date, false);
    expect(t.store.getState().sort.keys).toEqual([]);
    expect(t.svc.search).toHaveBeenCalledTimes(1);
  });

  it("asks MAST to sort a sortable column while more rows exist and refuses an unsortable one", async () => {
    const t = await ready({ search: vi.fn(async () => page(rows(200), { hasMore: true, totalReported: 450 })) });
    t.ctl.setText("2739");
    await t.ctl.search();
    t.ctl.sortBy({ name: "_prog_obs", label: "Prog.Obs", kind: "text", sortable: false, unit: null }, false);
    expect(t.store.getState().live.text).toBe("MAST cannot sort by this column while more rows exist");
    expect(t.svc.search).toHaveBeenCalledTimes(1);
    t.ctl.sortBy({ name: "effexptm", label: "Exp (s)", kind: "float", sortable: true, unit: "s" }, false);
    await flush();
    expect(t.svc.search).toHaveBeenLastCalledWith("g2:search", expect.objectContaining({ sort: [{ column: "effexptm", desc: false }] }), false);
  });

  it("copies a dataset id and confirms it with a toast", async () => {
    const t = await ready();
    await t.ctl.copy("jw02739-o001_t001_nircam");
    expect(t.svc.copyText).toHaveBeenCalledWith("jw02739-o001_t001_nircam");
    expect(t.store.getState().toast?.text).toBe("Copied jw02739-o001_t001_nircam");
  });

  it("clears the MAST cache and says how much was freed", async () => {
    const t = await ready();
    await t.ctl.clearCache();
    expect(t.svc.cacheClear).toHaveBeenCalledWith("all");
    expect(t.store.getState().toast?.text).toBe("MAST cache cleared: 2.5 MB freed");
  });

  it("toggles the view and the table thumbnails", async () => {
    const t = await ready();
    t.ctl.toggleView();
    expect(t.store.getState().view).toBe("table");
    t.ctl.toggleTableThumbs();
    expect(t.store.getState().tableThumbs).toBe(true);
  });

  it("opens Roman access from anywhere by switching to the Roman tab", async () => {
    const t = await ready();
    t.ctl.openRomanAccess();
    expect(t.store.getState().form.mission).toBe("roman");
  });
});

describe("detail pane", () => {
  const NIRCAM = "jw02739-o001_t001_nircam";
  const MIRI = "jw02739-o002_t001_miri";
  const F187N = jwstL3Set(NIRCAM).entries[1];
  const paneKey = (datasetId: string, entry: MastImageEntry, size: "pane" | "large" = "pane") => previewKey("jwst", datasetId, entry.previewFilename ?? "", size);

  function previewRef(datasetId: string, filename: string, size: "pane" | "large"): MastPreviewRef {
    const width = size === "pane" ? 2048 : 4096;
    return {
      requestId: `p:${filename}`,
      path: `C:\\cache\\mast\\img\\jwst\\${datasetId}\\${filename}.${width}.jpg`,
      width,
      height: Math.round((width * 8583) / 14341),
      bytes: 1000,
      filename: `${filename}.${width}.jpg`,
      wcs: null,
      imageCount: 6,
      elapsedMs: 900,
      fromCache: false,
      size,
      channels: 1,
      source: { width: 14341, height: 8583, bytes: 5_800_000 },
    };
  }

  function previewOk(): ArchiveServices["preview"] {
    return vi.fn(async (_id: string, _m: MastMissionId, datasetId: string, filename: string, size: "pane" | "large") => previewRef(datasetId, filename, size));
  }

  async function detailReady(over: Partial<ArchiveServices> = {}) {
    const t = await ready({
      imageList: vi.fn(async (_id: string, _m: MastMissionId, datasetId: string) => jwstL3Set(datasetId)),
      preview: previewOk(),
      products: vi.fn(async () => productList()),
      ...over,
    });
    t.ctl.setText("2739");
    await t.ctl.search();
    await flush();
    return t;
  }

  it("lists the selected dataset's images at once and loads the pane preview after 400 ms of stable selection", async () => {
    const t = await detailReady();
    expect(t.svc.imageList).toHaveBeenCalledWith(`g1:list:${NIRCAM}`, "jwst", NIRCAM);
    expect(t.svc.preview).not.toHaveBeenCalled();
    t.clock.advance(399);
    expect(t.svc.preview).not.toHaveBeenCalled();
    t.clock.advance(1);
    expect(t.svc.preview).toHaveBeenCalledWith(`g1:preview:${NIRCAM}:${F187N.previewFilename}:pane`, "jwst", NIRCAM, F187N.previewFilename, "pane");
    await flush();
    const p = t.store.getState().previews[paneKey(NIRCAM, F187N)];
    expect(p.stage).toBe("ready");
    expect(p.url).toMatch(/^http:\/\/asset\.localhost\//);
    expect(t.inflight.snapshot().total).toBe(0);
  });

  it("cancels the pane preview by its exact id when another dataset is selected, and lists that one", async () => {
    const pending = deferred<MastPreviewRef>();
    const t = await detailReady({ preview: vi.fn(() => pending.promise) });
    t.clock.advance(400);
    const id = vi.mocked(t.svc.preview).mock.calls[0][0];
    t.ctl.select(MIRI);
    expect(t.svc.cancel).toHaveBeenCalledWith({ requestIds: [id] });
    expect(t.store.getState().previews).toEqual({});
    pending.reject(mastErr("mast_cancelled"));
    await flush();
    expect(t.store.getState().previews).toEqual({});
    expect(t.svc.imageList).toHaveBeenLastCalledWith(`g1:list:${MIRI}`, "jwst", MIRI);
  });

  it("follows mast:progress for the preview in flight", async () => {
    let progress: ((event: MastProgressEvent) => void) | undefined;
    const t = await detailReady({
      preview: vi.fn(() => new Promise<MastPreviewRef>(() => {})),
      onProgress: vi.fn(async (cb: (event: MastProgressEvent) => void) => {
        progress = cb;
        return () => {};
      }),
    });
    t.clock.advance(400);
    const id = vi.mocked(t.svc.preview).mock.calls[0][0];
    expect(progress).toBeDefined();
    progress?.({ requestId: id, received: 2_200_000, total: 5_800_000, stage: "download" });
    expect(t.store.getState().previews[paneKey(NIRCAM, F187N)]).toMatchObject({ stage: "download", received: 2_200_000, total: 5_800_000 });
    progress?.({ requestId: id, received: 5_800_000, total: 5_800_000, stage: "decode" });
    expect(t.store.getState().previews[paneKey(NIRCAM, F187N)].stage).toBe("decode");
  });

  it("steps through the images of the group in the detail", async () => {
    const t = await detailReady();
    t.ctl.stepImage(1);
    expect(detailEntry(t.store.getState())?.filter).toBe("F200W");
    t.ctl.stepImage(-1);
    t.ctl.stepImage(-1);
    t.ctl.stepImage(-1);
    expect(detailEntry(t.store.getState())?.filter).toBe("F090W");
  });

  it("opens the Large view on the image shown, loads the 4096 px copy after the pane copy and sends focus back to the dataset", async () => {
    const t = await detailReady();
    t.clock.advance(400);
    await flush();
    t.ctl.openLarge(NIRCAM);
    expect(t.store.getState().large).toMatchObject({ open: true, datasetId: NIRCAM, imageKey: F187N.key });
    expect(t.svc.preview).toHaveBeenLastCalledWith(`g1:preview:${NIRCAM}:${F187N.previewFilename}:large`, "jwst", NIRCAM, F187N.previewFilename, "large");
    await flush();
    expect(t.store.getState().previews[paneKey(NIRCAM, F187N, "large")].stage).toBe("ready");
    t.ctl.closeLarge();
    const s = t.store.getState();
    expect(s.large.open).toBe(false);
    expect(s.focusRequest?.datasetId).toBe(NIRCAM);
  });

  it("asks for the pane copy before the large copy when the Large view opens on a dataset not shown yet", async () => {
    const panes = deferred<MastPreviewRef>();
    const preview = vi.fn((_id: string, _m: MastMissionId, datasetId: string, filename: string, size: "pane" | "large") =>
      size === "pane" ? panes.promise : Promise.resolve(previewRef(datasetId, filename, size)),
    );
    const t = await detailReady({ preview });
    t.ctl.openLarge(MIRI);
    await flush();
    expect(t.svc.imageList).toHaveBeenLastCalledWith(`g1:list:${MIRI}`, "jwst", MIRI);
    expect(preview).toHaveBeenCalledTimes(1);
    expect(preview.mock.calls[0].slice(2)).toEqual([MIRI, jwstL3Set(MIRI).entries[1].previewFilename, "pane"]);
    panes.resolve(previewRef(MIRI, "x", "pane"));
    await flush();
    expect(preview).toHaveBeenCalledTimes(2);
    expect(preview.mock.calls[1][4]).toBe("large");
  });

  it("walks images and datasets in the Large view and cancels the copy it leaves", async () => {
    const preview = vi.fn((_id: string, _m: MastMissionId, datasetId: string, filename: string, size: "pane" | "large") =>
      size === "pane" ? Promise.resolve(previewRef(datasetId, filename, size)) : new Promise<MastPreviewRef>(() => {}),
    );
    const t = await detailReady({ preview });
    t.ctl.openLarge(NIRCAM);
    await flush();
    const firstLarge = preview.mock.calls.find((c) => c[4] === "large")?.[0];
    expect(firstLarge).toBe(`g1:preview:${NIRCAM}:${F187N.previewFilename}:large`);
    t.ctl.largeStepImage(1);
    expect(t.store.getState().large.imageKey).toBe(jwstL3Set(NIRCAM).entries[2].key);
    expect(t.svc.cancel).toHaveBeenCalledWith({ requestIds: [firstLarge] });
    t.ctl.largeStepDataset(1);
    expect(t.store.getState().large.datasetId).toBe(MIRI);
    expect(t.store.getState().selectedId).toBe(MIRI);
    t.ctl.largeStepDataset(1);
    expect(t.store.getState().large.datasetId).toBe(MIRI);
    t.ctl.largeStepDataset(-1);
    expect(t.store.getState().large.datasetId).toBe(NIRCAM);
  });

  it("lists products when the Products tab opens, cancels them when another dataset is selected and refreshes on request", async () => {
    const lists: Deferred<MastProductList>[] = [];
    const products = vi.fn(() => {
      const d = deferred<MastProductList>();
      lists.push(d);
      return d.promise;
    });
    const t = await detailReady({ products });
    t.ctl.setDetailTab("products");
    expect(products).toHaveBeenCalledWith(`g1:products:${NIRCAM}`, "jwst", [NIRCAM], false);
    t.ctl.select(MIRI);
    expect(t.svc.cancel).toHaveBeenCalledWith({ requestIds: [`g1:products:${NIRCAM}`] });
    expect(products).toHaveBeenCalledTimes(1);
    t.clock.advance(400);
    expect(products).toHaveBeenLastCalledWith(`g1:products:${MIRI}`, "jwst", [MIRI], false);
    lists[1].resolve(productList());
    await flush();
    expect(t.store.getState().products[`jwst:${MIRI}`].stage).toBe("ready");
    t.ctl.refreshProducts();
    expect(products).toHaveBeenLastCalledWith(`g1:products:${MIRI}`, "jwst", [MIRI], true);
    expect(t.store.getState().products[`jwst:${MIRI}`].stage).toBe("loading");
  });

  it("keeps a products listing the user stopped until it is asked for again", async () => {
    const products = vi.fn(() => new Promise<MastProductList>(() => {}));
    const t = await detailReady({ products });
    t.ctl.setDetailTab("products");
    t.ctl.cancelProducts();
    expect(t.svc.cancel).toHaveBeenLastCalledWith({ requestIds: [`g1:products:${NIRCAM}`] });
    expect(t.store.getState().products[`jwst:${NIRCAM}`].stage).toBe("cancelled");
    t.clock.advance(1000);
    expect(products).toHaveBeenCalledTimes(1);
    t.ctl.retryDetail();
    expect(products).toHaveBeenCalledTimes(2);
  });

  it("asks for filmstrip thumbnails through the shared queue with their file names", async () => {
    const t = await detailReady();
    const first = jwstL3Set(NIRCAM).entries[0];
    expect(t.svc.thumbnail).toHaveBeenCalledWith(`g1:thumb:${NIRCAM}:${first.thumbnailFilename}`, "jwst", NIRCAM, "combined", first.thumbnailFilename);
    await flush();
    expect(t.store.getState().thumbRefs[stripKey(NIRCAM, first.thumbnailFilename ?? "")]?.url).toMatch(/^http:\/\/asset\.localhost\//);
    expect(vi.mocked(t.svc.thumbnail).mock.calls.length).toBeLessThanOrEqual(6);
  });

  it("makes no image requests for exclusive data or while the pane is hidden", async () => {
    const t = await ready({
      search: vi.fn(async () => page([row("jw01234-o001_t001_miri", { access: "PROPRIETARY", releaseDate: "2027-03-02T00:00:00" }), row(MIRI)])),
      imageList: vi.fn(async (_id: string, _m: MastMissionId, datasetId: string) => jwstL3Set(datasetId)),
      preview: previewOk(),
    });
    t.ctl.setText("2739");
    await t.ctl.search();
    await flush();
    t.clock.advance(1000);
    expect(t.svc.imageList).not.toHaveBeenCalled();
    expect(t.svc.preview).not.toHaveBeenCalled();
    t.ctl.toggleDetail();
    t.ctl.select(MIRI);
    await flush();
    expect(t.svc.imageList).not.toHaveBeenCalled();
    t.ctl.toggleDetail();
    expect(t.svc.imageList).toHaveBeenCalledWith(`g1:list:${MIRI}`, "jwst", MIRI);
  });

  it("opens the drawer in a narrow window and sends focus back to the card when it closes", async () => {
    const t = await detailReady();
    t.ctl.setNarrow(true);
    t.ctl.select(MIRI);
    await flush();
    expect(t.svc.imageList).not.toHaveBeenCalledWith(`g1:list:${MIRI}`, "jwst", MIRI);
    t.ctl.openDrawer();
    expect(t.svc.imageList).toHaveBeenLastCalledWith(`g1:list:${MIRI}`, "jwst", MIRI);
    t.ctl.closeDrawer();
    const s = t.store.getState();
    expect(s.detail.drawerOpen).toBe(false);
    expect(s.focusRequest?.datasetId).toBe(MIRI);
  });

  it("retries a failed image list and a failed preview from the detail", async () => {
    const imageList = vi
      .fn()
      .mockRejectedValueOnce(mastErr("mast_network", {}, "reset"))
      .mockImplementation(async (_id: string, _m: MastMissionId, datasetId: string) => jwstL3Set(datasetId));
    const preview = vi
      .fn()
      .mockRejectedValueOnce(mastErr("mast_no_image", { status: 404, detail: "Image not found" }))
      .mockImplementation(async (_id: string, _m: MastMissionId, datasetId: string, filename: string, size: "pane" | "large") => previewRef(datasetId, filename, size));
    const t = await detailReady({ imageList, preview });
    expect(t.store.getState().lists[`jwst:${NIRCAM}`].error?.code).toBe("mast_network");
    t.ctl.retryDetail();
    await flush();
    expect(imageList).toHaveBeenCalledTimes(2);
    t.clock.advance(400);
    await flush();
    expect(t.store.getState().previews[paneKey(NIRCAM, F187N)].stage).toBe("failed");
    t.ctl.retryDetail();
    await flush();
    expect(preview).toHaveBeenCalledTimes(2);
    expect(t.store.getState().previews[paneKey(NIRCAM, F187N)].stage).toBe("ready");
  });

  it("lets the user stop the pane download and does not restart it on its own", async () => {
    const preview = vi.fn((_id: string) => new Promise<MastPreviewRef>(() => {}));
    const t = await detailReady({ preview });
    t.clock.advance(400);
    expect(preview).toHaveBeenCalledTimes(1);
    t.ctl.cancelPreview("pane");
    expect(t.svc.cancel).toHaveBeenLastCalledWith({ requestIds: [preview.mock.calls[0][0]] });
    expect(t.store.getState().previews[paneKey(NIRCAM, F187N)]).toMatchObject({ stage: "failed", error: { code: "mast_cancelled" } });
    t.clock.advance(1000);
    expect(preview).toHaveBeenCalledTimes(1);
  });
});

describe("Roman token", () => {
  const MARKER = "AB-TEST-TOKEN-0123456789abcdef";
  const romanPage = () => page([row("r0001", { levelClass: "unknown", level: null, instrument: "WFI" })], { mission: "roman" });
  const savedAfterFirst = () =>
    vi.fn<ArchiveServices["tokenStatus"]>().mockResolvedValueOnce({ present: false, authorized: null }).mockResolvedValue({ present: true, authorized: null });

  it("checks the format before any request and says why the token was refused", async () => {
    const t = await ready();
    t.ctl.switchMission("roman");
    await t.ctl.saveToken("  short  ");
    expect(t.svc.tokenSave).not.toHaveBeenCalled();
    const s = t.store.getState();
    expect(romanPanelState(s)).toBe("rejected");
    expect(s.token.message).toBe("This is too short for a MAST token (at least 16 characters).");
  });

  it("saves a trimmed token under token:save, unlocks Roman and runs the typed query", async () => {
    const tokenSave = vi.fn(async () => ({ present: true, authorized: true }));
    const search = vi.fn(async () => romanPage());
    const t = await ready({ tokenSave, search });
    t.ctl.setText("274.688 -13.792");
    t.ctl.switchMission("roman");
    await t.ctl.saveToken(`  ${MARKER}\n`);
    expect(tokenSave).toHaveBeenCalledWith("token:save", MARKER);
    await flush();
    const s = t.store.getState();
    expect(romanPanelState(s)).toBe("authorized");
    expect(s.live.text === "Roman access confirmed." || s.live.text.startsWith("Search finished")).toBe(true);
    expect(search).toHaveBeenCalledWith(expect.stringMatching(/^g\d+:search$/), expect.objectContaining({ mission: "roman" }), false);
    expect(JSON.stringify(s)).not.toContain(MARKER);
  });

  it("keeps Roman locked and runs nothing when MAST accepts the token without Roman access", async () => {
    const t = await ready({ tokenSave: vi.fn(async () => ({ present: true, authorized: false })) });
    t.ctl.setText("M16");
    t.ctl.switchMission("roman");
    await t.ctl.saveToken(MARKER);
    await flush();
    expect(romanPanelState(t.store.getState())).toBe("not-authorized");
    expect(t.svc.search).not.toHaveBeenCalled();
    expect(t.svc.resolve).not.toHaveBeenCalled();
  });

  it("asks for the stored status again after a failed save and shows what is on disk", async () => {
    const tokenSave = vi.fn(async () => Promise.reject(mastErr("mast_token_rejected", {}, "anon")));
    const tokenStatus = vi.fn(async () => ({ present: false, authorized: null }));
    const t = await ready({ tokenSave, tokenStatus });
    t.ctl.switchMission("roman");
    await t.ctl.saveToken(MARKER);
    expect(tokenStatus).toHaveBeenLastCalledWith("token:check", false);
    const s = t.store.getState();
    expect(romanPanelState(s)).toBe("rejected");
    expect(s.token.message).toBe("MAST did not accept this token (mistyped, expired or revoked). Nothing was saved.");
  });

  it("tells a network failure at /info from one at the Roman check", async () => {
    const atInfo = await ready({ tokenSave: vi.fn(async () => Promise.reject(mastErr("mast_network", { operation: "info" }, "connection refused"))) });
    atInfo.ctl.switchMission("roman");
    await atInfo.ctl.saveToken(MARKER);
    expect(atInfo.store.getState().token.message).toBe("Could not reach MAST to check the token. Nothing was saved.");
    const atCheck = await ready({
      tokenSave: vi.fn(async () => Promise.reject(mastErr("mast_network", { operation: "search/roman/api/v0.1/auth_check" }, "connection refused"))),
      tokenStatus: savedAfterFirst(),
    });
    atCheck.ctl.switchMission("roman");
    await atCheck.ctl.saveToken(MARKER);
    expect(atCheck.store.getState().token.message).toBe("Token saved. MAST could not be reached to check Roman access.");
    expect(romanPanelState(atCheck.store.getState())).toBe("unreachable");
  });

  it("cancels a running check by its exact id and reads back whether the token was kept", async () => {
    const pending = deferred<{ present: boolean; authorized: boolean | null }>();
    const tokenSave = vi.fn(() => pending.promise);
    const t = await ready({ tokenSave, tokenStatus: savedAfterFirst() });
    t.ctl.switchMission("roman");
    const saving = t.ctl.saveToken(MARKER);
    expect(romanPanelState(t.store.getState())).toBe("checking");
    t.ctl.cancelTokenCheck();
    expect(t.svc.cancel).toHaveBeenLastCalledWith({ requestIds: ["token:save"] });
    pending.reject(mastErr("mast_cancelled"));
    await saving;
    const s = t.store.getState();
    expect(s.token.checking).toBe(false);
    expect(s.token.status).toEqual({ present: true, authorized: null });
    expect(romanPanelState(s)).toBe("unreachable");
    expect(s.token.message).toBeNull();
  });

  it("checks a stored token with token:check and shows an expired one", async () => {
    const tokenStatus = vi.fn(async (_id: string, check: boolean) => (check ? Promise.reject(mastErr("mast_token_rejected")) : { present: true, authorized: false }));
    const t = await ready({ tokenStatus });
    t.ctl.switchMission("roman");
    await t.ctl.checkToken();
    expect(tokenStatus).toHaveBeenLastCalledWith("token:check", true);
    expect(romanPanelState(t.store.getState())).toBe("expired");
  });

  it("checks an unchecked stored token once when Roman is opened, and never without a token", async () => {
    const tokenStatus = vi.fn(async (_id: string, check: boolean) => (check ? { present: true, authorized: true } : { present: true, authorized: null }));
    const t = await ready({ tokenStatus });
    t.ctl.switchMission("roman");
    await flush();
    expect(tokenStatus).toHaveBeenCalledWith("token:check", true);
    expect(romanPanelState(t.store.getState())).toBe("authorized");
    t.ctl.switchMission("jwst");
    t.ctl.switchMission("roman");
    await flush();
    expect(tokenStatus.mock.calls.filter((c) => c[1] === true)).toHaveLength(1);

    const none = await ready();
    none.ctl.switchMission("roman");
    none.ctl.openRomanAccess();
    await flush();
    expect(none.svc.tokenStatus).toHaveBeenCalledTimes(1);
    expect(none.svc.tokenStatus).toHaveBeenCalledWith("token:check", false);
  });

  it("removes the token and returns to the empty form", async () => {
    const t = await ready({ tokenStatus: vi.fn(async () => ({ present: true, authorized: false })) });
    t.ctl.switchMission("roman");
    await t.ctl.clearToken();
    expect(t.svc.tokenClear).toHaveBeenCalledTimes(1);
    const s = t.store.getState();
    expect(romanPanelState(s)).toBe("none");
    expect(s.live.text).toBe("Token removed from this computer.");
  });

  it("opens the MAST token page in the browser, and says so when it cannot", async () => {
    const openUrl = vi.fn(async () => {});
    const t = await ready({ openUrl });
    await t.ctl.openTokenPage();
    expect(openUrl).toHaveBeenCalledWith("https://auth.mast.stsci.edu/token?suggested_name=AstroBurst&suggested_scope=mast:exclusive_access");
    const broken = await ready({ openUrl: vi.fn(async () => Promise.reject(new Error("no browser"))) });
    await broken.ctl.openTokenPage();
    expect(broken.store.getState().toast?.text).toBe("Could not open the browser. The token page is auth.mast.stsci.edu/token.");
  });

  it("opens the token management view from the footer or the palette when Roman is unlocked", async () => {
    const t = await ready({ tokenStatus: vi.fn(async () => ({ present: true, authorized: true })) });
    t.ctl.openRomanAccess();
    await flush();
    const s = t.store.getState();
    expect(s.form.mission).toBe("roman");
    expect(s.token.expanded).toBe(true);
    expect(s.token.focus?.target).toBe("panel");
    t.ctl.replaceToken(true);
    expect(t.store.getState().token.replacing).toBe(true);
    t.ctl.setRomanPanel(false);
    expect(t.store.getState().token.expanded).toBe(false);
  });

  it("ignores a second save while a check runs", async () => {
    const pending = deferred<{ present: boolean; authorized: boolean | null }>();
    const tokenSave = vi.fn(() => pending.promise);
    const t = await ready({ tokenSave });
    t.ctl.switchMission("roman");
    const first = t.ctl.saveToken(MARKER);
    await t.ctl.saveToken(MARKER);
    await t.ctl.checkToken();
    expect(tokenSave).toHaveBeenCalledTimes(1);
    pending.resolve({ present: true, authorized: false });
    await first;
  });
});

describe("mission probes", () => {
  const at = 1_000_000;

  it("probes a mission and records MAST's answer", async () => {
    const tess = MISSIONS.find((m) => m.id === "tess") ?? MISSIONS[0];
    const probeMission = vi.fn(async () => tess);
    const t = await ready({ probeMission });
    await t.ctl.checkMission("tess");
    expect(probeMission).toHaveBeenCalledWith("tess");
    expect(t.store.getState().probes.tess).toEqual({ stage: "done", at, error: null });
  });

  it("ignores a second probe of the same mission while one runs", async () => {
    const pending = deferred<(typeof MISSIONS)[number]>();
    const probeMission = vi.fn(() => pending.promise);
    const t = await ready({ probeMission });
    const first = t.ctl.checkMission("tess");
    await t.ctl.checkMission("tess");
    expect(probeMission).toHaveBeenCalledTimes(1);
    pending.resolve(MISSIONS.find((m) => m.id === "tess") ?? MISSIONS[0]);
    await first;
  });

  it("keeps a failed probe's error", async () => {
    const t = await ready({ probeMission: vi.fn(async () => Promise.reject(mastErr("mast_network", {}, "connection refused"))) });
    await t.ctl.checkMission("tess");
    expect(t.store.getState().probes.tess).toMatchObject({ stage: "failed", error: { code: "mast_network" } });
  });

  it("searches again when the mission on screen answers once more", async () => {
    const iue = MISSIONS.find((m) => m.id === "iue") ?? MISSIONS[0];
    const search = vi.fn(async (_id: string, query: { mission: MastMissionId }) => {
      if (search.mock.calls.length === 1) throw mastErr("mast_not_deployed", { mission: query.mission, operation: "search/iue/api/v0.1/search", status: 404 });
      return page([row("SWP01234", { levelClass: "unknown", level: null })], { mission: "iue" });
    });
    const t = await ready({ search, probeMission: vi.fn(async () => iue) });
    t.ctl.switchMission("iue");
    t.ctl.setText("274.688 -13.792");
    await t.ctl.search();
    expect(resultsState(t.store.getState())).toBe("unavailable");
    await t.ctl.checkMission("iue");
    await flush();
    expect(search).toHaveBeenCalledTimes(2);
    expect(resultsState(t.store.getState())).toBe("ready");
  });
});
