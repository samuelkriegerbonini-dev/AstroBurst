import { describe, expect, it, vi } from "vitest";
import { createCubePrepStore, type CubePrepApi } from "../cubePrepStore";
import { createCubePrepDraft } from "../cubePrepModel";
import type { CubePrepProgress, CubePrepReport, CubePrepValidation } from "../../../shared/types/cubeprep";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

const report: CubePrepReport = { recipe_version: 1, name: "D1", output_dir: "/data/job", cube: "/data/job/cube.fits", steps: [], files: [], warnings: [], elapsed_ms: 10 };

function setup() {
  const draft = createCubePrepDraft();
  draft.recipe.inputs = [{ path: "/data/a.fits", dataset: "JWST", filter: "F200W" }];
  draft.recipe.steps = [{ step: "cube", reference: "F200W", layout: "mef", output: "cube.fits" }];
  draft.outputParent = "/data";
  draft.baseDir = "/data";
  const validation: CubePrepValidation = { recipe: draft.recipe, dependencies: ["/data/a.fits"], warnings: ["Check registration"] };
  const stop = vi.fn();
  let progress: ((p: CubePrepProgress) => void) | undefined;
  const api: CubePrepApi = {
    validate: vi.fn(async () => validation),
    run: vi.fn(async () => report),
    listen: vi.fn(async (_id, cb) => { progress = cb; return stop; }),
    cancel: vi.fn(async () => true),
  };
  const store = createCubePrepStore(api, null);
  store.edit(draft);
  return { store, api, stop, validation, progress: (p: CubePrepProgress) => progress?.(p) };
}

