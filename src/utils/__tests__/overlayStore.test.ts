import { describe, it, expect, vi } from "vitest";
import { OverlayStoreCore, EMPTY_OVERLAY_DOC, type OverlayLayer } from "../overlayStore";

function layer(id: string, kind = "grid"): OverlayLayer {
  return { id, kind, visible: true, paint: vi.fn() };
}

describe("OverlayStoreCore", () => {
  it("isolates layers per fileKey and returns a stable doc reference", () => {
    const store = new OverlayStoreCore();
    store.add("a", layer("grid"));
    expect(store.getDoc("a").layers).toHaveLength(1);
    expect(store.getDoc("b").layers).toHaveLength(0);
    expect(store.getDoc("b")).toBe(store.getDoc("b"));
    expect(store.getDoc("b")).toBe(EMPTY_OVERLAY_DOC);
    expect(EMPTY_OVERLAY_DOC.layers).toEqual([]);
    expect(store.has("a", "grid")).toBe(true);
    expect(store.has("b", "grid")).toBe(false);
  });

  it("adds, replaces by id in place and removes layers, notifying once per change", () => {
    const store = new OverlayStoreCore();
    const listener = vi.fn();
    store.subscribe(listener);

    const first = layer("grid");
    store.add("f", first);
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.getDoc("f").version).toBe(1);

    store.add("f", layer("catalog", "catalog"));
    expect(listener).toHaveBeenCalledTimes(2);
    expect(store.getDoc("f").layers.map((l) => l.id)).toEqual(["grid", "catalog"]);

    const replacement = layer("grid");
    store.add("f", replacement);
    expect(listener).toHaveBeenCalledTimes(3);
    expect(store.getDoc("f").layers.map((l) => l.id)).toEqual(["grid", "catalog"]);
    expect(store.getDoc("f").layers[0].paint).toBe(replacement.paint);

    store.remove("f", "missing");
    expect(listener).toHaveBeenCalledTimes(3);
    store.remove("f", "grid");
    expect(listener).toHaveBeenCalledTimes(4);
    expect(store.getDoc("f").layers.map((l) => l.id)).toEqual(["catalog"]);
    expect(store.getDoc("f").version).toBe(4);
  });

  it("toggles visibility per layer without touching other files", () => {
    const store = new OverlayStoreCore();
    store.add("a", layer("grid"));
    store.add("b", layer("grid"));
    const listener = vi.fn();
    store.subscribe(listener);

    store.setVisible("a", "grid", false);
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.getDoc("a").layers[0].visible).toBe(false);
    expect(store.getDoc("b").layers[0].visible).toBe(true);

    store.setVisible("a", "grid", false);
    expect(listener).toHaveBeenCalledTimes(1);
    store.setVisible("a", "missing", false);
    expect(listener).toHaveBeenCalledTimes(1);

    store.setVisible("a", "grid", true);
    expect(store.getDoc("a").layers[0].visible).toBe(true);
    expect(store.visibleLayers("a")).toHaveLength(1);
    store.setVisible("a", "grid", false);
    expect(store.visibleLayers("a")).toHaveLength(0);
  });

  it("clears every layer of one file and unsubscribes listeners", () => {
    const store = new OverlayStoreCore();
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);
    store.add("a", layer("grid"));
    store.add("a", layer("catalog", "catalog"));
    store.add("b", layer("grid"));
    store.clear("a");
    expect(store.getDoc("a").layers).toEqual([]);
    expect(store.getDoc("b").layers).toHaveLength(1);
    expect(listener).toHaveBeenCalledTimes(4);
    store.clear("a");
    expect(listener).toHaveBeenCalledTimes(4);
    unsubscribe();
    store.add("a", layer("grid"));
    expect(listener).toHaveBeenCalledTimes(4);
  });

  it("does not persist anything between store instances", () => {
    const store = new OverlayStoreCore();
    store.add("a", layer("grid"));
    expect(new OverlayStoreCore().getDoc("a").layers).toEqual([]);
  });
});
