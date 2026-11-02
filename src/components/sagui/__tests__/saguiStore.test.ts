import { describe, expect, it, vi } from "vitest";
import { createSaguiStore, type SaguiApi } from "../saguiStore";
import type { SaguiInputInfo, SaguiResult, SaguiSavedResult } from "../../../shared/types/sagui";
import type { CubePrepReport } from "../../../shared/types/cubeprep";

const input: SaguiInputInfo = { width: 2, height: 2, bands: [{ filter: "F200W", pivot_um: 2 }, { filter: "F444W", pivot_um: 4.4 }], unit: "Jy/pixel", has_err: false, has_dq: true, estimated_memory_bytes: 200000000, warnings: ["ERR is absent"] };
const result: SaguiResult = { run_id: "id", unit: "Jy", output_dir: "/out/run", labels_path: "/out/run/labels.fits", mask_path: "/out/run/mask.fits", table_path: "/out/run/regions.csv", manifest_path: "/out/run/manifest.json", width: 2, height: 2, labels: [0, 1, 2, 2], regions: [], warnings: [] };
function setup() {
  const api: SaguiApi = {
    load: vi.fn(),
    inspectInput: vi.fn(async () => input),
    inspectRuntime: vi.fn(async () => ({ available: true, ready: true, r_version: "4.6.2", sagui_version: "0.0.0.9000", sagui_commit: "pinned", issues: [] })),
    run: vi.fn(async () => result), listen: vi.fn(async () => vi.fn()), cancel: vi.fn(async () => true),
    install: vi.fn(async () => ({ library_path: "/private/install-one/library", rscript_path: "/bin/Rscript", manifest_path: "/private/install-one/manifest.json", warnings: [] })),
    listenInstall: vi.fn(async () => vi.fn()), cancelInstall: vi.fn(async () => true),
  };
  const store = createSaguiStore(api, null);
  store.edit({ ...store.get().config, input_path: "/data/cube.fits", output_dir: "/out", rscript_path: "/bin/Rscript", library_path: "/r/library", psf_target: "F444W", n_regions: 2 });
  return { api, store };
}