describe("Cube Prep desktop job lifecycle", () => {
  it("does not revive a completed run when a late cancellation request fails", async () => {
    const { store, api } = setup();
    const execution = deferred<CubePrepReport>();
    const cancellation = deferred<boolean>();
    vi.mocked(api.run).mockReturnValue(execution.promise);
    vi.mocked(api.cancel).mockReturnValue(cancellation.promise);
    const work = store.run();
    await vi.waitFor(() => expect(api.run).toHaveBeenCalledOnce());
    const cancel = store.cancel();
    execution.resolve(report);
    await work;
    cancellation.reject(new Error("Event already closed"));
    await cancel;
    expect(store.get().status).toBe("complete");
  });

  it("invalidates displayed products when the configured input changes", async () => {
    const { store } = setup();
    await store.run();
    store.edit({ ...store.get().draft, baseDir: "/different" });
    expect(store.get().report).toBeNull();
    expect(store.get().outputDir).toBeNull();
    expect(store.get().validation).toBeNull();
  });

  it("allows cancellation retry when no backend job accepted the request", async () => {
    const { store, api } = setup();
    const pending = deferred<CubePrepReport>();
    vi.mocked(api.run).mockReturnValue(pending.promise);
    vi.mocked(api.cancel).mockResolvedValue(false);
    const work = store.run();
    await vi.waitFor(() => expect(api.run).toHaveBeenCalledOnce());
    await store.cancel();
    expect(store.get().status).toBe("running");
    expect(store.get().error).toMatch(/retry/i);
    pending.resolve(report);
    await work;
  });

  it("validates before listening, then runs the resolved recipe and closes the listener", async () => {
    const { store, api, stop, validation } = setup();
    await store.run();
    expect(api.validate).toHaveBeenCalledOnce();
    expect(api.run).toHaveBeenCalledWith(validation.recipe, "/data", expect.stringContaining("/data/cubeprep-"), expect.any(String));
    expect(vi.mocked(api.listen).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(api.run).mock.invocationCallOrder[0]);
    expect(store.get().status).toBe("complete");
    expect(store.get().report).toEqual(report);
    expect(stop).toHaveBeenCalledOnce();
  });

  it("does not run a rejected recipe", async () => {
    const { store, api } = setup();
    vi.mocked(api.validate).mockRejectedValue(new Error("Missing ERR"));
    await store.run();
    expect(store.get().error).toBe("Missing ERR");
    expect(api.listen).not.toHaveBeenCalled();
    expect(api.run).not.toHaveBeenCalled();
  });

  it("ignores a late validation response after editing", async () => {
    const { store, api, validation } = setup();
    const pending = deferred<CubePrepValidation>();
    vi.mocked(api.validate).mockReturnValue(pending.promise);
    const work = store.validate();
    store.edit({ ...store.get().draft, baseDir: "/other" });
    pending.resolve(validation);
    await work;
    expect(store.get().validation).toBeNull();
    expect(store.get().status).toBe("idle");
  });

  it("continues with no panel subscribers and rejects overlapping runs and edits", async () => {
    const { store, api, progress, stop } = setup();
    const pending = deferred<CubePrepReport>();
    vi.mocked(api.run).mockReturnValue(pending.promise);
    const unsub = store.subscribe(vi.fn());
    const work = store.run();
    await vi.waitFor(() => expect(api.run).toHaveBeenCalledOnce());
    unsub();
    const before = store.get().draft;
    store.edit({ ...before, outputParent: "/changed" });
    await store.run();
    progress({ current: 1, total: 3, percent: 33, stage: "align" });
    expect(store.get().draft).toBe(before);
    expect(store.get().progress?.stage).toBe("align");
    pending.resolve(report);
    await work;
    expect(api.run).toHaveBeenCalledOnce();
    expect(stop).toHaveBeenCalledOnce();
  });

  it("cancels by the matching run event and retains partial output location", async () => {
    const { store, api, stop } = setup();
    const pending = deferred<CubePrepReport>();
    vi.mocked(api.run).mockReturnValue(pending.promise);
    const work = store.run();
    await vi.waitFor(() => expect(api.run).toHaveBeenCalledOnce());
    const runId = vi.mocked(api.run).mock.calls[0][3];
    await store.cancel();
    expect(api.cancel).toHaveBeenCalledWith(runId);
    pending.reject(Object.assign(new Error("Operation cancelled"), { code: "CANCELLED" }));
    await work;
    expect(store.get().status).toBe("cancelled");
    expect(store.get().outputDir).toContain("/data/cubeprep-");
    expect(stop).toHaveBeenCalledOnce();
  });

  it("publishes the run folder only when the backend run starts (SCIENCE-03)", async () => {
    const { store, api, validation } = setup();
    await store.run();
    expect(store.get().outputDir).toBe("/data/job");
    const refused = deferred<CubePrepValidation>();
    vi.mocked(api.validate).mockReturnValueOnce(refused.promise);
    const work = store.run();
    expect(store.get()).toMatchObject({ status: "validating", outputDir: null, report: null });
    refused.reject(new Error("Missing ERR"));
    await work;
    expect(store.get()).toMatchObject({ status: "error", error: "Missing ERR", outputDir: null });
    const early = deferred<CubePrepValidation>();
    vi.mocked(api.validate).mockReturnValueOnce(early.promise);
    const cancelled = store.run();
    await store.cancel();
    early.resolve(validation);
    await cancelled;
    expect(store.get()).toMatchObject({ status: "cancelled", outputDir: null });
    expect(api.run).toHaveBeenCalledOnce();
    vi.mocked(api.run).mockImplementationOnce(async (_recipe, _base, outputDir) => {
      expect(outputDir).toMatch(/^\/data\/cubeprep-field-/);
      expect(store.get()).toMatchObject({ status: "running", outputDir });
      expect(store.get().startedAt).toEqual(expect.any(Number));
      return report;
    });
    await store.run();
    expect(store.get()).toMatchObject({ status: "complete", outputDir: "/data/job" });
  });

  it("keeps the report when only the name, working folder or profile label change (SCIENCE-06)", async () => {
    const { store } = setup();
    await store.run();
    const draft = store.get().draft;
    store.edit({ ...draft, outputParent: "/elsewhere", profile: "scientific", recipe: { ...draft.recipe, name: "renamed" } });
    expect(store.get()).toMatchObject({ report, outputDir: "/data/job", validation: null, status: "idle" });
    store.edit({ ...store.get().draft, recipe: { ...store.get().draft.recipe, steps: [{ step: "cube", layout: "lightstack", output: "cube.fits" }] } });
    expect(store.get()).toMatchObject({ report: null, outputDir: null });
  });

  it("persists configuration without reviving a running job after restart", async () => {
    const data = new Map<string, string>();
    const storage = { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => { data.set(k, v); } };
    const { api, store: source } = setup();
    const first = createCubePrepStore(api, storage);
    first.edit(source.get().draft);
    await first.run();
    const restored = createCubePrepStore(api, storage);
    expect(restored.get().draft.outputParent).toBe("/data");
    expect(restored.get().report).toEqual(report);
    expect(restored.get().status).not.toBe("running");
  });
});
