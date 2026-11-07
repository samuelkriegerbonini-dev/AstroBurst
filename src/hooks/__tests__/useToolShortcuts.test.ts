import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createToolShortcutsStore, toolShortcutsStore, useToolShortcuts } from "../useToolShortcuts";
import { dockStore } from "../useDockLayout";
import { DOCK_TOOL_IDS } from "../../utils/dockLayout";
import { DOCK_LAYOUT_STORAGE_KEY } from "../../utils/dockPersistence";
import { DEFAULT_TOOL_SHORTCUTS, TOOL_SHORTCUTS_KEY } from "../../utils/toolCatalog";

function memoryStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  return {
    map,
    getItem: (key: string) => map.get(key) ?? null,
    setItem: vi.fn((key: string, value: string) => {
      map.set(key, value);
    }),
    removeItem: vi.fn((key: string) => {
      map.delete(key);
    }),
  };
}

type MemoryStorage = ReturnType<typeof memoryStorage>;

function stored(storage: MemoryStorage): unknown {
  return JSON.parse(storage.map.get(TOOL_SHORTCUTS_KEY) ?? "null");
}

const throwingStorage = {
  getItem(): string | null {
    throw new Error("SecurityError");
  },
  setItem(): void {
    throw new Error("QuotaExceededError");
  },
  removeItem(): void {
    throw new Error("SecurityError");
  },
};

describe("tool shortcuts store", () => {
  let storage: MemoryStorage;

  beforeEach(() => {
    storage = memoryStorage();
    vi.stubGlobal("localStorage", storage);
    toolShortcutsStore.reset();
    dockStore.dispatch({ type: "reset" });
  });

  afterEach(() => {
    toolShortcutsStore.reset();
    dockStore.dispatch({ type: "reset" });
    vi.unstubAllGlobals();
  });

  it("persists every toggle under ab.tools.shortcuts.v1 in canonical order and notifies once per change", () => {
    const listener = vi.fn();
    const unsubscribe = toolShortcutsStore.subscribe(listener);
    toolShortcutsStore.toggle("sagui");
    expect(toolShortcutsStore.get()).toEqual(DOCK_TOOL_IDS.filter((tool) => tool === "sagui" || DEFAULT_TOOL_SHORTCUTS.includes(tool)));
    expect(stored(storage)).toEqual(toolShortcutsStore.get());
    toolShortcutsStore.toggle("image");
    expect(toolShortcutsStore.get()).not.toContain("image");
    expect(stored(storage)).toEqual(toolShortcutsStore.get());
    expect(listener).toHaveBeenCalledTimes(2);
    unsubscribe();
    toolShortcutsStore.toggle("image");
    expect(listener).toHaveBeenCalledTimes(2);
    expect(toolShortcutsStore.get()).toContain("image");
  });

  it("reset restores the default shortcuts and saves them", () => {
    toolShortcutsStore.toggle("image");
    toolShortcutsStore.toggle("sagui");
    toolShortcutsStore.reset();
    expect(toolShortcutsStore.get()).toEqual(DEFAULT_TOOL_SHORTCUTS);
    expect(stored(storage)).toEqual(DEFAULT_TOOL_SHORTCUTS);
  });

  it("is reset by dockStore.resetAll, the action behind both Reset layout entry points", () => {
    toolShortcutsStore.toggle("image");
    toolShortcutsStore.toggle("sagui");
    dockStore.resetAll();
    expect(toolShortcutsStore.get()).toEqual(DEFAULT_TOOL_SHORTCUTS);
    expect(stored(storage)).toEqual(DEFAULT_TOOL_SHORTCUTS);
    expect(storage.map.has(DOCK_LAYOUT_STORAGE_KEY)).toBe(false);
  });

  it("keeps the session list when storage throws", () => {
    vi.stubGlobal("localStorage", throwingStorage);
    expect(() => toolShortcutsStore.toggle("sagui")).not.toThrow();
    expect(toolShortcutsStore.get()).toContain("sagui");
    expect(() => toolShortcutsStore.reset()).not.toThrow();
    expect(toolShortcutsStore.get()).toEqual(DEFAULT_TOOL_SHORTCUTS);
  });

  it("renders the current list through useToolShortcuts", () => {
    toolShortcutsStore.toggle("log");
    function Probe() {
      return createElement("span", null, useToolShortcuts().join(","));
    }
    expect(renderToStaticMarkup(createElement(Probe))).toBe(`<span>${DEFAULT_TOOL_SHORTCUTS.filter((tool) => tool !== "log").join(",")}</span>`);
  });
});

describe("createToolShortcutsStore", () => {
  it("starts from the stored list", () => {
    const store = createToolShortcutsStore(() => memoryStorage({ [TOOL_SHORTCUTS_KEY]: '["config","cube"]' }));
    expect(store.get()).toEqual(["cube", "config"]);
  });

  it("seeds every tool once for a profile upgraded from a release without shortcuts and saves the seed", () => {
    const storage = memoryStorage({ [DOCK_LAYOUT_STORAGE_KEY]: "{}" });
    expect(createToolShortcutsStore(() => storage).get()).toEqual([...DOCK_TOOL_IDS]);
    expect(stored(storage)).toEqual([...DOCK_TOOL_IDS]);
  });

  it("saves the defaults of a fresh profile, so a layout saved later does not read as an upgrade", () => {
    const storage = memoryStorage();
    expect(createToolShortcutsStore(() => storage).get()).toEqual(DEFAULT_TOOL_SHORTCUTS);
    storage.map.set(DOCK_LAYOUT_STORAGE_KEY, "{}");
    expect(createToolShortcutsStore(() => storage).get()).toEqual(DEFAULT_TOOL_SHORTCUTS);
  });

  it("keeps a reset profile on the defaults after the layout is saved again", () => {
    const storage = memoryStorage({ [TOOL_SHORTCUTS_KEY]: '["files"]', [DOCK_LAYOUT_STORAGE_KEY]: "{}" });
    const store = createToolShortcutsStore(() => storage);
    store.reset();
    storage.map.set(DOCK_LAYOUT_STORAGE_KEY, "{}");
    expect(createToolShortcutsStore(() => storage).get()).toEqual(DEFAULT_TOOL_SHORTCUTS);
  });

  it("starts from the defaults when storage is missing or throws", () => {
    expect(createToolShortcutsStore(() => null).get()).toEqual(DEFAULT_TOOL_SHORTCUTS);
    expect(createToolShortcutsStore(() => throwingStorage).get()).toEqual(DEFAULT_TOOL_SHORTCUTS);
  });
});
