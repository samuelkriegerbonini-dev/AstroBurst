import { describe, it, expect, vi, afterEach } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DEFAULT_DOCK_LAYOUT, dockLayoutErrors, dockReducer, type DockLayout } from "../dockLayout";
import {
  DOCK_LAYOUT_STORAGE_KEY,
  DOCK_LAYOUT_VERSION,
  LEGACY_SIZE_KEYS,
  migrateLegacySizes,
  parseDockLayout,
  serializeDockLayout,
} from "../dockPersistence";

function persisted(patch: Record<string, unknown> = {}): Record<string, unknown> {
  return { ...(JSON.parse(serializeDockLayout(DEFAULT_DOCK_LAYOUT)) as Record<string, unknown>), ...patch };
}

const CUSTOM_FALLBACK: DockLayout = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "toggle", tool: "headers" });

describe("parseDockLayout", () => {
  it("returns the fallback for garbage, other versions and non-object anchors", () => {
    const garbage: unknown[] = [
      null, undefined, 42, "layout", [], {}, true,
      persisted({ version: 2 }), persisted({ version: "1" }), persisted({ version: undefined }),
      persisted({ anchors: [] }), persisted({ anchors: "left-top" }), persisted({ anchors: null }),
    ];
    for (const raw of garbage) {
      expect(parseDockLayout(raw)).toBe(DEFAULT_DOCK_LAYOUT);
      expect(parseDockLayout(raw, CUSTOM_FALLBACK)).toBe(CUSTOM_FALLBACK);
    }
  });

  it("drops unknown ids", () => {
    const layout = parseDockLayout(persisted({
      anchors: { ...DEFAULT_DOCK_LAYOUT.anchors, "left-top": ["files", "bogus", 7, null, { id: "info" }, "info"] },
    }));
    expect(layout.anchors["left-top"]).toEqual(["files", "info"]);
    expect(dockLayoutErrors(layout)).toEqual([]);
  });

  it("keeps the first occurrence of a duplicated id across anchors", () => {
    const layout = parseDockLayout(persisted({
      anchors: {
        "left-top": ["files", "analysis", "info", "analysis"],
        "left-bottom": ["compose"],
        "right-top": ["headers", "analysis", "processing", "stacking"],
        "right-bottom": ["synth", "export", "config", "files"],
      },
    }));
    expect(layout.anchors["left-top"]).toEqual(["files", "image", "astrometry", "photometry", "cube", "info"]);
    expect(layout.anchors["right-top"]).toEqual(["headers", "processing", "stacking"]);
    expect(layout.anchors["right-bottom"]).toEqual(["synth", "export", "config", "log"]);
  });

  it("appends missing tools to their default anchor in default order", () => {
    const layout = parseDockLayout(persisted({ anchors: { "left-top": ["info"], "right-top": ["stacking", 3] }, active: {} }));
    expect(layout.anchors).toEqual({
      "left-top": ["info", "files"],
      "left-bottom": ["compose"],
      "right-top": ["stacking", "headers", "image", "astrometry", "photometry", "cube", "processing"],
      "right-bottom": ["synth", "export", "config", "log"],
    });
    expect(dockLayoutErrors(layout)).toEqual([]);
  });

  it("keeps a tool docked in a non-default anchor and lets other anchors be empty", () => {
    const all = ["files", "info", "compose", "headers", "image", "astrometry", "photometry", "cube", "processing", "stacking", "synth", "export", "config", "log"];
    const layout = parseDockLayout(persisted({ anchors: { "right-bottom": all }, active: { "right-bottom": "export" } }));
    expect(layout.anchors).toEqual({ "left-top": [], "left-bottom": [], "right-top": [], "right-bottom": all });
    expect(layout.active).toEqual({ "left-top": null, "left-bottom": null, "right-top": null, "right-bottom": "export" });
  });

  it("clears an open tool that is not docked in that anchor", () => {
    const layout = parseDockLayout(persisted({
      active: { "left-top": "analysis", "left-bottom": "bogus", "right-top": "processing", "right-bottom": 5 },
    }));
    expect(layout.active).toEqual({ "left-top": null, "left-bottom": null, "right-top": "processing", "right-bottom": null });
    expect(parseDockLayout(persisted({ active: "files" })).active).toEqual({ "left-top": null, "left-bottom": null, "right-top": null, "right-bottom": null });
  });

  it("clamps finite sizes and replaces anything else with the default", () => {
    expect(parseDockLayout(persisted({ sizes: { leftW: 9999, rightW: "400", bottomH: null, bottomSplit: 0.1 } })).sizes)
      .toEqual({ leftW: 640, rightW: 380, bottomH: 280, bottomSplit: 0.25 });
    expect(parseDockLayout(persisted({ sizes: { leftW: Number.NaN, rightW: Infinity, bottomH: 140.4, bottomSplit: 0.6666 } })).sizes)
      .toEqual({ leftW: 300, rightW: 380, bottomH: 140, bottomSplit: 0.667 });
    expect(parseDockLayout(persisted({ sizes: undefined })).sizes).toEqual(DEFAULT_DOCK_LAYOUT.sizes);
  });

  it("round-trips through serializeDockLayout", () => {
    let layout = DEFAULT_DOCK_LAYOUT;
    layout = dockReducer(layout, { type: "move", tool: "image", anchor: "left-bottom", index: 0 });
    layout = dockReducer(layout, { type: "move", tool: "info", anchor: "right-top" });
    layout = dockReducer(layout, { type: "toggle", tool: "export" });
    layout = dockReducer(layout, { type: "resize", key: "bottomSplit", value: 0.375 });
    layout = dockReducer(layout, { type: "resize", key: "leftW", value: 512 });
    const text = serializeDockLayout(layout);
    expect(JSON.parse(text)).toEqual({ version: DOCK_LAYOUT_VERSION, anchors: layout.anchors, active: layout.active, sizes: layout.sizes });
    expect(parseDockLayout(JSON.parse(text))).toEqual(layout);
  });
});

