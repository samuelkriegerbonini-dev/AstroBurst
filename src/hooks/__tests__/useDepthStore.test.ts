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
    expect(store.getSnapshot("a|")).toEqual({ depth: depthOf(25.5), loading: false, error: null });
    expect(notified).toBe(2);
    expect(store.getSnapshot(null)).toBe(EMPTY_DEPTH);
    expect(store.getSnapshot("b|")).toBe(EMPTY_DEPTH);
    expect(EMPTY_DEPTH).toEqual({ depth: null, loading: false, error: null });
    expect(Object.isFrozen(EMPTY_DEPTH)).toBe(true);
    expect(depthKey("a", null)).toBe("a|");
    expect(depthKey("a", undefined)).toBe("a|");
    expect(depthKey("a", "v2")).toBe("a|v2");

    const again = store.measure("a|", "a", { nSigma: 3 });
    expect(store.getSnapshot("a|")).toEqual({ depth: depthOf(25.5), loading: true, error: null });
    calls[1].resolve(depthOf(26));
    await again;
    expect(store.getSnapshot("a|").depth).toEqual(depthOf(26));
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
    expect(store.getSnapshot("b|")).toEqual({ depth: depthOf(27), loading: false, error: null });

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

  it("latest gives the newest depth of a file whatever the measure key", async () => {
    const { calls, run } = manualRun();
    const store = new DepthStore(run);
    const v1 = store.measure("a|v1", "a", {});
    calls[0].resolve(depthOf(25));
    await v1;
    expect(store.latest("a")).toEqual(depthOf(25));
    const v2 = store.measure("a|v2", "a", {});
    calls[1].resolve(depthOf(26));
    await v2;
    expect(store.latest("a")).toEqual(depthOf(26));
    expect(store.latest("b")).toBeNull();
    expect(store.latest("a|")).toBeNull();

    const v3 = store.measure("a|v3", "a", {});
    calls[2].reject(new Error(NO_WCS));
    await v3;
    expect(store.latest("a")).toEqual(depthOf(26));

    const other = store.measure("ab|", "ab", {});
    calls[3].resolve(depthOf(22));
    await other;
    expect(store.latest("a")).toEqual(depthOf(26));
  });
});
