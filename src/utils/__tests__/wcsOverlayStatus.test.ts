import { describe, expect, it, vi } from "vitest";
import { WcsOverlayStatusStore, wcsOverlayNote } from "../wcsOverlayStatus";

describe("WcsOverlayStatusStore", () => {
  it("publishes the last grid and compass error and notifies only on a change", () => {
    const store = new WcsOverlayStatusStore();
    const listener = vi.fn();
    store.subscribe(listener);
    expect(store.get()).toEqual({ grid: null, compass: null });

    store.set("grid", "no celestial WCS in the header");
    expect(store.get()).toEqual({ grid: "no celestial WCS in the header", compass: null });
    expect(listener).toHaveBeenCalledTimes(1);

    store.set("grid", "no celestial WCS in the header");
    expect(listener).toHaveBeenCalledTimes(1);

    store.set("grid", null);
    expect(store.get().grid).toBeNull();
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("stops notifying after unsubscribe", () => {
    const store = new WcsOverlayStatusStore();
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);
    unsubscribe();
    store.set("compass", "the WCS has no usable orientation");
    expect(listener).not.toHaveBeenCalled();
  });
});

describe("wcsOverlayNote", () => {
  it("is empty when every ticked overlay drew", () => {
    expect(wcsOverlayNote({ grid: true, compass: true }, { grid: null, compass: null })).toBeNull();
  });

  it("ignores an error of an overlay that is not ticked", () => {
    expect(wcsOverlayNote({ grid: false, compass: false }, { grid: "no WCS", compass: "no WCS" })).toBeNull();
  });

  it("says no WCS and names the reason of each ticked overlay that failed", () => {
    expect(wcsOverlayNote({ grid: true, compass: false }, { grid: "no celestial WCS", compass: null })).toEqual({
      text: "no WCS",
      title: "grid: no celestial WCS",
    });
    expect(
      wcsOverlayNote({ grid: true, compass: true }, { grid: "no celestial WCS", compass: "the WCS has no usable orientation" }),
    ).toEqual({
      text: "no WCS",
      title: "grid: no celestial WCS; compass: the WCS has no usable orientation",
    });
  });
});
