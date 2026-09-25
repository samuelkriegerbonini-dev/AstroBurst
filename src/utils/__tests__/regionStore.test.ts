import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { RegionStoreCore, EMPTY_DOC, SAVE_DEBOUNCE_MS, shapeKindKey } from "../regionStore";
import { STORAGE_PREFIX, DEFAULT_REGION_PROPS } from "../regionPersistence";
import type { Region, RegionShape } from "../../shared/types/regions";

class MemoryStorage {
  data = new Map<string, string>();
  getItem(k: string): string | null {
    return this.data.has(k) ? (this.data.get(k) as string) : null;
  }
  setItem(k: string, v: string): void {
    this.data.set(k, v);
  }
}

function region(id: string, backgroundId: string | null = null): Region {
  return { id, shape: { shape: "circle", x: 10, y: 10, r: 3 }, props: { ...DEFAULT_REGION_PROPS }, backgroundId };
}

function annulus(id: string): Region {
  return {
    id,
    shape: { shape: "annulus", x: 10, y: 10, r_inner: 5, r_outer: 8 },
    props: { ...DEFAULT_REGION_PROPS },
    backgroundId: null,
  };
}

function point(id: string): Region {
  return { id, shape: { shape: "point", x: 4, y: 6 }, props: { ...DEFAULT_REGION_PROPS }, backgroundId: null };
}