const OLD_ANCHORS = {
  "left-top": ["files", "info"],
  "left-bottom": ["compose"],
  "right-top": ["headers", "analysis", "processing", "stacking"],
  "right-bottom": ["synth", "export", "config"],
};

const OLD_ACTIVE = { "left-top": "files", "left-bottom": "compose", "right-top": null, "right-bottom": null };

function oldBlob(anchors: Record<string, unknown> = {}, active: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    version: 1,
    anchors: { ...OLD_ANCHORS, ...anchors },
    active: { ...OLD_ACTIVE, ...active },
    sizes: { leftW: 320, rightW: 420, bottomH: 260, bottomSplit: 0.4 },
  };
}

describe("legacy analysis tool", () => {
  it("replaces Analysis by the four analysis tools at its slot, opens Image in its place and docks Log bottom right", () => {
    const layout = parseDockLayout(oldBlob({}, { "right-top": "analysis" }));
    expect(layout.anchors["right-top"]).toEqual(["headers", "image", "astrometry", "photometry", "cube", "processing", "stacking"]);
    expect(layout.active["right-top"]).toBe("image");
    expect(layout.anchors["right-bottom"]).toEqual(["synth", "export", "config", "log"]);
    expect(layout.anchors["left-top"]).toEqual(["files", "info"]);
    expect(layout.anchors["left-bottom"]).toEqual(["compose"]);
    expect(layout.sizes).toEqual({ leftW: 320, rightW: 420, bottomH: 260, bottomSplit: 0.4 });
    expect(dockLayoutErrors(layout)).toEqual([]);
  });

  it("expands Analysis where the user had moved it", () => {
    const layout = parseDockLayout(oldBlob({ "left-bottom": ["analysis", "compose"], "right-top": ["headers", "processing", "stacking"] }));
    expect(layout.anchors["left-bottom"]).toEqual(["image", "astrometry", "photometry", "cube", "compose"]);
    expect(layout.anchors["right-top"]).toEqual(["headers", "processing", "stacking"]);
    expect(layout.active["right-top"]).toBeNull();
  });

  it("keeps an analysis tool already docked elsewhere and expands only the others", () => {
    const layout = parseDockLayout(oldBlob({ "left-top": ["files", "info", "image"] }));
    expect(layout.anchors["left-top"]).toEqual(["files", "info", "image"]);
    expect(layout.anchors["right-top"]).toEqual(["headers", "astrometry", "photometry", "cube", "processing", "stacking"]);
    expect(dockLayoutErrors(layout)).toEqual([]);
  });

  it("closes an anchor whose open Analysis was docked in another anchor", () => {
    const layout = parseDockLayout(oldBlob({ "left-bottom": ["analysis", "compose"], "right-top": ["headers", "processing", "stacking"] }, { "right-top": "analysis" }));
    expect(layout.active["right-top"]).toBeNull();
    expect(layout.active["left-bottom"]).toBe("compose");
  });

  it("migrates the layout of the validation checklist", () => {
    const layout = parseDockLayout(oldBlob(
      { "left-bottom": ["analysis", "compose"], "right-top": ["headers", "processing", "stacking"] },
      { "left-bottom": "analysis" },
    ));
    expect(layout.anchors["left-bottom"]).toEqual(["image", "astrometry", "photometry", "cube", "compose"]);
    expect(layout.active["left-bottom"]).toBe("image");
    expect(layout.anchors["right-bottom"]).toEqual(["synth", "export", "config", "log"]);
    expect(serializeDockLayout(layout)).not.toContain("analysis");
  });

  it("is idempotent through a save and a reload", () => {
    for (const raw of [
      oldBlob({}, { "right-top": "analysis" }),
      oldBlob({ "left-bottom": ["analysis", "compose"], "right-top": ["headers", "processing", "stacking"] }, { "left-bottom": "analysis" }),
      oldBlob({ "left-top": ["files", "info", "image"] }, { "right-top": "analysis" }),
    ]) {
      const once = parseDockLayout(raw);
      expect(parseDockLayout(JSON.parse(serializeDockLayout(once)))).toEqual(once);
    }
  });

  it("leaves a layout without Analysis alone", () => {
    const layout = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "cube", anchor: "left-top", index: 0 });
    expect(parseDockLayout(JSON.parse(serializeDockLayout(layout)))).toEqual(layout);
  });
});

