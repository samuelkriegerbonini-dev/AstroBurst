import { describe, expect, it, vi } from "vitest";
import {
  EMPTY_LIGHT_CURVES,
  LightCurveStore,
  MAX_LIGHT_CURVE_GROUPS,
  lightCurveStore,
  useActiveLightCurveTable,
  useLightCurveGroups,
  useLightCurveView,
  useLightCurves,
} from "../useLightCurveStore";
import { DEFAULT_LIGHT_CURVE_VIEW, type LightCurveDoc, type LightCurveTimeScale, type LightCurveViewState } from "../../shared/types/lightCurve";
import { hooks } from "../../test/hookRuntime";

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

function doc(id: string, n: number, scale: LightCurveTimeScale): LightCurveDoc {
  const points = Array.from({ length: n }, (_, i) => i);
  return {
    id,
    label: id,
    quantity: "rel_flux",
    unit: null,
    timeScale: scale,
    time: points.map((i) => 60061 + i / 100),
    value: points.map(() => 1),
    err: points.map(() => 0.001),
    pointRef: points,
    flags: points.map(() => []),
    source: { kind: "table", path: `${id}.csv`, time_column: "t", value_column: "f" },
    notes: [],
  };
}

function putGroups(store: LightCurveStore, first: number, last: number): void {
  for (let i = first; i <= last; i++) store.put(`g${i}`, [doc(`d${i}`, 3, "bjd_tdb")]);
}

function names(first: number, last: number): string[] {
  return Array.from({ length: last - first + 1 }, (_, i) => `g${first + i}`);
}

