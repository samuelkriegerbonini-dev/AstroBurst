import { describe, expect, it } from "vitest";
import { DISPLAY_MEMORY_SAVE_MS, DisplayMemoryStore, type DisplayMemoryTimers } from "../displayMemoryStore";
import {
  DISPLAY_MEMORY_KEY,
  LEGACY_DISPLAY_KEY,
  emptyDisplayMemory,
  fileEntry,
  rememberLook,
  serializeDisplayMemory,
  touchFile,
  type DisplayMemoryDoc,
} from "../displayMemory";
import { DEFAULT_DISPLAY_SETTINGS, type DisplayLook } from "../../shared/types/display";

const ASINH: DisplayLook = {
  stretch: "asinh",
  limits: "zscale",
  percentileLow: 1,
  percentileHigh: 99.5,
  userLo: null,
  userHi: null,
  zscaleContrast: 0.25,
  asinhA: 0.1,
  power: 2,
  colormap: "viridis",
  invert: true,
  symmetric: false,
  centre: 0,
};
const MAGMA: DisplayLook = { ...ASINH, stretch: "mtf", limits: "minmax", colormap: "magma", invert: false };

function memoryStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  const writes: [string, string][] = [];
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => {
      writes.push([k, v]);
      map.set(k, v);
    },
    map,
    writes,
  };
}

function fakeTimers() {
  const queue = new Map<number, () => void>();
  const delays: number[] = [];
  let next = 0;
  const timers: DisplayMemoryTimers & { run: () => void; pending: () => number; delays: number[] } = {
    set: (fn, ms) => {
      delays.push(ms);
      next += 1;
      queue.set(next, fn);
      return next;
    },
    clear: (handle) => {
      queue.delete(handle as number);
    },
    run: () => {
      const due = [...queue.values()];
      queue.clear();
      due.forEach((fn) => fn());
    },
    pending: () => queue.size,
    delays,
  };
  return timers;
}

function savedDoc(): string {
  return serializeDisplayMemory(emptyDisplayMemory(null));
}

function filesOf(text: string): string[] {
  return (JSON.parse(text) as DisplayMemoryDoc).files.map(([k]) => k);
}

