import { describe, expect, it } from "vitest";
import { DepthStore, EMPTY_DEPTH, MAX_DEPTH_DOCS, depthKey } from "../useDepthStore";
import type { ImageDepth, ImageDepthOptions } from "../../shared/types/depth";

const depthOf = (limitAb: number) => ({ limit_ab: limitAb }) as ImageDepth;

function manualRun() {
  const calls: { path: string; options: ImageDepthOptions; resolve: (depth: ImageDepth) => void; reject: (error: unknown) => void }[] = [];
  const run = (path: string, options: ImageDepthOptions) =>
    new Promise<ImageDepth>((resolve, reject) => {
      calls.push({ path, options, resolve, reject });
    });
  return { calls, run };
}

const NO_WCS = "The image has no celestial WCS: give the aperture radius in pixels";

describe("DepthStore", () => {
  it("measures once per key and keeps the result", async () => {
    const { calls, run } = manualRun();
    const store = new DepthStore(run);
    let notified = 0;
    store.subscribe(() => notified++);
    const pending = store.measure("a|", "a", {});
    expect(store.getSnapshot("a|")).toEqual({ depth: null, loading: true, error: null });
    expect(calls).toHaveLength(1);
    expect(calls[0].path).toBe("a");
    expect(calls[0].options).toEqual({});
    calls[0].resolve(depthOf(25.5));
    await pending;
    expect(store.getSnapshot("a|")).toEqual({ depth: depthOf(25.5), loading: false, error: null, options: {} });
    expect(notified).toBe(2);
    expect(store.getSnapshot(null)).toBe(EMPTY_DEPTH);
    expect(store.getSnapshot("b|")).toBe(EMPTY_DEPTH);
    expect(EMPTY_DEPTH).toEqual({ depth: null, loading: false, error: null });
    expect(Object.isFrozen(EMPTY_DEPTH)).toBe(true);
    expect(depthKey("a", null)).toBe("a|");
    expect(depthKey("a", undefined)).toBe("a|");
    expect(depthKey("a", "v2")).toBe("a|v2");

    const again = store.measure("a|", "a", { nSigma: 3 });
    expect(store.getSnapshot("a|")).toEqual({ depth: depthOf(25.5), loading: true, error: null, options: {} });
    calls[1].resolve(depthOf(26));
    await again;
    expect(store.getSnapshot("a|").depth).toEqual(depthOf(26));
    expect(store.getSnapshot("a|").options).toEqual({ nSigma: 3 });
  });

  it("errors, stale answers and the cache limit", async () => {
    const { calls, run } = manualRun();
    const store = new DepthStore(run);
    const first = store.measure("a|", "a", {});
    calls[0].resolve(depthOf(25));
    await first;
    const failing = store.measure("a|", "a", {});
    calls[1].reject(new Error(NO_WCS));
    await failing;
    expect(store.getSnapshot("a|")).toEqual({ depth: null, loading: false, error: NO_WCS });

    const older = store.measure("b|", "b", { seed: 1 });
    const newer = store.measure("b|", "b", { seed: 2 });
    calls[3].resolve(depthOf(27));
    await newer;
    calls[2].resolve(depthOf(20));
    await older;
    expect(store.getSnapshot("b|")).toEqual({ depth: depthOf(27), loading: false, error: null, options: { seed: 2 } });

    const forgotten = store.measure("c|", "c", {});
    store.forget("c|");
    expect(store.getSnapshot("c|")).toBe(EMPTY_DEPTH);
    calls[4].resolve(depthOf(24));
    await forgotten;
    expect(store.getSnapshot("c|")).toBe(EMPTY_DEPTH);

    const fresh = new DepthStore(async (path) => depthOf(path.length));
    const keys = Array.from({ length: MAX_DEPTH_DOCS + 1 }, (_, i) => `k${i}|`);
    for (const key of keys) await fresh.measure(key, key.slice(0, -1), {});
    expect(MAX_DEPTH_DOCS).toBe(8);
    expect(fresh.getSnapshot(keys[0])).toBe(EMPTY_DEPTH);
    for (const key of keys.slice(1)) expect(fresh.getSnapshot(key).depth).not.toBeNull();
  });

  it("latest gives the depth of the same data revision only, never one of an earlier revision of the path", async () => {
    const { calls, run } = manualRun();
    const store = new DepthStore(run);
    const before = store.measure(depthKey("a.fits", "a.fits"), "a.fits", {});
    calls[0].resolve(depthOf(25));
    await before;
    expect(store.latest("a.fits", "a.fits")).toEqual(depthOf(25));
    expect(store.latest("a.fits", "a.fits#1")).toBeNull();
    expect(store.latest("b.fits", "b.fits")).toBeNull();

    const rewritten = store.measure(depthKey("a.fits", "a.fits#1"), "a.fits", {});
    calls[1].resolve(depthOf(26));
    await rewritten;
    expect(store.latest("a.fits", "a.fits#1")).toEqual(depthOf(26));
    expect(store.latest("a.fits", "a.fits")).toEqual(depthOf(25));

    const failed = store.measure(depthKey("a.fits", "a.fits#2"), "a.fits", {});
    calls[2].reject(new Error(NO_WCS));
    await failed;
    expect(store.latest("a.fits", "a.fits#2")).toBeNull();

    const other = store.measure(depthKey("a.fits.gz", null), "a.fits.gz", {});
    calls[3].resolve(depthOf(22));
    await other;
    expect(store.latest("a.fits", null)).toBeNull();
  });
});
