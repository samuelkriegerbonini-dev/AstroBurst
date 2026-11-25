import { describe, expect, it, vi } from "vitest";
import { createSaguiStore, isRecipePath, saguiConfigProblems, type SaguiApi } from "../saguiStore";
import type { SaguiInputInfo, SaguiProgress, SaguiResult, SaguiRuntimeInfo, SaguiSavedResult } from "../../../shared/types/sagui";
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
    expect(store.get()).toMatchObject({ status: "restored", operation: null, result: saved.result, config: { ...config, output_dir: "/out" }, input: null, runtime: null, outputDir: saved.result.output_dir });
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

  it("keeps a failure whose message names a cancelled folder as an error when nobody cancelled", async () => {
    const { api, store } = setup();
    const unreadable = "The input cube C:\\runs\\cancelled\\cube.fits is no longer readable; results were not published";
    vi.mocked(api.run).mockRejectedValueOnce(new Error(unreadable));
    await store.run();
    expect(store.get()).toMatchObject({ status: "error", error: unreadable, result: null });
    const denied = "Cannot create the SAGUI working folder D:\\cancelled\\install-1: Access is denied. (os error 5)";
    store.setInstallParent("D:\\cancelled");
    vi.mocked(api.install).mockRejectedValueOnce(new Error(denied));
    await store.install();
    expect(store.get()).toMatchObject({ status: "error", error: denied });
    expect(store.get().config.library_path).toBe("/r/library");
  });

  it("reports a job the user cancelled as cancelled whatever text the backend returns", async () => {
    const { api, store } = setup();
    let rejectRun!: (reason: unknown) => void;
    vi.mocked(api.run).mockReturnValueOnce(new Promise((_yes, no) => { rejectRun = no; }));
    const running = store.run();
    await vi.waitFor(() => expect(api.run).toHaveBeenCalledOnce());
    await store.cancel();
    rejectRun(new Error("Rscript failed (exit code: 1); see /out/stderr.log"));
    await running;
    expect(store.get()).toMatchObject({ status: "cancelled", error: null, result: null });
    store.setInstallParent("/private");
    let rejectInstall!: (reason: unknown) => void;
    vi.mocked(api.install).mockReturnValueOnce(new Promise((_yes, no) => { rejectInstall = no; }));
    const installing = store.install();
    await vi.waitFor(() => expect(api.install).toHaveBeenCalledOnce());
    await store.cancel();
    rejectInstall(new Error("Rscript failed (exit code: 1); see /private/stderr.log"));
    await installing;
    expect(store.get()).toMatchObject({ status: "cancelled", error: null });
    expect(store.get().config.library_path).toBe("/r/library");
  });

  it("treats a failure after a refused cancellation as an error", async () => {
    const { api, store } = setup();
    let reject!: (reason: unknown) => void;
    vi.mocked(api.run).mockReturnValueOnce(new Promise((_yes, no) => { reject = no; }));
    vi.mocked(api.cancel).mockResolvedValueOnce(false);
    const work = store.run();
    await vi.waitFor(() => expect(api.run).toHaveBeenCalledOnce());
    await store.cancel();
    expect(store.get().status).toBe("running");
    const unreadable = "The input cube /jobs/cancelled/cube.fits is no longer readable; results were not published";
    reject(new Error(unreadable));
    await work;
    expect(store.get()).toMatchObject({ status: "error", error: unreadable });
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
    expect(vi.mocked(api.run).mock.calls[0][0].output_dir).toBe("/out");
    store.edit({ ...store.get().config, n_regions: 3 });
    expect(store.get().result).toBeNull();
    expect(store.get().input).toEqual(input);
  });

  it("publishes the SAGUI run folder only once R starts (SCIENCE-04)", async () => {
    const { api, store } = setup();
    const folders: (string | null)[] = [];
    store.subscribe(() => folders.push(store.get().outputDir));
    vi.mocked(api.inspectRuntime).mockResolvedValueOnce({ available: false, ready: false, r_version: null, sagui_version: null, sagui_commit: null, issues: ["Rscript not installed"] });
    await store.run();
    expect(store.get().status).toBe("error");
    expect(folders.length).toBeGreaterThan(0);
    expect(folders.every((folder) => folder === null)).toBe(true);
    vi.mocked(api.run).mockImplementationOnce(async () => {
      expect(store.get()).toMatchObject({ status: "running", outputDir: null });
      return result;
    });
    await store.run();
    expect(store.get()).toMatchObject({ status: "complete", outputDir: "/out/run" });
    let resolveInput!: (value: SaguiInputInfo) => void;
    vi.mocked(api.inspectInput).mockReturnValueOnce(new Promise((yes) => { resolveInput = yes; }));
    store.edit({ ...store.get().config, input_path: "/data/next.fits" });
    store.edit({ ...store.get().config, psf_target: "F444W" });
    const early = store.run();
    expect(store.get().outputDir).toBeNull();
    await store.cancel();
    resolveInput(input);
    await early;
    expect(store.get()).toMatchObject({ status: "cancelled", outputDir: null });
  });

  it("clears only what an edited field invalidates (SCIENCE-05)", async () => {
    const { store } = setup();
    await store.run();
    const ran = store.get();
    expect(ran).toMatchObject({ result, outputDir: "/out/run", input });
    store.edit({ ...ran.config, output_dir: "/next" });
    expect(store.get()).toMatchObject({ result, outputDir: "/out/run", input, runtime: ran.runtime, status: "idle" });
    store.edit({ ...store.get().config, rscript_path: "/opt/Rscript" });
    expect(store.get()).toMatchObject({ result, input, runtime: null });
    store.edit({ ...store.get().config, n_regions: 3 });
    expect(store.get()).toMatchObject({ result: null, outputDir: null, input });
    store.edit({ ...store.get().config, input_path: "/data/other.fits" });
    expect(store.get().input).toBeNull();
  });

  it("keeps a verified installation and runtime through analysis edits (SCIENCE-05)", async () => {
    const { store } = setup();
    store.setInstallParent("/private");
    await store.install();
    const { installation, runtime } = store.get();
    expect(installation).not.toBeNull();
    store.edit({ ...store.get().config, n_regions: 5, starlet_scales: [1, 2] });
    expect(store.get().installation).toBe(installation);
    expect(store.get().runtime).toBe(runtime);
  });

  it("restores the saved analysis and input but keeps the current R paths and working folder (SCIENCE-08)", async () => {
    const { store, api } = setup();
    await store.inspectRuntime();
    const runtime = store.get().runtime;
    const config = { ...store.get().config, input_path: "/old/science.fits", output_dir: "/old/jobs", rscript_path: "/old/bin/Rscript", library_path: "/old/library", n_regions: 7, mode: "sparse" as const, dq_exclude_bits: 3 };
    vi.mocked(api.load).mockResolvedValueOnce({ result: { ...result, output_dir: "/old/jobs/sagui-saved" }, config });
    await store.restore("/old/jobs/sagui-saved/manifest.json");
    expect(store.get().config).toEqual({ ...config, output_dir: "/out", rscript_path: "/bin/Rscript", library_path: "/r/library" });
    expect(store.get()).toMatchObject({ status: "restored", runtime, outputDir: "/old/jobs/sagui-saved" });
  });

  it("keeps the previous result and input while a Cube Prep hand-off is checked or dropped (SCIENCE-14)", async () => {
    const { api, store } = setup();
    await store.run();
    const before = store.get();
    const report = { output_dir: "/prepared/run", cube: "final.fits", steps: [] } as unknown as CubePrepReport;
    let resolve!: (value: SaguiInputInfo) => void;
    vi.mocked(api.inspectInput).mockReturnValueOnce(new Promise((yes) => { resolve = yes; }));
    let current = true;
    const pending = store.prepareCubePrep(report, () => current);
    const kept = { result, input: before.input, outputDir: "/out/run", config: before.config };
    expect(store.get()).toMatchObject({ status: "checking", ...kept });
    current = false;
    resolve({ ...input, unit: "MJy/sr" });
    expect(await pending).toBe(false);
    expect(store.get()).toMatchObject({ status: "idle", ...kept });
    vi.mocked(api.inspectInput).mockRejectedValueOnce(new Error("SAGUI requires a multiband cube"));
    await expect(store.prepareCubePrep(report)).rejects.toThrow("multiband");
    expect(store.get()).toMatchObject({ status: "error", ...kept });
    expect(await store.prepareCubePrep(report)).toBe(true);
    expect(store.get()).toMatchObject({ status: "idle", result: null, outputDir: null, input });
    expect(store.get().config.input_path).toBe("/prepared/run/final.fits");
  });

  it("excludes only DO_NOT_USE by default and keeps a stored mask (OD-L009 a)", () => {
    const { api } = setup();
    expect(createSaguiStore(api, null).get().config.dq_exclude_bits).toBe(1);
    const stored = { input_path: "/data/cube.fits", output_dir: "/out", rscript_path: "", library_path: "", dq_exclude_bits: 4294967295 };
    const storage = { getItem: (key: string) => key === "ab.sagui.v1" ? JSON.stringify(stored) : null, setItem: vi.fn() };
    expect(createSaguiStore(api, storage).get().config.dq_exclude_bits).toBe(4294967295);
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

describe("SAGUI fixes (PREPUI)", () => {
  it("clears a PSF target that was stated for another cube (PREPUI-04)", () => {
    const { store } = setup();
    expect(store.get().config.psf_target).toBe("F444W");
    store.edit({ ...store.get().config, input_path: "/data/other.fits" });
    expect(store.get().config.psf_target).toBe("");
    store.edit({ ...store.get().config, input_path: "/data/third.fits", psf_target: "F200W" });
    expect(store.get().config.psf_target).toBe("F200W");
    store.edit({ ...store.get().config, n_regions: 4 });
    expect(store.get().config.psf_target).toBe("F200W");
  });

  it("sets the union of the Cube Prep DQ masks when the handed-off cube mixes HST and JWST planes (PREPUI-07)", async () => {
    const { store, api } = setup();
    const report = { output_dir: "/prepared", cube: "final.fits", steps: [] } as unknown as CubePrepReport;
    vi.mocked(api.inspectInput).mockResolvedValueOnce({ ...input, dq_flags: [{ filter: "F814W", table: "hst", flagged_finite: { COSMIC_RAY: 12 } }, { filter: "F444W", table: "jwst", flagged_finite: { DO_NOT_USE: 3 } }] });
    await store.prepareCubePrep(report);
    expect(store.get().config.dq_exclude_bits).toBe(16383);
    vi.mocked(api.inspectInput).mockResolvedValueOnce({ ...input, dq_flags: [{ filter: "F200W", table: "jwst", flagged_finite: {} }, { filter: "F444W", table: "jwst", flagged_finite: {} }] });
    await store.prepareCubePrep(report);
    expect(store.get().config.dq_exclude_bits).toBe(1);
    store.edit({ ...store.get().config, dq_exclude_bits: 7 });
    vi.mocked(api.inspectInput).mockResolvedValueOnce(input);
    await store.prepareCubePrep(report);
    expect(store.get().config.dq_exclude_bits).toBe(7);
  });

  it("does not read the cube and start R again for a run right after a check (PREPUI-09)", async () => {
    const { store, api } = setup();
    await store.inspect();
    await store.run();
    expect(api.inspectInput).toHaveBeenCalledOnce();
    expect(api.inspectRuntime).toHaveBeenCalledOnce();
    expect(store.get()).toMatchObject({ status: "complete", result });
    store.edit({ ...store.get().config, input_path: "/data/other.fits" });
    store.edit({ ...store.get().config, psf_target: "F444W" });
    await store.run();
    expect(api.inspectInput).toHaveBeenCalledTimes(2);
    expect(api.inspectInput).toHaveBeenLastCalledWith("/data/other.fits");
  });

  it("names the fix of each runtime issue when Run refuses the runtime (PREPUI-13)", async () => {
    const { store, api } = setup();
    vi.mocked(api.inspectRuntime).mockResolvedValue({ available: true, ready: false, r_version: "4.5.1", sagui_version: "0.0.0.9000", sagui_commit: "other", issues: ["Expected R 4.6.2; found 4.5.1", "SAGUI does not match the pinned upstream commit"] });
    await store.run();
    expect(api.run).not.toHaveBeenCalled();
    expect(store.get().error).toBe("Expected R 4.6.2; found 4.5.1: install R 4.6.2 alongside your current R (the Windows installer is in the CRAN archive) and choose its Rscript. SAGUI does not match the pinned upstream commit: use Install / repair SAGUI library to build the pinned commit into a new private library.");
  });

  it("reports an installed library when Cancel is pressed during the check that follows the installation (PREPUI-14)", async () => {
    const { store, api } = setup();
    store.setInstallParent("/private");
    let verify!: (value: SaguiRuntimeInfo) => void;
    vi.mocked(api.inspectRuntime).mockReturnValueOnce(new Promise((yes) => { verify = yes; }));
    const work = store.install();
    await vi.waitFor(() => expect(api.inspectRuntime).toHaveBeenCalledOnce());
    await store.cancel();
    verify({ available: true, ready: true, r_version: "4.6.2", sagui_version: "0.0.0.9000", sagui_commit: "pinned", issues: [] });
    await work;
    expect(store.get()).toMatchObject({ status: "installed", config: { library_path: "/private/install-one/library" } });
    expect(api.cancelInstall).not.toHaveBeenCalled();
    expect(store.canCancel()).toBe(false);
  });

  it("names the run folder only once the backend reports that R started (PREPUI-15)", async () => {
    const { store, api } = setup();
    let progress!: (value: SaguiProgress) => void;
    vi.mocked(api.listen).mockImplementation(async (_id, callback) => { progress = callback; return vi.fn(); });
    let finish!: (value: SaguiResult) => void;
    vi.mocked(api.run).mockReturnValueOnce(new Promise((yes) => { finish = yes; }));
    const work = store.run();
    await vi.waitFor(() => expect(api.run).toHaveBeenCalledOnce());
    expect(store.get()).toMatchObject({ status: "running", outputDir: null });
    progress({ stage: "Checking calibrated input and runtime", current: 0, total: 5, percent: 0 });
    expect(store.get().outputDir).toBeNull();
    progress({ stage: "Running official SAGUI in isolated R", current: 1, total: 5, percent: 20 });
    expect(store.get().outputDir).toBe(`/out/sagui-${vi.mocked(api.run).mock.calls[0][1]}`);
    finish(result);
    await work;
    expect(store.get().outputDir).toBe("/out/run");
  });

  it("says that no run folder exists when the run is cancelled before R starts (PREPUI-15)", async () => {
    const { store, api } = setup();
    let reject!: (reason: unknown) => void;
    vi.mocked(api.run).mockReturnValueOnce(new Promise((_yes, no) => { reject = no; }));
    const work = store.run();
    await vi.waitFor(() => expect(api.run).toHaveBeenCalledOnce());
    await store.cancel();
    reject(new Error("SAGUI cancelled"));
    await work;
    expect(store.get()).toMatchObject({ status: "cancelled", outputDir: null });
  });

  it("checks a newly chosen cube without starting R (proposal 14)", async () => {
    const { store, api } = setup();
    await store.inspectCube();
    expect(api.inspectInput).toHaveBeenCalledWith("/data/cube.fits");
    expect(api.inspectRuntime).not.toHaveBeenCalled();
    expect(store.get()).toMatchObject({ status: "idle", input });
    store.edit({ ...store.get().config, input_path: "/data/recipe.json" });
    await store.inspectCube();
    expect(api.inspectInput).toHaveBeenCalledOnce();
  });

  it("restores stored numbers that are not valid to their defaults and words a fractional region count (PREPUI-23)", () => {
    const stored = { input_path: "/data/cube.fits", output_dir: "/out", rscript_path: "", library_path: "", n_regions: null, starlet_threshold: null, dq_exclude_bits: 3, knn_k: "x", starlet_scales: null };
    const storage = { getItem: (key: string) => key === "ab.sagui.v1" ? JSON.stringify(stored) : null, setItem: vi.fn() };
    expect(createSaguiStore(setup().api, storage).get().config).toMatchObject({ n_regions: 20, starlet_threshold: 3, dq_exclude_bits: 3, knn_k: 40, starlet_scales: [1, 2, 3] });
    const ready = { input_path: "C:\\runs\\cube.fits", output_dir: "C:\\runs", rscript_path: "Rscript", library_path: "lib", n_regions: 6, starlet: true, starlet_scales: [1, 2, 3], starlet_threshold: 3, dq_exclude_bits: 1, psf_target: "F444W", mode: "ward" as const, knn_k: 40 };
    expect(saguiConfigProblems({ ...ready, n_regions: 2.5 })).toEqual(["Number of regions must be a whole number."]);
    expect(saguiConfigProblems({ ...ready, n_regions: 1 })).toEqual(["Choose at least two regions."]);
    expect(saguiConfigProblems({ ...ready, n_regions: Number.NaN })).toEqual(["Enter the number of regions."]);
  });
});

describe("SAGUI cube field", () => {
  const ready = { input_path: "C:\runs\cube.fits", output_dir: "C:\runs", rscript_path: "Rscript", library_path: "lib", n_regions: 6, starlet: true, starlet_scales: [1, 2, 3], starlet_threshold: 3, dq_exclude_bits: 1, psf_target: "F444W", mode: "ward" as const, knn_k: 40 };
  it("asks for a FITS cube and names a Cube Prep recipe put in its place", () => {
    expect(saguiConfigProblems(ready)).toEqual([]);
    for (const path of ["C:\runs\cube.fits.gz", "/data/cube.FTS", "/data/cube.fit.fz"]) expect(saguiConfigProblems({ ...ready, input_path: path })).toEqual([]);
    expect(saguiConfigProblems({ ...ready, input_path: "C:\astrokit\entregas\desktop-d1-scientific-recipe.json" })).toEqual(["The cube field holds a JSON file. A Cube Prep recipe opens in Cube Prep; choose its prepared FITS cube here."]);
    expect(saguiConfigProblems({ ...ready, input_path: "C:\runs\regions.csv" })).toEqual(["Choose a FITS cube: .fits, .fit or .fts, optionally compressed as .gz, .bz2 or .fz."]);
    expect(saguiConfigProblems({ ...ready, input_path: " " })).toEqual(["Choose a prepared multiband FITS cube."]);
  });

  it("recognises a recipe path regardless of case and spaces", () => {
    expect(isRecipePath(" C:\recipes\D1.JSON ")).toBe(true);
    expect(isRecipePath("C:\runs\cube.fits")).toBe(false);
    expect(isRecipePath("")).toBe(false);
  });
});