describe("DisplayMemoryStore", () => {
  it("migrates the legacy key on first use and writes the new key on flush", () => {
    const legacy = JSON.stringify({ ...DEFAULT_DISPLAY_SETTINGS, stretch: "asinh", limits: "zscale", colormap: "viridis", invert: true, colorbar: false });
    const storage = memoryStorage({ [LEGACY_DISPLAY_KEY]: legacy });
    const store = new DisplayMemoryStore(storage, fakeTimers());
    expect(store.get().defaults).toEqual(ASINH);
    expect(store.get().sameGroup).toBe(true);
    expect(storage.map.has(DISPLAY_MEMORY_KEY)).toBe(false);
    store.flush();
    expect(storage.writes.map(([k]) => k)).toEqual([DISPLAY_MEMORY_KEY]);
    expect(JSON.parse(storage.map.get(DISPLAY_MEMORY_KEY) as string)).toEqual(store.get());
    expect(storage.map.get(LEGACY_DISPLAY_KEY)).toBe(legacy);

    storage.map.set(LEGACY_DISPLAY_KEY, JSON.stringify({ ...DEFAULT_DISPLAY_SETTINGS, colormap: "heat" }));
    const reopened = new DisplayMemoryStore(storage, fakeTimers());
    expect(reopened.get().defaults.colormap).toBe("viridis");
    reopened.flush();
    expect(storage.writes).toHaveLength(1);
  });

  it("coalesces updates within DISPLAY_MEMORY_SAVE_MS into one setItem", () => {
    const storage = memoryStorage({ [DISPLAY_MEMORY_KEY]: savedDoc() });
    const timers = fakeTimers();
    const store = new DisplayMemoryStore(storage, timers);
    store.update((d) => rememberLook(d, "a", null, ASINH, null));
    store.update((d) => rememberLook(d, "b", null, MAGMA, null));
    store.update((d) => touchFile(d, "a"));
    expect(storage.writes).toHaveLength(0);
    expect(timers.delays).toEqual([DISPLAY_MEMORY_SAVE_MS]);
    timers.run();
    expect(storage.writes).toHaveLength(1);
    expect(filesOf(storage.writes[0][1])).toEqual(["b", "a"]);
  });

  it("an update that returns the same doc neither notifies nor schedules a write", () => {
    const storage = memoryStorage({ [DISPLAY_MEMORY_KEY]: savedDoc() });
    const timers = fakeTimers();
    const store = new DisplayMemoryStore(storage, timers);
    let notified = 0;
    store.subscribe(() => {
      notified += 1;
    });
    store.update((d) => d);
    store.update((d) => touchFile(d, "unknown.fits"));
    expect(notified).toBe(0);
    expect(timers.pending()).toBe(0);
    store.flush();
    expect(storage.writes).toHaveLength(0);
  });

  it("flush writes only when dirty", () => {
    const storage = memoryStorage({ [DISPLAY_MEMORY_KEY]: savedDoc() });
    const timers = fakeTimers();
    const store = new DisplayMemoryStore(storage, timers);
    store.flush();
    expect(storage.writes).toHaveLength(0);
    store.update((d) => rememberLook(d, "a", null, ASINH, null));
    store.flush();
    expect(storage.writes).toHaveLength(1);
    expect(timers.pending()).toBe(0);
    store.flush();
    timers.run();
    expect(storage.writes).toHaveLength(1);
    expect(JSON.parse(storage.map.get(DISPLAY_MEMORY_KEY) as string)).toEqual(store.get());
  });

  it("a throwing setItem keeps the in-memory doc, evicts half of the files and retries once", () => {
    const attempts: string[] = [];
    const full = {
      getItem: () => null,
      setItem: (_k: string, v: string) => {
        attempts.push(v);
        throw new Error("QuotaExceededError");
      },
    };
    const store = new DisplayMemoryStore(full, fakeTimers());
    for (let i = 0; i < 6; i++) store.update((d) => rememberLook(d, `f${i}`, null, ASINH, null));
    const before = store.get();
    expect(() => store.flush()).not.toThrow();
    expect(attempts).toHaveLength(2);
    expect(filesOf(attempts[0])).toEqual(["f0", "f1", "f2", "f3", "f4", "f5"]);
    expect(filesOf(attempts[1])).toEqual(["f3", "f4", "f5"]);
    expect(store.get()).toBe(before);
    store.update((d) => rememberLook(d, "f6", null, MAGMA, null));
    store.flush();
    expect(attempts).toHaveLength(2);
    expect(fileEntry(store.get(), "f6")).toEqual({ look: MAGMA });

    let failures = 1;
    const once = memoryStorage();
    const flaky = {
      getItem: once.getItem,
      setItem: (k: string, v: string) => {
        if (failures-- > 0) throw new Error("QuotaExceededError");
        once.setItem(k, v);
      },
    };
    const recovering = new DisplayMemoryStore(flaky, fakeTimers());
    for (let i = 0; i < 4; i++) recovering.update((d) => rememberLook(d, `g${i}`, null, ASINH, null));
    recovering.flush();
    expect(once.writes).toHaveLength(1);
    expect(filesOf(once.writes[0][1])).toEqual(["g2", "g3"]);
    expect(recovering.get().files.map(([k]) => k)).toEqual(["g2", "g3"]);
  });

  it("a throwing getItem and a null storage start from factory defaults without throwing", () => {
    const blocked = {
      getItem: (): string | null => {
        throw new Error("SecurityError");
      },
      setItem: () => {},
    };
    let fromBlocked: DisplayMemoryStore | null = null;
    expect(() => {
      fromBlocked = new DisplayMemoryStore(blocked, fakeTimers());
    }).not.toThrow();
    expect((fromBlocked as unknown as DisplayMemoryStore).get()).toEqual(emptyDisplayMemory(null));

    const timers = fakeTimers();
    const memoryOnly = new DisplayMemoryStore(null, timers);
    expect(memoryOnly.get()).toEqual(emptyDisplayMemory(null));
    memoryOnly.update((d) => rememberLook(d, "a", null, ASINH, null));
    expect(() => memoryOnly.flush()).not.toThrow();
    expect(fileEntry(memoryOnly.get(), "a")).toEqual({ look: ASINH });
    expect(timers.pending()).toBe(0);
  });

  it("subscribe is notified on update and fileEntry keeps its identity while unchanged", () => {
    const store = new DisplayMemoryStore(null, fakeTimers());
    store.update((d) => rememberLook(d, "a", null, ASINH, null));
    store.update((d) => rememberLook(d, "b", null, MAGMA, null));
    const a = fileEntry(store.get(), "a");
    let notified = 0;
    const unsubscribe = store.subscribe(() => {
      notified += 1;
    });
    store.update((d) => rememberLook(d, "b", null, ASINH, null));
    expect(notified).toBe(1);
    expect(a).not.toBeNull();
    expect(fileEntry(store.get(), "a")).toBe(a);
    expect(fileEntry(store.get(), "b")).toEqual({ look: ASINH });
    unsubscribe();
    store.update((d) => rememberLook(d, "c", null, ASINH, null));
    expect(notified).toBe(1);
  });
});