describe("SAGUI desktop lifecycle", () => {
  it("restores a verified saved result and its settings without executing or checking R", async () => {
    const { store, api } = setup();
    const config = { ...store.get().config, input_path: "/old/science.fits", output_dir: "/old/jobs", n_regions: 7, starlet_scales: [2, 4] };
    const saved = { result: { ...result, output_dir: "/old/jobs/sagui-saved", manifest_path: "/old/jobs/sagui-saved/manifest.json" }, config };
    vi.mocked(api.load).mockResolvedValueOnce(saved);
    await store.restore(saved.result.manifest_path);
    expect(store.get()).toMatchObject({ status: "restored", operation: null, result: saved.result, config, input: null, runtime: null, outputDir: saved.result.output_dir });
    expect(api.load).toHaveBeenCalledWith(saved.result.manifest_path);
    expect(api.run).not.toHaveBeenCalled();
    expect(api.inspectInput).not.toHaveBeenCalled();
    expect(api.inspectRuntime).not.toHaveBeenCalled();
  });

  it("does not publish a late restored result over newer user settings", async () => {
    const { store, api } = setup();
    const saved = { result, config: { ...store.get().config } };
    let resolve!: (value: SaguiSavedResult) => void;
    vi.mocked(api.load).mockReturnValueOnce(new Promise((yes) => { resolve = yes; }));
    const pending = store.restore("/old/manifest.json");
    store.edit({ ...store.get().config, input_path: "/new/science.fits" });
    resolve(saved);
    await pending;
    expect(store.get().config.input_path).toBe("/new/science.fits");
    expect(store.get().result).toBeNull();
    expect(store.get().status).toBe("idle");
  });

  it("clears the displayed result when verification of a saved run fails", async () => {
    const { store, api } = setup();
    vi.mocked(api.load).mockResolvedValueOnce({ result, config: { ...store.get().config } });
    await store.restore("/old/manifest.json");
    vi.mocked(api.load).mockRejectedValueOnce(new Error("labels.fits SHA-256 mismatch"));
    await store.restore("/changed/manifest.json");
    expect(store.get().result).toBeNull();
    expect(store.get().status).toBe("error");
    expect(store.get().error).toContain("SHA-256 mismatch");
  });
  it("preflights a resolved Cube Prep product without starting a job or retaining an unrelated PSF", async () => {
    const { store, api } = setup();
    const report = { output_dir: "/prepared/run", cube: "08_export/final.fits", steps: [] } as unknown as CubePrepReport;
    expect(await store.prepareCubePrep(report)).toBe(true);
    expect(api.inspectInput).toHaveBeenCalledWith("/prepared/run/08_export/final.fits");
    expect(store.get().config).toMatchObject({ input_path: "/prepared/run/08_export/final.fits", psf_target: "", rscript_path: "/bin/Rscript", output_dir: "/out" });
    expect(store.get().input).toEqual(input);
    expect(api.run).not.toHaveBeenCalled();
    expect(api.inspectRuntime).not.toHaveBeenCalled();
  });

  it("rejects invalid and obsolete Cube Prep handoffs without changing the selected input", async () => {
    const { store, api } = setup();
    const report = { output_dir: "/prepared", cube: "/absolute/final.fits", steps: [] } as unknown as CubePrepReport;
    vi.mocked(api.inspectInput).mockRejectedValueOnce(new Error("SAGUI requires calibrated per-pixel flux units"));
    await expect(store.prepareCubePrep(report)).rejects.toThrow("per-pixel flux");
    expect(store.get().config.input_path).toBe("/data/cube.fits");
    let resolve!: (value: SaguiInputInfo) => void;
    vi.mocked(api.inspectInput).mockReturnValueOnce(new Promise((yes) => { resolve = yes; }));
    let current = true;
    const pending = store.prepareCubePrep(report, () => current);
    current = false;
    resolve(input);
    expect(await pending).toBe(false);
    expect(store.get().config.input_path).toBe("/data/cube.fits");
    expect(api.inspectInput).toHaveBeenLastCalledWith("/absolute/final.fits");
  });
  it("installs into a separate folder and adopts the library only after runtime verification", async () => {
    const { store, api } = setup();
    store.setInstallParent("/private");
    await store.install();
    expect(api.install).toHaveBeenCalledWith("/bin/Rscript", "/private", expect.any(String));
    expect(vi.mocked(api.listenInstall).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(api.install).mock.invocationCallOrder[0]);
    expect(api.inspectRuntime).toHaveBeenCalledWith("/bin/Rscript", "/private/install-one/library");
    expect(store.get().config.library_path).toBe("/private/install-one/library");
    expect(store.get().status).toBe("installed");
    expect(store.get().result).toBeNull();
  });

  it("retains the previous library if a new install is incomplete or incompatible", async () => {
    const { store, api } = setup();
    store.setInstallParent("/private");
    vi.mocked(api.inspectRuntime).mockResolvedValue({ available: true, ready: false, r_version: "4.6.2", sagui_version: null, sagui_commit: null, issues: ["SAGUI failed verification"] });
    await store.install();
    expect(store.get().config.library_path).toBe("/r/library");
    expect(store.get().status).toBe("error");
    expect(store.get().error).toContain("SAGUI failed verification");
  });

  it("cancels installation using its own event without starting segmentation", async () => {
    const { store, api } = setup();
    store.setInstallParent("/private");
    let reject!: (reason: unknown) => void;
    const stop = vi.fn();
    vi.mocked(api.listenInstall).mockResolvedValue(stop);
    vi.mocked(api.install).mockReturnValue(new Promise((_yes, no) => { reject = no; }));
    const work = store.install();
    await vi.waitFor(() => expect(api.install).toHaveBeenCalledOnce());
    await store.run();
    await store.cancel();
    expect(api.cancelInstall).toHaveBeenCalledWith(vi.mocked(api.install).mock.calls[0][2]);
    expect(api.cancel).not.toHaveBeenCalled();
    expect(api.run).not.toHaveBeenCalled();
    reject(new Error("Installation cancelled"));
    await work;
    expect(store.get().status).toBe("cancelled");
    expect(store.get().config.library_path).toBe("/r/library");
    expect(stop).toHaveBeenCalledOnce();
  });

  it("ignores a cancellation reply belonging to the previous run", async () => {
    const { api, store } = setup();
    let resolveFirst!: (value: SaguiResult) => void;
    let resolveSecond!: (value: SaguiResult) => void;
    let finishCancel!: (value: boolean) => void;
    vi.mocked(api.run).mockReturnValueOnce(new Promise((yes) => { resolveFirst = yes; })).mockReturnValueOnce(new Promise((yes) => { resolveSecond = yes; }));
    vi.mocked(api.cancel).mockReturnValue(new Promise((yes) => { finishCancel = yes; }));
    const first = store.run();
    await vi.waitFor(() => expect(api.run).toHaveBeenCalledTimes(1));
    const cancellation = store.cancel();
    resolveFirst(result);
    await first;
    const second = store.run();
    await vi.waitFor(() => expect(api.run).toHaveBeenCalledTimes(2));
    finishCancel(false);
    await cancellation;
    expect(store.get().status).toBe("running");
    expect(store.get().error).toBeNull();
    resolveSecond({ ...result, run_id: "second" });
    await second;
  });

  it("can diagnose R before selecting a cube", async () => {
    const { api, store } = setup();
    store.edit({ ...store.get().config, input_path: "" });
    await store.inspectRuntime();
    expect(api.inspectInput).not.toHaveBeenCalled();
    expect(store.get().runtime?.ready).toBe(true);
  });

  it("keeps the runner alive after panel unmount and releases its progress listener", async () => {
    const { api, store } = setup();
    const stop = vi.fn();
    vi.mocked(api.listen).mockResolvedValue(stop);
    let reject!: (error: unknown) => void;
    vi.mocked(api.run).mockReturnValue(new Promise((_resolve, no) => { reject = no; }));
    const unsubscribe = store.subscribe(vi.fn());
    const work = store.run();
    await vi.waitFor(() => expect(api.run).toHaveBeenCalledOnce());
    unsubscribe();
    const before = store.get().config;
    store.edit({ ...before, n_regions: 3 });
    expect(store.get().config).toBe(before);
    vi.mocked(api.cancel).mockResolvedValue(false);
    await store.cancel();
    expect(store.get().status).toBe("running");
    vi.mocked(api.cancel).mockResolvedValue(true);
    await store.cancel();
    expect(api.cancel).toHaveBeenCalledWith(vi.mocked(api.run).mock.calls[0][1]);
    reject(new Error("SAGUI cancelled; results were not published"));
    await work;
    expect(store.get().status).toBe("cancelled");
    expect(store.get().result).toBeNull();
    expect(stop).toHaveBeenCalledOnce();
  });

  it("requires a ready official runtime and never substitutes fake results", async () => {
    const { api, store } = setup();
    vi.mocked(api.inspectRuntime).mockResolvedValue({ available: false, ready: false, r_version: null, sagui_version: null, sagui_commit: null, issues: ["Rscript not installed"] });
    await store.run();
    expect(api.run).not.toHaveBeenCalled();
    expect(store.get().error).toContain("Rscript not installed");
    expect(store.get().result).toBeNull();
  });

  it("listens before launch, preserves unknown errors, and invalidates old segmentation on edit", async () => {
    const { api, store } = setup();
    await store.run();
    expect(vi.mocked(api.listen).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(api.run).mock.invocationCallOrder[0]);
    expect(store.get().result).toEqual(result);
    expect(store.get().unit).toBe("Jy/pixel");
    expect(vi.mocked(api.run).mock.calls[0][0].output_dir).toBe("/out");
    store.edit({ ...store.get().config, n_regions: 3 });
    expect(store.get().result).toBeNull();
    expect(store.get().input).toBeNull();
  });

  it("does not attach a late input inspection to a different cube", async () => {
    const { api, store } = setup();
    let resolve!: (value: SaguiInputInfo) => void;
    vi.mocked(api.inspectInput).mockReturnValue(new Promise((yes) => { resolve = yes; }));
    const work = store.inspect();
    store.edit({ ...store.get().config, input_path: "/data/other.fits" });
    resolve(input);
    await work;
    expect(store.get().input).toBeNull();
  });
});
