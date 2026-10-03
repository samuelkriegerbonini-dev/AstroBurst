import { describe, it, expect, vi } from "vitest";
import { EMPTY_STAR_DETECTION, StarDetectionStore, scopedStarDetection } from "../useStarDetectionStore";
import type { StarDetectionResult } from "../../shared/types";

const RESULT: StarDetectionResult = {
  stars: [{ x: 1, y: 2, flux: 3, fwhm: 2.5, snr: 40 }],
  background_median: 10,
  background_sigma: 1,
  image_width: 100,
  image_height: 80,
  elapsed_ms: 12,
  n_detected: 1,
};

const OTHER: StarDetectionResult = { ...RESULT, n_detected: 7 };

describe("StarDetectionStore", () => {
  it("starts empty", () => {
    expect(new StarDetectionStore().getSnapshot()).toBe(EMPTY_STAR_DETECTION);
    expect(EMPTY_STAR_DETECTION).toEqual({ scope: null, result: null, loading: false, error: null });
  });

  it("marks a detection as loading and commits its result", () => {
    const store = new StarDetectionStore();
    const seq = store.begin("image:a");
    expect(store.getSnapshot()).toEqual({ scope: "image:a", result: null, loading: true, error: null });
    store.commit(seq, RESULT);
    expect(store.getSnapshot()).toEqual({ scope: "image:a", result: RESULT, loading: false, error: null });
  });

  it("records a failure and clears it on the next detection", () => {
    const store = new StarDetectionStore();
    const seq = store.begin("image:a");
    store.fail(seq, "boom");
    expect(store.getSnapshot()).toEqual({ scope: "image:a", result: null, loading: false, error: "boom" });
    store.begin("image:a");
    expect(store.getSnapshot().error).toBeNull();
    expect(store.getSnapshot().loading).toBe(true);
  });

  it("ignores a commit or failure from a detection superseded by a newer one", () => {
    const store = new StarDetectionStore();
    const first = store.begin("image:a");
    const second = store.begin("image:a");
    store.commit(first, OTHER);
    store.fail(first, "stale");
    expect(store.getSnapshot()).toEqual({ scope: "image:a", result: null, loading: true, error: null });
    store.commit(second, RESULT);
    expect(store.getSnapshot().result).toBe(RESULT);
  });

  it("keeps the result of a same-scope detection when a new one starts", () => {
    const store = new StarDetectionStore();
    store.commit(store.begin("image:a"), RESULT);
    store.begin("image:a");
    expect(store.getSnapshot().result).toBe(RESULT);
  });

  it("clears everything and invalidates an in-flight detection when the scope changes", () => {
    const store = new StarDetectionStore();
    store.commit(store.begin("image:a"), RESULT);
    const inFlight = store.begin("image:a");
    store.syncScope("image:b");
    expect(store.getSnapshot()).toEqual({ scope: "image:b", result: null, loading: false, error: null });
    store.commit(inFlight, OTHER);
    expect(store.getSnapshot().result).toBeNull();
  });

  it("clears the previous scope when a detection begins on another one", () => {
    const store = new StarDetectionStore();
    const old = store.begin("image:a");
    store.commit(old, RESULT);
    store.begin("image:b");
    expect(store.getSnapshot()).toEqual({ scope: "image:b", result: null, loading: true, error: null });
  });

  it("does nothing when the scope is unchanged", () => {
    const store = new StarDetectionStore();
    const seq = store.begin("image:a");
    const before = store.getSnapshot();
    const listener = vi.fn();
    store.subscribe(listener);
    store.syncScope("image:a");
    expect(store.getSnapshot()).toBe(before);
    expect(listener).not.toHaveBeenCalled();
    store.commit(seq, RESULT);
    expect(store.getSnapshot().result).toBe(RESULT);
  });

  it("resets to empty and drops an in-flight detection", () => {
    const store = new StarDetectionStore();
    const seq = store.begin("image:a");
    store.reset();
    expect(store.getSnapshot()).toBe(EMPTY_STAR_DETECTION);
    store.commit(seq, RESULT);
    expect(store.getSnapshot()).toBe(EMPTY_STAR_DETECTION);
  });

  it("keeps the state while at least one consumer retains it", () => {
    const store = new StarDetectionStore();
    const releaseA = store.retain();
    const releaseB = store.retain();
    store.commit(store.begin("image:a"), RESULT);
    releaseA();
    expect(store.getSnapshot().result).toBe(RESULT);
    releaseB();
    expect(store.getSnapshot()).toBe(EMPTY_STAR_DETECTION);
  });

  it("drops a detection that finishes after the last consumer released the store", () => {
    const store = new StarDetectionStore();
    const release = store.retain();
    const seq = store.begin("image:a");
    release();
    expect(store.getSnapshot()).toBe(EMPTY_STAR_DETECTION);
    store.commit(seq, RESULT);
    store.fail(seq, "late");
    expect(store.getSnapshot()).toBe(EMPTY_STAR_DETECTION);
  });

  it("survives the StrictMode retain, release, retain sequence", () => {
    const store = new StarDetectionStore();
    store.retain()();
    const release = store.retain();
    store.commit(store.begin("image:a"), RESULT);
    expect(store.getSnapshot().result).toBe(RESULT);
    release();
    expect(store.getSnapshot()).toBe(EMPTY_STAR_DETECTION);
  });

  it("ignores a release called twice", () => {
    const store = new StarDetectionStore();
    const releaseA = store.retain();
    const releaseB = store.retain();
    releaseA();
    releaseA();
    store.commit(store.begin("image:a"), RESULT);
    expect(store.getSnapshot().result).toBe(RESULT);
    releaseB();
    expect(store.getSnapshot()).toBe(EMPTY_STAR_DETECTION);
  });

  it("works with subscribe and getSnapshot detached from the store", () => {
    const store = new StarDetectionStore();
    const { subscribe, getSnapshot } = store;
    const listener = vi.fn();
    const unsubscribe = subscribe(listener);
    store.begin("image:a");
    expect(listener).toHaveBeenCalledTimes(1);
    expect(getSnapshot()).toEqual({ scope: "image:a", result: null, loading: true, error: null });
    unsubscribe();
    store.reset();
    expect(listener).toHaveBeenCalledTimes(1);
    expect(getSnapshot()).toBe(EMPTY_STAR_DETECTION);
  });
});

describe("scopedStarDetection", () => {
  it("returns the snapshot for its own scope and the empty state for any other", () => {
    const store = new StarDetectionStore();
    store.commit(store.begin("image:a"), RESULT);
    const snapshot = store.getSnapshot();
    expect(scopedStarDetection(snapshot, "image:a")).toBe(snapshot);
    expect(scopedStarDetection(snapshot, "image:b")).toBe(EMPTY_STAR_DETECTION);
    expect(scopedStarDetection(EMPTY_STAR_DETECTION, "image:a")).toBe(EMPTY_STAR_DETECTION);
  });
});
