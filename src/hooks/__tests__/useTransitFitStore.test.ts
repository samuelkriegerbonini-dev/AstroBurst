import { describe, expect, it, vi } from "vitest";
import {
  EMPTY_TRANSIT_FIT,
  MAX_TRANSIT_FITS,
  TransitFitStore,
  transitFitKey,
  transitFitStore,
  useTransitFit,
} from "../useTransitFitStore";
import type { TransitFitRequest, TransitFitResult } from "../../shared/types/transit";
import { DEFAULT_TRANSIT_FIT_SETUP, type TransitFitSetup } from "../../utils/transitFitSetup";
import { hooks } from "../../test/hookRuntime";

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

const S: TransitFitSetup = { ...DEFAULT_TRANSIT_FIT_SETUP, period: "3" };
const OTHER_SETUP: TransitFitSetup = { ...DEFAULT_TRANSIT_FIT_SETUP, period: "5" };

const REQ: TransitFitRequest = {
  kind: "transit",
  time: [1, 2, 3],
  flux: [1, 0.99, 1],
  err: null,
  period: 3,
  t0: { value: 2, free: true },
  rp: { value: 0.1, free: true },
  a_rs: { value: 9, free: false },
  b: { value: 0.3, free: false },
  u1: { value: 0.4, free: false },
  u2: { value: 0.25, free: false },
  flux_ratio: null,
  baseline_order: 0,
  scale_errors: false,
};

const RESULT: TransitFitResult = {
  model: "quadratic limb darkening",
  period: 3,
  parameters: [{ name: "rp", value: 0.1, err: 0.001, free: true, at_bound: false }],
  baseline: { order: 0, reference_time: 2, coefficients: [1], errors: [0.001] },
  depth: 0.01,
  depth_err: 0.0002,
  chi2: 1,
  dof: 1,
  reduced_chi2: 1,
  weighted: false,
  error_scale: 1,
  rms: 0.001,
  rms_ppm: 1000,
  n_points: 3,
  n_in_transit: 1,
  iterations: 3,
  converged: true,
  free_names: ["t0", "rp"],
  time: [],
  model_flux: [],
  baseline_flux: [],
  residual: [],
  notes: [],
  warnings: [],
  correlation: null,
  derived: {
    inclination_deg: null,
    inclination_err: null,
    duration_t14_days: null,
    duration_t14_err: null,
    duration_t23_days: null,
    duration_t23_err: null,
    stellar_density_cgs: null,
    stellar_density_err: null,
  },
  time_averaging: { cadence_days: 1, bin_points: [], n_bins: [], rms_ppm: [], expected_ppm: [], beta: 1 },
  curve_time: [],
  curve_flux: [],
  curve_baseline: [],
  kind: "transit",
  elapsed_ms: 1,
};

const FIT = { request: REQ, result: RESULT, signature: "s1", fittedUtc: "2026-11-13T12:00:00.000Z" };

function writeSetups(store: TransitFitStore, first: number, last: number): void {
  for (let i = first; i <= last; i++) store.setSetup("g", `d${i}`, S);
}