describe("RegionStoreCore", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("isolates documents per fileKey and returns a stable doc reference", () => {
    const store = new RegionStoreCore(new MemoryStorage());
    store.add("a", region("r1"));
    expect(store.getDoc("a").regions).toHaveLength(1);
    expect(store.getDoc("b").regions).toHaveLength(0);
    expect(store.getDoc("b")).toBe(store.getDoc("b"));
    expect(EMPTY_DOC.regions).toEqual([]);
  });

  it("bumps the version and notifies once per mutation", () => {
    const store = new RegionStoreCore(new MemoryStorage());
    const listener = vi.fn();
    store.subscribe(listener);
    store.add("f", region("r1"));
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.getDoc("f").version).toBe(1);
    store.update("f", "r1", { shape: { shape: "circle", x: 1, y: 1, r: 2 } });
    expect(listener).toHaveBeenCalledTimes(2);
    expect(store.getDoc("f").version).toBe(2);
    expect(store.getDoc("f").regions[0].shape).toEqual({ shape: "circle", x: 1, y: 1, r: 2 });
    store.select("f", "r1");
    expect(listener).toHaveBeenCalledTimes(3);
    expect(store.getDoc("f").selectedId).toBe("r1");
    expect(store.getDoc("f").version).toBe(3);
    store.select("f", "r1");
    expect(listener).toHaveBeenCalledTimes(3);
    store.add("f", annulus("bg"));
    store.setBackground("f", "r1", "bg");
    expect(listener).toHaveBeenCalledTimes(5);
    expect(store.getDoc("f").regions[0].backgroundId).toBe("bg");
    expect(store.getDoc("f").version).toBe(5);
    store.remove("f", "bg");
    expect(listener).toHaveBeenCalledTimes(6);
    store.remove("f", "missing");
    expect(listener).toHaveBeenCalledTimes(6);
    store.update("f", "missing", { backgroundId: null });
    expect(listener).toHaveBeenCalledTimes(6);
  });

  it("keeps the regions array identity on select", () => {
    const store = new RegionStoreCore(new MemoryStorage());
    store.add("f", region("r1"));
    const before = store.getDoc("f").regions;
    store.select("f", "r1");
    expect(store.getDoc("f").regions).toBe(before);
  });

  it("removing the selected region clears selection and the dependants backgroundId", () => {
    const store = new RegionStoreCore(new MemoryStorage());
    store.add("f", annulus("bg"));
    store.add("f", region("r1", "bg"));
    store.add("f", region("r2", "bg"));
    store.select("f", "bg");
    store.remove("f", "bg");
    const doc = store.getDoc("f");
    expect(doc.selectedId).toBeNull();
    expect(doc.regions.map((r) => r.id)).toEqual(["r1", "r2"]);
    expect(doc.regions.every((r) => r.backgroundId === null)).toBe(true);
  });

  it("rejects an unknown or self background", () => {
    const store = new RegionStoreCore(new MemoryStorage());
    store.add("f", region("r1"));
    store.setBackground("f", "r1", "nope");
    expect(store.getDoc("f").regions[0].backgroundId).toBeNull();
    store.setBackground("f", "r1", "r1");
    expect(store.getDoc("f").regions[0].backgroundId).toBeNull();
  });

  it("replaceAll and clear reset the document", () => {
    const store = new RegionStoreCore(new MemoryStorage());
    store.add("f", region("r1"));
    store.select("f", "r1");
    store.replaceAll("f", [region("x"), region("y")]);
    expect(store.getDoc("f").regions.map((r) => r.id)).toEqual(["x", "y"]);
    expect(store.getDoc("f").selectedId).toBeNull();
    store.clear("f");
    expect(store.getDoc("f").regions).toEqual([]);
  });

  it("persists after the debounce and on flush, under the prefixed key", () => {
    const storage = new MemoryStorage();
    const store = new RegionStoreCore(storage, () => 12345);
    store.add("file.fits", region("r1"));
    expect(storage.data.has(`${STORAGE_PREFIX}file.fits`)).toBe(false);
    vi.advanceTimersByTime(SAVE_DEBOUNCE_MS - 1);
    expect(storage.data.has(`${STORAGE_PREFIX}file.fits`)).toBe(false);
    vi.advanceTimersByTime(1);
    expect(JSON.parse(storage.data.get(`${STORAGE_PREFIX}file.fits`) as string)).toHaveLength(1);
    store.add("file.fits", region("r2"));
    store.flush();
    expect(JSON.parse(storage.data.get(`${STORAGE_PREFIX}file.fits`) as string)).toHaveLength(2);
    expect(store.getLastSavedAt()).toBe(12345);
  });

  it("skips persistence for a non-persisting update until persistNow", () => {
    const storage = new MemoryStorage();
    const store = new RegionStoreCore(storage, () => 999);
    store.add("f", region("r1"));
    store.flush();
    const key = `${STORAGE_PREFIX}f`;
    const saved = storage.data.get(key) as string;

    for (let i = 0; i < 10; i++) {
      store.update("f", "r1", { shape: { shape: "circle", x: i, y: i, r: 3 } }, false);
    }
    vi.advanceTimersByTime(SAVE_DEBOUNCE_MS * 2);
    expect(storage.data.get(key)).toBe(saved);
    expect(store.getDoc("f").regions[0].shape).toEqual({ shape: "circle", x: 9, y: 9, r: 3 });

    store.persistNow("f");
    expect(JSON.parse(storage.data.get(key) as string)[0].shape).toEqual({ shape: "circle", x: 9, y: 9, r: 3 });
  });

  it("flush still writes edits that were deferred by an interrupted drag", () => {
    const storage = new MemoryStorage();
    const store = new RegionStoreCore(storage);
    store.add("f", region("r1"));
    store.flush();
    store.update("f", "r1", { shape: { shape: "circle", x: 42, y: 7, r: 3 } }, false);
    store.flush();
    expect(JSON.parse(storage.data.get(`${STORAGE_PREFIX}f`) as string)[0].shape).toEqual({
      shape: "circle", x: 42, y: 7, r: 3,
    });
  });

  it("does not rewrite storage when only the selection changed", () => {
    const storage = new MemoryStorage();
    let writes = 0;
    const counting = {
      getItem: (k: string) => storage.getItem(k),
      setItem: (k: string, v: string) => { writes += 1; storage.setItem(k, v); },
    };
    const store = new RegionStoreCore(counting);
    store.add("f", region("r1"));
    store.flush();
    expect(writes).toBe(1);
    store.select("f", "r1");
    store.flush();
    expect(writes).toBe(1);
  });

  it("reloads from storage in a fresh instance", () => {
    const storage = new MemoryStorage();
    const first = new RegionStoreCore(storage);
    first.add("f", region("r1", null));
    first.add("f", annulus("bg"));
    first.setBackground("f", "r1", "bg");
    first.flush();
    const second = new RegionStoreCore(storage);
    const doc = second.getDoc("f");
    expect(doc.regions.map((r) => r.id)).toEqual(["r1", "bg"]);
    expect(doc.regions[0].backgroundId).toBe("bg");
    expect(doc.version).toBe(0);
  });

  it("tool and draft setters notify and skip no-ops", () => {
    const store = new RegionStoreCore(null);
    const listener = vi.fn();
    store.subscribe(listener);
    store.setTool("circle");
    expect(store.getTool()).toBe("circle");
    store.setTool("circle");
    expect(listener).toHaveBeenCalledTimes(1);
    const draft = { shape: "circle", x: 0, y: 0, r: 1 } as const;
    store.setDraft("a", draft);
    expect(store.getDraftFor("a")).toBe(draft);
    store.setDraft("a", draft);
    expect(listener).toHaveBeenCalledTimes(2);
    store.setDraft("a", null);
    expect(store.getDraftFor("a")).toBeNull();
    expect(listener).toHaveBeenCalledTimes(3);
  });

  it("scopes the draft to one fileKey and never leaks it to another image", () => {
    const store = new RegionStoreCore(null);
    const draft: RegionShape = { shape: "polygon", points: [[1, 1], [5, 1], [3, 6]] };
    store.setDraft("a", draft);
    expect(store.getDraftFor("a")).toBe(draft);
    expect(store.getDraftFor("b")).toBeNull();
    expect(store.getDraftFor(null)).toBeNull();
    expect(store.getDraftKey()).toBe("a");
  });

  it("clearDraft only clears the draft of the given fileKey", () => {
    const store = new RegionStoreCore(null);
    const listener = vi.fn();
    const draft = { shape: "circle", x: 0, y: 0, r: 1 } as const;
    store.setDraft("a", draft);
    store.subscribe(listener);
    store.clearDraft("b");
    expect(store.getDraftFor("a")).toBe(draft);
    expect(listener).not.toHaveBeenCalled();
    store.clearDraft("a");
    expect(store.getDraftFor("a")).toBeNull();
    expect(listener).toHaveBeenCalledTimes(1);
    store.clearDraft();
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("replaces a draft when the fileKey changes", () => {
    const store = new RegionStoreCore(null);
    const a = { shape: "circle", x: 0, y: 0, r: 1 } as const;
    const b = { shape: "circle", x: 9, y: 9, r: 2 } as const;
    store.setDraft("a", a);
    store.setDraft("b", b);
    expect(store.getDraftFor("a")).toBeNull();
    expect(store.getDraftFor("b")).toBe(b);
  });

  describe("removeShapeKind", () => {
    it("removes every point of the file in one commit and leaves the other shapes untouched", () => {
      const store = new RegionStoreCore(new MemoryStorage());
      store.add("f", point("p1"));
      store.add("f", region("c1"));
      store.add("f", point("p2"));
      store.add("f", annulus("bg"));
      store.add("f", point("p3"));
      store.add("g", point("other"));
      const listener = vi.fn();
      store.subscribe(listener);
      const versionBefore = store.getDoc("f").version;
      store.removeShapeKind("f", "point");
      const doc = store.getDoc("f");
      expect(doc.regions.map((r) => r.id)).toEqual(["c1", "bg"]);
      expect(doc.version).toBe(versionBefore + 1);
      expect(listener).toHaveBeenCalledTimes(1);
      expect(store.getDoc("g").regions.map((r) => r.id)).toEqual(["other"]);
    });

    it("clears the selection and the backgroundId references that pointed to a removed region", () => {
      const store = new RegionStoreCore(new MemoryStorage());
      store.add("f", point("p1"));
      store.add("f", region("c1", "p1"));
      store.add("f", annulus("bg"));
      store.add("f", region("c2", "bg"));
      store.select("f", "p1");
      store.removeShapeKind("f", "point");
      const doc = store.getDoc("f");
      expect(doc.selectedId).toBeNull();
      expect(doc.regions.find((r) => r.id === "c1")?.backgroundId).toBeNull();
      expect(doc.regions.find((r) => r.id === "c2")?.backgroundId).toBe("bg");
    });

    it("keeps a selection that points to a surviving region", () => {
      const store = new RegionStoreCore(new MemoryStorage());
      store.add("f", point("p1"));
      store.add("f", region("c1"));
      store.select("f", "c1");
      store.removeShapeKind("f", "point");
      expect(store.getDoc("f").selectedId).toBe("c1");
    });

    it("is a no-op without a region of that kind", () => {
      const store = new RegionStoreCore(new MemoryStorage());
      store.add("f", region("c1"));
      store.select("f", "c1");
      const listener = vi.fn();
      store.subscribe(listener);
      const before = store.getDoc("f");
      store.removeShapeKind("f", "point");
      expect(store.getDoc("f")).toBe(before);
      expect(listener).not.toHaveBeenCalled();
    });

    it("persists the surviving regions with a single write", () => {
      const storage = new MemoryStorage();
      let writes = 0;
      const counting = {
        getItem: (k: string) => storage.getItem(k),
        setItem: (k: string, v: string) => { writes += 1; storage.setItem(k, v); },
      };
      const store = new RegionStoreCore(counting);
      store.add("f", point("p1"));
      store.add("f", point("p2"));
      store.add("f", region("c1"));
      store.flush();
      expect(writes).toBe(1);
      store.removeShapeKind("f", "point");
      vi.advanceTimersByTime(SAVE_DEBOUNCE_MS);
      expect(writes).toBe(2);
      const saved = JSON.parse(storage.data.get(`${STORAGE_PREFIX}f`) as string) as Region[];
      expect(saved.map((r) => r.id)).toEqual(["c1"]);
      expect(new RegionStoreCore(storage).getDoc("f").regions.map((r) => r.id)).toEqual(["c1"]);
    });
  });
});

describe("shapeKindKey", () => {
  it("changes when the points are removed and different points are added", () => {
    const armed = shapeKindKey([point("p1"), point("p2")], "point");
    expect(shapeKindKey([], "point")).not.toBe(armed);
    expect(shapeKindKey([point("p3")], "point")).not.toBe(armed);
    expect(shapeKindKey([point("p1"), point("p2"), point("p3")], "point")).not.toBe(armed);
  });

  it("ignores other shapes, the order of the regions and the point positions", () => {
    const armed = shapeKindKey([point("p1"), region("c1"), point("p2")], "point");
    const moved: Region = { ...point("p1"), shape: { shape: "point", x: 50, y: 60 } };
    expect(shapeKindKey([point("p2"), annulus("bg"), moved], "point")).toBe(armed);
  });

  it("does not confuse one id containing a separator with two ids", () => {
    expect(shapeKindKey([point("a,b")], "point")).not.toBe(shapeKindKey([point("a"), point("b")], "point"));
  });
});
