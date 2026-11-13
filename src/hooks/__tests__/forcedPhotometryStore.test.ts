import { describe, it, expect, vi } from "vitest";
import { EMPTY_FORCED, ForcedPhotometryStore } from "../useForcedPhotometryStore";
import {
  DEFAULT_FORCED_FIELDS,
  forcedRequest,
  isStale,
  parseForcedPositions,
  requestKey,
} from "../../utils/forcedPhotometryRequest";
import type { ForcedPhotometryRequest, ForcedPhotometryResult } from "../../shared/types/forcedPhotometry";

type RunFake = (request: ForcedPhotometryRequest) => Promise<ForcedPhotometryResult>;

function result(n: number): ForcedPhotometryResult {
  return {
    bands: [
      {
        index: 0,
        path: "C:/d/r.fits",
        file_name: "r.fits",
        label: "r",
        error: null,
        filter: null,
        photcal: null,
        calibration_origin: null,
        jy_per_native_unit: null,
        bunit: null,
        pixel_scale_arcsec: 0.262,
        aperture_radius_px: 3.8,
        annulus_px: null,
        aperture_correction: null,
        err_origin: null,
        dq_source: null,
        f_corr: null,
        n_measured: n,
        n_failed: 0,
        warnings: [],
      },
    ],
    sources: Array.from({ length: n }, (_, i) => ({ id: `s${i + 1}`, ra_deg: 53.1 + i * 0.1, dec_deg: -28.1, cells: [null] })),
    aperture: { radius_arcsec: 1, correction: "none" },
    limit_sigma: 3,
    elapsed_ms: 5,
    warnings: [],
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function ready(store: ForcedPhotometryStore): void {
  store.addBand({ name: "r.fits", path: "C:/d/r.fits" });
  store.setFields({ positionsText: "53.1 -28.1 a\n53.2 -28.2 b" });
}

function storeWith(run: RunFake = vi.fn<RunFake>(), cancel = vi.fn(async () => true)) {
  return { store: new ForcedPhotometryStore(run, cancel), run, cancel };
}

const paths = (store: ForcedPhotometryStore) => store.getSnapshot().fields.bands.map((b) => b.path);
const G = { name: "g.fits", path: "C:/d/g.fits" };
const R = { name: "r.fits", path: "C:/d/r.fits" };

describe("ForcedPhotometryStore", () => {
  it("starts empty with the default fields and one snapshot object", () => {
    const { store } = storeWith();
    expect(store.getSnapshot()).toEqual(EMPTY_FORCED);
    expect(store.getSnapshot()).toBe(store.getSnapshot());
    expect(EMPTY_FORCED.fields).toBe(DEFAULT_FORCED_FIELDS);
    expect(EMPTY_FORCED).toEqual({ fields: DEFAULT_FORCED_FIELDS, running: false, error: null, result: null, resultKey: null, selected: null });
    expect(Object.isFrozen(EMPTY_FORCED)).toBe(true);
  });

  it("sets fields and tells its listeners only when something changed", () => {
    const { store } = storeWith();
    const listener = vi.fn();
    store.subscribe(listener);
    store.setFields({ radius: 2 });
    expect(listener).toHaveBeenCalledTimes(1);
    store.setFields({ radius: 2 });
    expect(listener).toHaveBeenCalledTimes(1);
    store.setFields({ radiusUnit: "px", annulusInner: 5 });
    expect(listener).toHaveBeenCalledTimes(2);
    expect(store.getSnapshot().fields.radius).toBe(2);
    expect(store.getSnapshot().fields.radiusUnit).toBe("px");
    expect(store.getSnapshot().fields.annulusInner).toBe(5);
  });

  it("adds bands once, edits them and removes them in order", () => {
    const { store } = storeWith();
    store.addBand(G);
    store.addBand(R);
    store.addBand(G);
    expect(paths(store)).toEqual(["C:/d/g.fits", "C:/d/r.fits"]);
    store.updateBand("C:/d/g.fits", { label: "g", fCorr: 1.6 });
    expect(store.getSnapshot().fields.bands).toEqual([
      { path: "C:/d/g.fits", fileName: "g.fits", label: "g", dataset: "", fCorr: 1.6 },
      { path: "C:/d/r.fits", fileName: "r.fits", label: "", dataset: "", fCorr: null },
    ]);
    const listener = vi.fn();
    store.subscribe(listener);
    store.updateBand("C:/d/x.fits", { label: "x" });
    store.removeBand("C:/d/x.fits");
    store.updateBand("C:/d/g.fits", { label: "g" });
    expect(listener).not.toHaveBeenCalled();
    store.removeBand("C:/d/g.fits");
    expect(paths(store)).toEqual(["C:/d/r.fits"]);
  });

  it("a run without a request shows the first blocker and calls nothing", async () => {
    const { store, run } = storeWith();
    await store.run();
    expect(store.getSnapshot().error).toBe("Choose at least one band image.");
    expect(run).not.toHaveBeenCalled();
    expect(store.getSnapshot().running).toBe(false);
  });

  it("a run sends the request, then keeps the result with the key of that request", async () => {
    const answer = deferred<ForcedPhotometryResult>();
    const run = vi.fn<RunFake>(() => answer.promise);
    const { store } = storeWith(run);
    ready(store);
    const plan = forcedRequest(store.getSnapshot().fields);
    const running = store.run();
    expect(store.getSnapshot().running).toBe(true);
    expect(store.getSnapshot().error).toBeNull();
    const measured = result(2);
    answer.resolve(measured);
    await running;
    expect(run).toHaveBeenCalledTimes(1);
    expect(run.mock.calls[0][0]).toEqual(plan.request);
    const after = store.getSnapshot();
    expect(after.result).toBe(measured);
    expect(after.resultKey).toBe(requestKey(plan.request!));
    expect(after.selected).toBeNull();
    expect(after.running).toBe(false);
  });

  it("a failed run keeps the previous result and shows the message", async () => {
    const run = vi.fn<RunFake>();
    const { store } = storeWith(run);
    ready(store);
    const first = result(2);
    run.mockResolvedValueOnce(first);
    await store.run();
    store.select(1);
    const key = store.getSnapshot().resultKey;
    const message = "no band could be measured: r.fits has no celestial WCS: sky positions cannot be placed on it";
    run.mockRejectedValueOnce(new Error(message));
    await store.run();
    const after = store.getSnapshot();
    expect(after.error).toBe(message);
    expect(after.result).toBe(first);
    expect(after.resultKey).toBe(key);
    expect(after.selected).toBe(1);
    expect(after.running).toBe(false);
    run.mockRejectedValueOnce("Operation cancelled");
    await store.run();
    expect(store.getSnapshot().error).toBe("Operation cancelled");
  });

  it("a second run while one is in flight is ignored", async () => {
    const run = vi.fn<RunFake>(() => new Promise<ForcedPhotometryResult>(() => {}));
    const { store } = storeWith(run);
    ready(store);
    void store.run();
    await store.run();
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("cancel reaches the backend only while a run is in flight", async () => {
    const run = vi.fn<RunFake>(() => new Promise<ForcedPhotometryResult>(() => {}));
    const cancel = vi.fn(async () => true);
    const { store } = storeWith(run, cancel);
    await store.cancel();
    expect(cancel).not.toHaveBeenCalled();
    ready(store);
    void store.run();
    await store.cancel();
    expect(cancel).toHaveBeenCalledTimes(1);

    const failingCancel = vi.fn(async (): Promise<boolean> => {
      throw new Error("no progress handle");
    });
    const other = new ForcedPhotometryStore(run, failingCancel);
    ready(other);
    void other.run();
    await expect(other.cancel()).resolves.toBeUndefined();
    expect(failingCancel).toHaveBeenCalledTimes(1);
  });

  it("select accepts only rows of the result", async () => {
    const { store } = storeWith(vi.fn<RunFake>(async () => result(2)));
    store.select(0);
    expect(store.getSnapshot().selected).toBeNull();
    ready(store);
    await store.run();
    store.select(1);
    expect(store.getSnapshot().selected).toBe(1);
    store.select(5);
    expect(store.getSnapshot().selected).toBeNull();
    store.select(1);
    store.select(-1);
    expect(store.getSnapshot().selected).toBeNull();
    store.select(1);
    store.select(null);
    expect(store.getSnapshot().selected).toBeNull();
    const listener = vi.fn();
    store.subscribe(listener);
    store.select(null);
    expect(listener).not.toHaveBeenCalled();
  });

  it("positions handed over by another section become editable text", () => {
    const { store } = storeWith();
    store.loadPositions([{ id: "10995300315432397", ra_deg: 53.1387, dec_deg: -28.0854 }]);
    const text = store.getSnapshot().fields.positionsText;
    expect(text).toBe("id,ra,dec\n10995300315432397,53.1387,-28.0854");
    expect(parseForcedPositions(text).positions).toEqual([{ id: "10995300315432397", ra_deg: 53.1387, dec_deg: -28.0854 }]);
  });

  it("reset drops the state and the answer of a run in flight", async () => {
    const answer = deferred<ForcedPhotometryResult>();
    const { store } = storeWith(vi.fn<RunFake>(() => answer.promise));
    ready(store);
    const running = store.run();
    store.reset();
    answer.resolve(result(2));
    await running;
    expect(store.getSnapshot()).toEqual(EMPTY_FORCED);
    expect(store.getSnapshot()).toBe(EMPTY_FORCED);
  });

  it("changing fields keeps the result on screen and makes it stale", async () => {
    const { store } = storeWith(vi.fn<RunFake>(async () => result(2)));
    ready(store);
    await store.run();
    const measured = store.getSnapshot().result;
    expect(isStale(store.getSnapshot().resultKey, forcedRequest(store.getSnapshot().fields))).toBe(false);
    store.setFields({ radius: 2.5 });
    const snapshot = store.getSnapshot();
    expect(snapshot.result).toBe(measured);
    expect(isStale(snapshot.resultKey, forcedRequest(snapshot.fields))).toBe(true);
  });
});