describe("migrateLegacySizes", () => {
  const read = (values: Record<string, string>) => (key: string) => values[key] ?? null;

  it("reads the sidebar, right column and bottom panel sizes", () => {
    expect(LEGACY_SIZE_KEYS).toEqual(["ab.layout.sidebarW", "ab.layout.rightW", "ab.layout.bottomH"]);
    expect(migrateLegacySizes(read({ "ab.layout.sidebarW": "250", "ab.layout.rightW": "500", "ab.layout.bottomH": "320" })))
      .toEqual({ leftW: 250, rightW: 500, bottomH: 320 });
  });

  it("clamps to the new limits", () => {
    expect(migrateLegacySizes(read({ "ab.layout.sidebarW": "50", "ab.layout.rightW": "100", "ab.layout.bottomH": "9999" })))
      .toEqual({ leftW: 180, rightW: 280, bottomH: 600 });
  });

  it("ignores missing and junk values", () => {
    expect(migrateLegacySizes(read({}))).toEqual({});
    expect(migrateLegacySizes(read({ "ab.layout.sidebarW": "abc", "ab.layout.rightW": "", "ab.layout.bottomH": "Infinity" }))).toEqual({});
    expect(migrateLegacySizes(read({ "ab.layout.sidebarW": "  ", "ab.layout.bottomH": "300px", "ab.layout.rightW": "420" }))).toEqual({ rightW: 420 });
  });
});

function memoryStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => { map.set(k, v); },
    removeItem: (k: string) => { map.delete(k); },
    map,
  };
}