describe("TransitFitStore", () => {
  it("an unknown curve has the default setup and no fit, and get is stable between writes", () => {
    expect(transitFitKey("tso:a", "white")).toEqual("tso:a#white");
    expect(EMPTY_TRANSIT_FIT).toEqual({ setup: DEFAULT_TRANSIT_FIT_SETUP, request: null, result: null, signature: null, fittedUtc: null });
    expect(EMPTY_TRANSIT_FIT.setup).toBe(DEFAULT_TRANSIT_FIT_SETUP);
    expect(Object.isFrozen(EMPTY_TRANSIT_FIT)).toBe(true);

    const store = new TransitFitStore();
    const listener = vi.fn();
    store.subscribe(listener);
    expect(store.get("g", "d")).toBe(EMPTY_TRANSIT_FIT);
    expect(store.size()).toEqual(0);
    store.setSetup("g", "d", S);
    const record = store.get("g", "d");
    expect(store.get("g", "d")).toBe(record);
    expect(record.setup).toBe(S);
    expect(record).toEqual({ setup: S, request: null, result: null, signature: null, fittedUtc: null });
    expect(Object.isFrozen(record)).toBe(true);
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.size()).toEqual(1);
    expect(store.get("g", "other")).toBe(EMPTY_TRANSIT_FIT);
    expect(store.get("other", "d")).toBe(EMPTY_TRANSIT_FIT);

    store.setSetup("g", "d", OTHER_SETUP);
    expect(store.get("g", "d")).not.toBe(record);
    expect(store.get("g", "d").setup).toBe(OTHER_SETUP);
    expect(record.setup).toBe(S);
    expect(listener).toHaveBeenCalledTimes(2);
    const unsubscribed = vi.fn();
    store.subscribe(unsubscribed)();
    store.setSetup("g", "d", S);
    expect(unsubscribed).not.toHaveBeenCalled();
    expect(listener).toHaveBeenCalledTimes(3);
  });

  it("a fit is kept when the setup changes and dropped by clearResult", () => {
    const store = new TransitFitStore();
    const listener = vi.fn();
    store.subscribe(listener);
    store.setResult("g", "d", FIT);
    const fitted = store.get("g", "d");
    expect(fitted.request).toBe(REQ);
    expect(fitted.result).toBe(RESULT);
    expect(fitted.signature).toEqual("s1");
    expect(fitted.fittedUtc).toEqual("2026-11-13T12:00:00.000Z");
    expect(fitted.setup).toBe(DEFAULT_TRANSIT_FIT_SETUP);
    expect(Object.isFrozen(fitted)).toBe(true);
    expect(Object.keys(fitted).sort()).toEqual(["fittedUtc", "request", "result", "setup", "signature"]);
    expect(listener).toHaveBeenCalledTimes(1);

    store.setSetup("g", "d", S);
    expect(store.get("g", "d")).toEqual({ ...fitted, setup: S });
    expect(store.get("g", "d").result).toBe(RESULT);
    expect(store.get("g", "d").signature).toEqual("s1");
    expect(listener).toHaveBeenCalledTimes(2);

    store.setResult("g", "d", { ...FIT, signature: "s2" });
    expect(store.get("g", "d").setup).toBe(S);
    expect(store.get("g", "d").signature).toEqual("s2");
    expect(listener).toHaveBeenCalledTimes(3);

    store.clearResult("g", "d");
    expect(store.get("g", "d")).toEqual({ setup: S, request: null, result: null, signature: null, fittedUtc: null });
    expect(store.get("g", "d").setup).toBe(S);
    expect(Object.isFrozen(store.get("g", "d"))).toBe(true);
    expect(listener).toHaveBeenCalledTimes(4);

    store.clearResult("x", "y");
    expect(listener).toHaveBeenCalledTimes(4);
    expect(store.size()).toEqual(1);
    expect(store.get("x", "y")).toBe(EMPTY_TRANSIT_FIT);
  });

  it("the sixty-fifth record drops the one written longest ago", () => {
    expect(MAX_TRANSIT_FITS).toEqual(64);
    const store = new TransitFitStore();
    writeSetups(store, 0, 63);
    expect(store.size()).toEqual(64);
    expect(store.get("g", "d0").setup).toBe(S);
    writeSetups(store, 64, 64);
    expect(store.size()).toEqual(64);
    expect(store.get("g", "d0")).toBe(EMPTY_TRANSIT_FIT);
    expect(store.get("g", "d1").setup).toBe(S);
    expect(store.get("g", "d64").setup).toBe(S);

    writeSetups(store, 0, 0);
    expect(store.size()).toEqual(64);
    expect(store.get("g", "d1")).toBe(EMPTY_TRANSIT_FIT);
    expect(store.get("g", "d0").setup).toBe(S);
    expect(store.get("g", "d2").setup).toBe(S);

    const rewritten = new TransitFitStore();
    writeSetups(rewritten, 0, 63);
    rewritten.setResult("g", "d0", FIT);
    rewritten.clearResult("g", "d1");
    writeSetups(rewritten, 64, 65);
    expect(rewritten.size()).toEqual(64);
    expect(rewritten.get("g", "d0").result).toBe(RESULT);
    expect(rewritten.get("g", "d1").setup).toBe(S);
    expect(rewritten.get("g", "d2")).toBe(EMPTY_TRANSIT_FIT);
    expect(rewritten.get("g", "d3")).toBe(EMPTY_TRANSIT_FIT);
    expect(rewritten.get("g", "d4").setup).toBe(S);
  });

  it("removeGroup drops the records of that group only", () => {
    const store = new TransitFitStore();
    store.setSetup("tso:a", "white", S);
    store.setResult("tso:a", "bin1", FIT);
    store.setSetup("tso:ab", "white", OTHER_SETUP);
    const listener = vi.fn();
    store.subscribe(listener);
    store.removeGroup("tso:a");
    expect(store.size()).toEqual(1);
    expect(store.get("tso:a", "white")).toBe(EMPTY_TRANSIT_FIT);
    expect(store.get("tso:a", "bin1")).toBe(EMPTY_TRANSIT_FIT);
    expect(store.get("tso:ab", "white").setup).toBe(OTHER_SETUP);
    expect(listener).toHaveBeenCalledTimes(1);
    store.removeGroup("tso:a");
    store.removeGroup("tso:");
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.size()).toEqual(1);
  });
});

describe("transit fit hook", () => {
  it("the hook reads the store through useSyncExternalStore", () => {
    const group = "tso:transit-fit-hook-test";
    try {
      hooks.mount();
      transitFitStore.setResult(group, "white", FIT);
      transitFitStore.setSetup(group, "null", S);
      transitFitStore.setSetup("null", "white", S);
      const Probe = () => ({
        fit: useTransitFit(group, "white"),
        unknown: useTransitFit(group, "bin1"),
        noGroup: useTransitFit(null, "white"),
        noDoc: useTransitFit(group, null),
      });
      const first = hooks.render(Probe, undefined) as ReturnType<typeof Probe>;
      expect(first.fit).toBe(transitFitStore.get(group, "white"));
      expect(first.fit.result).toBe(RESULT);
      expect(first.unknown).toBe(EMPTY_TRANSIT_FIT);
      expect(first.noGroup).toBe(EMPTY_TRANSIT_FIT);
      expect(first.noDoc).toBe(EMPTY_TRANSIT_FIT);

      const notifiedBefore = hooks.run.notified;
      transitFitStore.setSetup(group, "bin1", S);
      expect(hooks.run.notified).toBeGreaterThan(notifiedBefore);
      const second = hooks.render(Probe, undefined) as ReturnType<typeof Probe>;
      expect(second.fit).toBe(first.fit);
      expect(second.unknown.setup).toBe(S);
      expect(second.noGroup).toBe(EMPTY_TRANSIT_FIT);
    } finally {
      hooks.mount();
      transitFitStore.removeGroup(group);
      transitFitStore.removeGroup("null");
    }
    expect(transitFitStore.size()).toEqual(0);
  });
});