describe("LightCurveStore", () => {
  it("put stores the documents of a group and get returns the same array until the next write", () => {
    const store = new LightCurveStore();
    const listener = vi.fn();
    store.subscribe(listener);
    const d1 = doc("a", 3, "bjd_tdb");
    const given = [d1];
    store.put("frames:a", given);
    expect(store.get("frames:a")[0]).toBe(d1);
    expect(store.get("frames:a")).toBe(store.get("frames:a"));
    expect(store.get("frames:a")).toBe(given);
    expect(Object.isFrozen(store.get("frames:a"))).toBe(true);
    expect(store.get("frames:b")).toBe(EMPTY_LIGHT_CURVES);
    expect(EMPTY_LIGHT_CURVES).toHaveLength(0);
    expect(Object.isFrozen(EMPTY_LIGHT_CURVES)).toBe(true);
    expect(listener).toHaveBeenCalledTimes(1);
    const before = store.get("frames:a");
    store.put("frames:a", [doc("b", 3, "bjd_tdb")]);
    expect(store.get("frames:a")).not.toBe(before);
    expect(store.get("frames:a")[0].id).toBe("b");
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("the seventeenth group drops the oldest one that nobody retains", () => {
    expect(MAX_LIGHT_CURVE_GROUPS).toBe(16);
    const store = new LightCurveStore();
    putGroups(store, 0, 15);
    store.patchView("g0", { display: "normalised" });
    expect(store.groups()).toEqual(names(0, 15));
    putGroups(store, 16, 16);
    expect(store.get("g0")).toBe(EMPTY_LIGHT_CURVES);
    expect(store.getView("g0")).toBe(DEFAULT_LIGHT_CURVE_VIEW);
    expect(store.groups()).toEqual(names(1, 16));

    const retained = new LightCurveStore();
    const release = retained.retain("g0");
    putGroups(retained, 0, 16);
    expect(retained.get("g0")).toHaveLength(1);
    expect(retained.get("g1")).toBe(EMPTY_LIGHT_CURVES);
    expect(retained.groups()).toEqual(["g0", ...names(2, 16)]);
    release();
    release();
    putGroups(retained, 17, 17);
    expect(retained.get("g0")).toBe(EMPTY_LIGHT_CURVES);
    expect(retained.groups()).toEqual(names(2, 17));

    const shared = new LightCurveStore();
    const releaseFirst = shared.retain("g0");
    shared.retain("g0");
    releaseFirst();
    releaseFirst();
    putGroups(shared, 0, 16);
    expect(shared.get("g0")).toHaveLength(1);

    const crowded = new LightCurveStore();
    names(0, 15).forEach((name) => crowded.retain(name));
    putGroups(crowded, 0, 16);
    expect(crowded.groups()).toEqual(names(0, 16));
  });

  it("put over a group makes it the newest", () => {
    const store = new LightCurveStore();
    putGroups(store, 0, 15);
    putGroups(store, 0, 0);
    putGroups(store, 16, 16);
    expect(store.get("g1")).toBe(EMPTY_LIGHT_CURVES);
    expect(store.get("g0")).toHaveLength(1);
    expect(store.groups()).toEqual([...names(2, 15), "g0", "g16"]);
  });

  it("remove drops the documents and the view state, and an empty put is a remove", () => {
    const store = new LightCurveStore();
    store.put("g", [doc("a", 3, "bjd_tdb")]);
    store.patchView("g", { display: "normalised" });
    const listener = vi.fn();
    store.subscribe(listener);
    store.remove("g");
    expect(store.get("g")).toBe(EMPTY_LIGHT_CURVES);
    expect(store.getView("g")).toBe(DEFAULT_LIGHT_CURVE_VIEW);
    expect(store.groups()).toEqual([]);
    expect(listener).toHaveBeenCalledTimes(1);
    store.remove("g");
    expect(listener).toHaveBeenCalledTimes(1);

    store.put("g", [doc("a", 3, "bjd_tdb")]);
    store.patchView("g", { display: "normalised" });
    store.put("g", []);
    expect(store.get("g")).toBe(EMPTY_LIGHT_CURVES);
    expect(store.getView("g")).toBe(DEFAULT_LIGHT_CURVE_VIEW);
    expect(store.groups()).toEqual([]);
    store.put("g", [doc("a", 3, "bjd_tdb")]);
    expect(store.getView("g")).toBe(DEFAULT_LIGHT_CURVE_VIEW);
  });

  it("the view state is kept across a put and its windows reset when the time scale or the length changes", () => {
    const patch: Partial<LightCurveViewState> = {
      display: "normalised",
      window: { start: 1, end: 2 },
      marginDays: 0.1,
      excluded: [1],
      fold: { periodDays: 2, t0: 0 },
    };
    const store = new LightCurveStore();
    store.put("g", [doc("a", 3, "bjd_tdb")]);
    store.patchView("g", patch);
    const patched = store.getView("g");
    expect(patched).toEqual({ ...DEFAULT_LIGHT_CURVE_VIEW, ...patch });
    store.put("g", [doc("b", 3, "bjd_tdb")]);
    expect(store.getView("g")).toBe(patched);

    store.put("g", [doc("c", 3, "jd_header")]);
    expect(store.getView("g")).toEqual({ ...DEFAULT_LIGHT_CURVE_VIEW, display: "normalised" });
    expect(store.getView("g").window).toBeNull();
    expect(store.getView("g").marginDays).toBe(0);
    expect(store.getView("g").excluded).toEqual([]);
    expect(store.getView("g").fold).toBeNull();

    store.patchView("g", patch);
    store.put("g", [doc("d", 4, "jd_header")]);
    expect(store.getView("g")).toEqual({ ...DEFAULT_LIGHT_CURVE_VIEW, display: "normalised" });

    store.patchView("g", patch);
    store.put("g", [doc("e", 4, "jd_header"), doc("f", 9, "index")]);
    expect(store.getView("g")).toEqual({ ...DEFAULT_LIGHT_CURVE_VIEW, ...patch });
  });

  it("patchView on an unknown group does nothing and getView returns the default", () => {
    const store = new LightCurveStore();
    const listener = vi.fn();
    store.subscribe(listener);
    store.patchView("nope", { display: "normalised" });
    expect(listener).not.toHaveBeenCalled();
    expect(store.getView("nope")).toBe(DEFAULT_LIGHT_CURVE_VIEW);
    expect(DEFAULT_LIGHT_CURVE_VIEW.display).toBe("native");

    store.put("g", [doc("a", 3, "bjd_tdb")]);
    expect(store.getView("g")).toBe(DEFAULT_LIGHT_CURVE_VIEW);
    store.patchView("g", { selection: "a" });
    const first = store.getView("g");
    expect(first).toEqual({ ...DEFAULT_LIGHT_CURVE_VIEW, selection: "a" });
    expect(store.getView("g")).toBe(first);
    expect(listener).toHaveBeenCalledTimes(2);
    store.patchView("g", { clipSigma: 4 });
    expect(store.getView("g")).not.toBe(first);
    expect(store.getView("g")).toEqual({ ...DEFAULT_LIGHT_CURVE_VIEW, selection: "a", clipSigma: 4 });
    expect(first.clipSigma).toBeNull();
    expect(listener).toHaveBeenCalledTimes(3);
    const unsubscribed = vi.fn();
    store.subscribe(unsubscribed)();
    store.patchView("g", { clipSigma: 5 });
    expect(unsubscribed).not.toHaveBeenCalled();
  });

  it("groups filters by prefix and keeps its array between writes", () => {
    const store = new LightCurveStore();
    store.put("frames:a", [doc("a", 3, "jd_header")]);
    store.put("table:x", [doc("x", 3, "bjd_tdb")]);
    store.put("table:y", [doc("y", 3, "bjd_tdb")]);
    const tables = store.groups("table:");
    expect(tables).toEqual(["table:x", "table:y"]);
    expect(store.groups("table:")).toBe(tables);
    expect(store.groups()).toEqual(["frames:a", "table:x", "table:y"]);
    expect(store.groups()).toBe(store.groups());
    expect(store.groups("tso:")).toEqual([]);
    expect(store.groups("tso:")).toBe(store.groups("tso:"));
    store.put("frames:b", [doc("b", 3, "jd_header")]);
    expect(store.groups("table:")).not.toBe(tables);
    expect(store.groups("table:")).toEqual(["table:x", "table:y"]);
    const beforeRemove = store.groups("table:");
    store.remove("table:x");
    expect(store.groups("table:")).not.toBe(beforeRemove);
    expect(store.groups("table:")).toEqual(["table:y"]);
  });

  it("the active table follows setActiveTable and clears when its group is removed", () => {
    const store = new LightCurveStore();
    store.put("table:x", [doc("x", 3, "bjd_tdb")]);
    store.put("table:y", [doc("y", 3, "bjd_tdb")]);
    const listener = vi.fn();
    store.subscribe(listener);
    expect(store.getActiveTable()).toBeNull();
    store.setActiveTable("table:x");
    expect(store.getActiveTable()).toBe("table:x");
    expect(listener).toHaveBeenCalledTimes(1);
    store.remove("table:y");
    expect(store.getActiveTable()).toBe("table:x");
    expect(listener).toHaveBeenCalledTimes(2);
    store.remove("table:x");
    expect(store.getActiveTable()).toBeNull();
    expect(listener).toHaveBeenCalledTimes(3);
    store.setActiveTable("table:z");
    store.setActiveTable(null);
    expect(store.getActiveTable()).toBeNull();
    expect(listener).toHaveBeenCalledTimes(5);
  });
});

describe("light-curve store hooks", () => {
  it("the hooks read the store through useSyncExternalStore", () => {
    const group = "table:hook-test.csv";
    const others = Array.from({ length: MAX_LIGHT_CURVE_GROUPS }, (_, i) => `frames:hook-test-${i}`);
    try {
      hooks.mount();
      lightCurveStore.put(group, [doc("a", 3, "bjd_tdb")]);
      lightCurveStore.patchView(group, { display: "normalised" });
      lightCurveStore.setActiveTable(group);
      const Probe = () => ({
        docs: useLightCurves(group),
        none: useLightCurves(null),
        view: useLightCurveView(group),
        noView: useLightCurveView(null),
        tables: useLightCurveGroups("table:hook-test"),
        active: useActiveLightCurveTable(),
      });
      const first = hooks.render(Probe, undefined) as ReturnType<typeof Probe>;
      expect(first.docs).toBe(lightCurveStore.get(group));
      expect(first.docs[0].id).toBe("a");
      expect(first.none).toBe(EMPTY_LIGHT_CURVES);
      expect(first.view).toBe(lightCurveStore.getView(group));
      expect(first.view.display).toBe("normalised");
      expect(first.noView).toBe(DEFAULT_LIGHT_CURVE_VIEW);
      expect(first.tables).toEqual([group]);
      expect(first.active).toBe(group);

      const notifiedBefore = hooks.run.notified;
      others.forEach((name) => lightCurveStore.put(name, [doc(name, 2, "jd_header")]));
      expect(hooks.run.notified).toBeGreaterThan(notifiedBefore);
      const second = hooks.render(Probe, undefined) as ReturnType<typeof Probe>;
      expect(second.docs).toBe(first.docs);
      expect(second.view).toBe(first.view);
      expect(second.tables).toEqual([group]);

      hooks.mount();
      lightCurveStore.put(others[0], [doc("again", 2, "jd_header")]);
      expect(lightCurveStore.get(group)).toBe(EMPTY_LIGHT_CURVES);
      expect(lightCurveStore.getActiveTable()).toBeNull();
    } finally {
      hooks.mount();
      lightCurveStore.setActiveTable(null);
      [group, ...others].forEach((name) => lightCurveStore.remove(name));
    }
    expect(lightCurveStore.groups()).toEqual([]);
  });
});