async function freshStore(storage: object) {
  vi.stubGlobal("localStorage", storage);
  vi.resetModules();
  return import("../../hooks/useDockLayout");
}

function storedLayout(storage: ReturnType<typeof memoryStorage>): DockLayout {
  return parseDockLayout(JSON.parse(storage.map.get(DOCK_LAYOUT_STORAGE_KEY) ?? "null"));
}

describe("dockStore", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("starts from the default layout when nothing is stored", async () => {
    const { dockStore } = await freshStore(memoryStorage());
    expect(dockStore.get()).toEqual(DEFAULT_DOCK_LAYOUT);
  });

  it("loads the stored layout", async () => {
    const layout = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "image", anchor: "left-bottom" });
    const { dockStore } = await freshStore(memoryStorage({ [DOCK_LAYOUT_STORAGE_KEY]: serializeDockLayout(layout) }));
    expect(dockStore.get()).toEqual(layout);
  });

  it("merges the legacy sizes only when the dock key is absent", async () => {
    const legacy = { "ab.layout.sidebarW": "260", "ab.layout.bottomH": "350" };
    const first = await freshStore(memoryStorage(legacy));
    expect(first.dockStore.get().sizes).toEqual({ leftW: 260, rightW: 380, bottomH: 350, bottomSplit: 0.5 });
    const second = await freshStore(memoryStorage({ ...legacy, [DOCK_LAYOUT_STORAGE_KEY]: serializeDockLayout(DEFAULT_DOCK_LAYOUT) }));
    expect(second.dockStore.get().sizes).toEqual(DEFAULT_DOCK_LAYOUT.sizes);
  });

  it("falls back to the default layout for corrupted JSON", async () => {
    const { dockStore } = await freshStore(memoryStorage({ [DOCK_LAYOUT_STORAGE_KEY]: "{not json" }));
    expect(dockStore.get()).toEqual(DEFAULT_DOCK_LAYOUT);
  });

  it("writes synchronously and notifies only when the layout changes", async () => {
    const storage = memoryStorage();
    const { dockStore } = await freshStore(storage);
    const listener = vi.fn();
    const unsubscribe = dockStore.subscribe(listener);
    dockStore.dispatch({ type: "open", tool: "files" });
    expect(listener).not.toHaveBeenCalled();
    expect(storage.map.has(DOCK_LAYOUT_STORAGE_KEY)).toBe(false);
    dockStore.dispatch({ type: "toggle", tool: "image" });
    expect(listener).toHaveBeenCalledTimes(1);
    expect(storedLayout(storage).active["right-top"]).toBe("image");
    unsubscribe();
    dockStore.dispatch({ type: "toggle", tool: "image" });
    expect(listener).toHaveBeenCalledTimes(1);
    expect(storedLayout(storage).active["right-top"]).toBeNull();
  });

  it("keeps clamp corrections out of storage, also when a later action writes the layout", async () => {
    const storage = memoryStorage();
    const { dockStore } = await freshStore(storage);
    dockStore.dispatch({ type: "resize", key: "rightW", value: 520 });
    expect(storedLayout(storage).sizes.rightW).toBe(520);
    dockStore.dispatch({ type: "resize", key: "rightW", value: 300, persist: false });
    expect(dockStore.get().sizes.rightW).toBe(300);
    expect(storedLayout(storage).sizes.rightW).toBe(520);
    dockStore.dispatch({ type: "toggle", tool: "headers" });
    expect(dockStore.get().sizes.rightW).toBe(300);
    expect(storedLayout(storage).sizes.rightW).toBe(520);
    expect(storedLayout(storage).active["right-top"]).toBe("headers");
  });

  it("persists a handle drag to the clamped value even when the store already holds it", async () => {
    const storage = memoryStorage();
    const { dockStore } = await freshStore(storage);
    dockStore.dispatch({ type: "resize", key: "bottomH", value: 400 });
    dockStore.dispatch({ type: "resize", key: "bottomH", value: 220, persist: false });
    const listener = vi.fn();
    dockStore.subscribe(listener);
    dockStore.dispatch({ type: "resize", key: "bottomH", value: 220 });
    expect(listener).not.toHaveBeenCalled();
    expect(storedLayout(storage).sizes.bottomH).toBe(220);
  });

  it("resetAll restores the default and removes only the dock and legacy keys", async () => {
    const storage = memoryStorage({
      "ab.layout.sidebarW": "260",
      "ab.layout.rightW": "500",
      "ab.layout.bottomH": "350",
      "astroburst.readout.v1": "{\"x\":1}",
      "ab.gpu": "true",
    });
    const { dockStore } = await freshStore(storage);
    dockStore.dispatch({ type: "move", tool: "files", anchor: "right-top" });
    dockStore.dispatch({ type: "resize", key: "leftW", value: 600, persist: false });
    expect(storage.map.has(DOCK_LAYOUT_STORAGE_KEY)).toBe(true);
    dockStore.resetAll();
    expect(dockStore.get()).toEqual(DEFAULT_DOCK_LAYOUT);
    expect([...storage.map.keys()].sort()).toEqual(["ab.gpu", "astroburst.readout.v1"]);
    dockStore.dispatch({ type: "toggle", tool: "info" });
    expect(storedLayout(storage).sizes).toEqual(DEFAULT_DOCK_LAYOUT.sizes);
  });

  it("works when storage throws", async () => {
    const throwing = {
      getItem: () => { throw new Error("blocked"); },
      setItem: () => { throw new Error("blocked"); },
      removeItem: () => { throw new Error("blocked"); },
    };
    const { dockStore } = await freshStore(throwing);
    expect(dockStore.get()).toEqual(DEFAULT_DOCK_LAYOUT);
    expect(() => dockStore.dispatch({ type: "toggle", tool: "headers" })).not.toThrow();
    expect(dockStore.get().active["right-top"]).toBe("headers");
    expect(() => dockStore.resetAll()).not.toThrow();
    expect(dockStore.get()).toEqual(DEFAULT_DOCK_LAYOUT);
  });
});

