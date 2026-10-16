import { describe, expect, it, vi } from "vitest";
import { createArchiveStore } from "../state/archiveStore";
import { createInflight } from "../state/inflight";
import { createInitialState } from "../state/archiveReducer";
import { MISSIONS } from "./fixtures";

describe("archive store", () => {
  it("applies actions through the reducer and notifies subscribers", () => {
    const store = createArchiveStore();
    const listener = vi.fn();
    const off = store.subscribe(listener);
    store.dispatch({ type: "missionsLoaded", missions: MISSIONS });
    expect(store.getState().missions).toHaveLength(10);
    expect(listener).toHaveBeenCalledTimes(1);
    off();
    store.dispatch({ type: "toast", text: "x" });
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("keeps the same state object and stays quiet when an action changes nothing", () => {
    const store = createArchiveStore(createInitialState());
    const before = store.getState();
    const listener = vi.fn();
    store.subscribe(listener);
    store.dispatch({ type: "toastDismissed" });
    expect(store.getState()).toBe(before);
    expect(listener).not.toHaveBeenCalled();
  });

  it("lets a listener dispatch without losing later notifications", () => {
    const store = createArchiveStore();
    const seen: number[] = [];
    store.subscribe(() => {
      seen.push(store.getState().toastSeq);
      if (store.getState().toastSeq === 1) store.dispatch({ type: "toast", text: "second" });
    });
    store.dispatch({ type: "toast", text: "first" });
    expect(store.getState().toastSeq).toBe(2);
    expect(seen).toEqual([1, 2]);
  });
});

describe("inflight counter", () => {
  it("counts every request and thumbnails separately", () => {
    const counter = createInflight();
    const a = counter.begin("other");
    const b = counter.begin("thumb");
    const c = counter.begin("thumb");
    expect(counter.snapshot()).toEqual({ total: 3, thumbs: 2 });
    b();
    b();
    expect(counter.snapshot()).toEqual({ total: 2, thumbs: 1 });
    a();
    c();
    expect(counter.snapshot()).toEqual({ total: 0, thumbs: 0 });
  });

  it("returns the same snapshot object until the counts change", () => {
    const counter = createInflight();
    const first = counter.snapshot();
    expect(counter.snapshot()).toBe(first);
    const listener = vi.fn();
    counter.subscribe(listener);
    const end = counter.begin("other");
    expect(counter.snapshot()).not.toBe(first);
    end();
    expect(listener).toHaveBeenCalledTimes(2);
  });
});