async function freshGpuStore() {
  vi.resetModules();
  return import("../../hooks/useGpuDisplay");
}

describe("gpuDisplayStore", () => {
  it("starts off and notifies only when the value changes", async () => {
    const { gpuDisplayStore } = await freshGpuStore();
    expect(gpuDisplayStore.get()).toBe(false);
    const listener = vi.fn();
    const unsubscribe = gpuDisplayStore.subscribe(listener);
    gpuDisplayStore.set(false);
    expect(listener).not.toHaveBeenCalled();
    gpuDisplayStore.set(true);
    expect(gpuDisplayStore.get()).toBe(true);
    expect(listener).toHaveBeenCalledTimes(1);
    gpuDisplayStore.set(true);
    expect(listener).toHaveBeenCalledTimes(1);
    gpuDisplayStore.set(false);
    expect(gpuDisplayStore.get()).toBe(false);
    expect(listener).toHaveBeenCalledTimes(2);
    unsubscribe();
    gpuDisplayStore.set(true);
    expect(listener).toHaveBeenCalledTimes(2);
    expect(gpuDisplayStore.get()).toBe(true);
  });
});

describe("useDockLayout and useGpuDisplay", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("render on the server from the current store values", async () => {
    const { dockStore, useDockLayout } = await freshStore(memoryStorage());
    const { gpuDisplayStore, useGpuDisplay } = await import("../../hooks/useGpuDisplay");
    dockStore.dispatch({ type: "toggle", tool: "image" });
    gpuDisplayStore.set(true);
    function Probe() {
      const layout = useDockLayout();
      const gpu = useGpuDisplay();
      return createElement("span", null, `${layout.active["right-top"]}|${String(gpu)}`);
    }
    expect(renderToStaticMarkup(createElement(Probe))).toBe("<span>image|true</span>");
  });
});
