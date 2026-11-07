import { useSyncExternalStore } from "react";
import { cancelSagui, cancelSaguiInstall, inspectSaguiInput, inspectSaguiRuntime, installSaguiRuntime, loadSaguiResult, onSaguiInstallProgress, onSaguiProgress, runSagui } from "../../services/sagui";
import type { SaguiConfig, SaguiInputInfo, SaguiInstallation, SaguiProgress, SaguiResult, SaguiRuntimeInfo, SaguiSavedResult } from "../../shared/types/sagui";
import type { CubePrepReport } from "../../shared/types/cubeprep";
import { cubePrepError, cubePrepPsfTarget, joinProductPath } from "../cubeprep/cubePrepModel";

export interface SaguiApi {
  load(manifestPath: string): Promise<SaguiSavedResult>;
  inspectInput(path: string): Promise<SaguiInputInfo>;
  inspectRuntime(rscriptPath: string, libraryPath: string): Promise<SaguiRuntimeInfo>;
  run(config: SaguiConfig, runId: string): Promise<SaguiResult>;
  listen(runId: string, callback: (progress: SaguiProgress) => void): Promise<() => void>;
  cancel(runId: string): Promise<boolean>;
  install(rscriptPath: string, outputDir: string, runId: string): Promise<SaguiInstallation>;
  listenInstall(runId: string, callback: (progress: SaguiProgress) => void): Promise<() => void>;
  cancelInstall(runId: string): Promise<boolean>;
}

export interface SaguiState {
  config: SaguiConfig;
  input: SaguiInputInfo | null;
  runtime: SaguiRuntimeInfo | null;
  status: "idle" | "checking" | "running" | "installing" | "installed" | "cancelling" | "cancelled" | "complete" | "restored" | "error";
  operation: "segment" | "install" | "restore" | null;
  installParent: string;
  installation: SaguiInstallation | null;
  progress: SaguiProgress | null;
  result: SaguiResult | null;
  outputDir: string | null;
  unit: string | null;
  error: string | null;
}

const DEFAULT_CONFIG: SaguiConfig = { input_path: "", output_dir: "", rscript_path: "", library_path: "", n_regions: 20, starlet: true, starlet_scales: [1, 2, 3], starlet_threshold: 3, dq_exclude_bits: 1, psf_target: "", mode: "ward", knn_k: 40 };
const RESULT_INDEPENDENT_KEYS = new Set<keyof SaguiConfig>(["output_dir", "rscript_path", "library_path"]);
const API: SaguiApi = { load: loadSaguiResult, inspectInput: inspectSaguiInput, inspectRuntime: inspectSaguiRuntime, run: runSagui, listen: onSaguiProgress, cancel: cancelSagui, install: installSaguiRuntime, listenInstall: onSaguiInstallProgress, cancelInstall: cancelSaguiInstall };
type SaguiStorage = Pick<Storage, "getItem" | "setItem">;
function browserStorage(): SaguiStorage | null { try { return typeof window === "undefined" ? null : window.localStorage; } catch { return null; } }

export function saguiConfigProblems(config: SaguiConfig): string[] {
  const problems: string[] = [];
  if (!config.input_path.trim()) problems.push("Choose a prepared multiband FITS cube.");
  if (!config.rscript_path.trim() || !config.library_path.trim()) problems.push("Choose Rscript and its private SAGUI library.");
  if (!config.output_dir.trim()) problems.push("Choose a working folder.");
  if (!config.psf_target.trim()) problems.push("Identify the verified common PSF target.");
  if (!Number.isInteger(config.n_regions) || config.n_regions < 2) problems.push("Choose at least two regions.");
  if (!Number.isInteger(config.dq_exclude_bits) || config.dq_exclude_bits < 0 || config.dq_exclude_bits > 0xffffffff) problems.push("DQ bits must be an unsigned 32-bit integer.");
  if (!config.starlet_scales.length || config.starlet_scales.some((scale) => !Number.isInteger(scale) || scale < 1 || scale > 10)) problems.push("Starlet scales must be integers from 1 to 10.");
  if (!Number.isFinite(config.starlet_threshold) || config.starlet_threshold <= 0) problems.push("Starlet threshold must be positive.");
  if (!Number.isInteger(config.knn_k) || config.knn_k < 2 || config.knn_k > 4096) problems.push("Sparse neighbours must be between 2 and 4096.");
  return problems;
}

export function createSaguiStore(api: SaguiApi = API, storage: SaguiStorage | null = browserStorage()) {
  let config = { ...DEFAULT_CONFIG };
  let installParent = "";
  try {
    const saved = JSON.parse(storage?.getItem("ab.sagui.v1") ?? "null") as Partial<SaguiConfig> | null;
    if (saved && typeof saved.input_path === "string" && typeof saved.rscript_path === "string" && typeof saved.library_path === "string" && typeof saved.output_dir === "string") config = { ...config, ...saved };
    installParent = storage?.getItem("ab.sagui.installParent.v1") ?? "";
  } catch { config = { ...DEFAULT_CONFIG }; }
  let state: SaguiState = { config, input: null, runtime: null, status: "idle", operation: null, installParent, installation: null, progress: null, result: null, outputDir: null, unit: null, error: null };
  let revision = 0;
  let active: string | null = null;
  let cancelled = false;
  const subscribers = new Set<() => void>();
  const patch = (change: Partial<SaguiState>) => { state = { ...state, ...change }; subscribers.forEach((listener) => listener()); };
  const check = async (candidate: SaguiConfig) => {
    const [input, runtime] = await Promise.all([api.inspectInput(candidate.input_path), api.inspectRuntime(candidate.rscript_path, candidate.library_path)]);
    return { input, runtime };
  };
  const inspect = async () => {
    if (active) return;
    const key = ++revision;
    patch({ status: "checking", operation: null, error: null, input: null, runtime: null });
    try {
      const data = await check(state.config);
      if (key === revision) patch({ ...data, status: "idle" });
    } catch (error) { if (key === revision) patch({ status: "error", error: cubePrepError(error) }); }
  };
  const restore = async (manifestPath: string) => {
    if (active) return;
    const key = ++revision;
    patch({ status: "checking", operation: "restore", error: null, input: null, result: null, progress: null, outputDir: null, unit: null, installation: null });
    try {
      const saved = await api.load(manifestPath);
      if (key !== revision) return;
      const { rscript_path, library_path, output_dir } = state.config;
      const config = { ...saved.config, rscript_path, library_path, output_dir };
      patch({ config, result: saved.result, outputDir: saved.result.output_dir, unit: saved.result.unit, status: "restored", operation: null });
      try { storage?.setItem("ab.sagui.v1", JSON.stringify(config)); } catch { return; }
    } catch (error) { if (key === revision) patch({ status: "error", operation: null, error: cubePrepError(error) }); }
  };
  const inspectRuntime = async () => {
    if (active) return;
    const key = ++revision;
    patch({ status: "checking", operation: null, error: null, runtime: null });
    try {
      const runtime = await api.inspectRuntime(state.config.rscript_path, state.config.library_path);
      if (key === revision) patch({ runtime, status: "idle" });
    } catch (error) { if (key === revision) patch({ status: "error", error: cubePrepError(error) }); }
  };
  const prepareCubePrep = async (report: CubePrepReport, isCurrent: () => boolean = () => true): Promise<boolean> => {
    if (active) throw new Error("Wait for the active SAGUI job before choosing another cube.");
    if (!report.cube) throw new Error("The Cube Prep report has no final cube.");
    const key = ++revision;
    const path = joinProductPath(report.output_dir, report.cube);
    patch({ status: "checking", operation: null, error: null });
    try {
      const input = await api.inspectInput(path);
      if (key !== revision) return false;
      if (!isCurrent()) { patch({ status: "idle" }); return false; }
      const config = { ...state.config, input_path: path, output_dir: state.config.output_dir || report.output_dir, psf_target: cubePrepPsfTarget(report, input.bands.map((band) => band.filter)) };
      patch({ config, input, status: "idle", installation: null, result: null, progress: null, outputDir: null, unit: null });
      try { storage?.setItem("ab.sagui.v1", JSON.stringify(config)); } catch { return true; }
      return true;
    } catch (error) {
      if (key !== revision) return false;
      if (!isCurrent()) { patch({ status: "idle" }); return false; }
      patch({ status: "error", error: cubePrepError(error) });
      throw error;
    }
  };
  const run = async () => {
    if (active) return;
    const problems = saguiConfigProblems(state.config);
    if (problems.length) { patch({ status: "error", error: problems.join(" ") }); return; }
    ++revision;
    const id = `${new Date().toISOString().replace(/[^0-9]/g, "")}-${Math.random().toString(36).slice(2, 9)}`;
    active = id;
    cancelled = false;
    const candidate = { ...state.config };
    patch({ status: "checking", operation: "segment", error: null, result: null, input: null, runtime: null, unit: null, progress: null, outputDir: null });
    let stop: (() => void) | undefined;
    try {
      const data = await check(candidate);
      if (cancelled) { patch({ status: "cancelled" }); return; }
      patch(data);
      if (!data.runtime.ready) throw new Error(data.runtime.issues.join(" ") || "The official SAGUI runtime is not ready.");
      stop = await api.listen(id, (progress) => { if (active === id && !cancelled) patch({ progress }); });
      if (cancelled) { patch({ status: "cancelled" }); return; }
      patch({ status: "running", unit: data.input.unit, outputDir: joinProductPath(candidate.output_dir, `sagui-${id}`) });
      const result = await api.run(candidate, id);
      patch({ result, status: "complete", progress: null, outputDir: result.output_dir });
    } catch (error) {
      patch({ status: cancelled ? "cancelled" : "error", error: cancelled ? null : cubePrepError(error), progress: null });
    } finally { stop?.(); active = null; cancelled = false; }
  };
  const install = async () => {
    if (active) return;
    if (!state.config.rscript_path.trim() || !state.installParent.trim()) { patch({ status: "error", error: "Choose the Rscript executable and a library installation folder." }); return; }
    ++revision;
    const id = `${new Date().toISOString().replace(/[^0-9]/g, "")}-${Math.random().toString(36).slice(2, 9)}`;
    active = id;
    cancelled = false;
    const { rscript_path: rscript } = state.config;
    const parent = state.installParent;
    patch({ status: "checking", operation: "install", error: null, installation: null, result: null, input: null, runtime: null, unit: null, progress: null, outputDir: joinProductPath(parent, `install-${id}`) });
    let stop: (() => void) | undefined;
    try {
      stop = await api.listenInstall(id, (progress) => { if (active === id && !cancelled) patch({ progress }); });
      if (cancelled) { patch({ status: "cancelled" }); return; }
      patch({ status: "installing" });
      const installation = await api.install(rscript, parent, id);
      if (cancelled) { patch({ status: "cancelled", progress: null }); return; }
      patch({ status: "checking", progress: null });
      const runtime = await api.inspectRuntime(installation.rscript_path, installation.library_path);
      if (cancelled) { patch({ status: "cancelled" }); return; }
      if (!runtime.ready) throw new Error(runtime.issues.join(" ") || "The installed SAGUI runtime could not be verified.");
      const config = { ...state.config, rscript_path: installation.rscript_path, library_path: installation.library_path };
      patch({ config, installation, runtime, status: "installed", error: null });
      try { storage?.setItem("ab.sagui.v1", JSON.stringify(config)); } catch { return; }
    } catch (error) {
      patch({ status: cancelled ? "cancelled" : "error", error: cancelled ? null : cubePrepError(error), progress: null });
    } finally { stop?.(); active = null; cancelled = false; }
  };
  const cancel = async () => {
    if (!active || cancelled) return;
    const id = active;
    cancelled = true;
    const running = state.status === "running" || state.status === "installing";
    const installation = state.operation === "install";
    patch({ status: "cancelling" });
    if (!running) return;
    try {
      const accepted = await (installation ? api.cancelInstall(id) : api.cancel(id));
      if (!accepted && active === id) { cancelled = false; patch({ status: installation ? "installing" : "running", error: "Cancellation was not accepted. Please retry." }); }
    } catch (error) { if (active === id) { cancelled = false; patch({ status: installation ? "installing" : "running", error: cubePrepError(error) }); } }
  };
  return {
    get: () => state,
    isJobActive: () => active !== null,
    subscribe: (callback: () => void) => { subscribers.add(callback); return () => { subscribers.delete(callback); }; },
    edit: (next: SaguiConfig) => {
      if (active) return;
      ++revision;
      const previous = state.config;
      const changed = (key: keyof SaguiConfig) => JSON.stringify(previous[key]) !== JSON.stringify(next[key]);
      const keys = [...new Set([...Object.keys(previous), ...Object.keys(next)])] as (keyof SaguiConfig)[];
      const keepResult = state.result !== null && !keys.some((key) => !RESULT_INDEPENDENT_KEYS.has(key) && changed(key));
      patch({
        config: next, status: "idle", operation: null, progress: null, error: null,
        ...(changed("input_path") ? { input: null } : {}),
        ...(changed("rscript_path") || changed("library_path") ? { runtime: null } : {}),
        ...(keepResult ? {} : { result: null, outputDir: null, unit: null }),
      });
      try { storage?.setItem("ab.sagui.v1", JSON.stringify(next)); } catch { return; }
    },
    setInstallParent: (path: string) => {
      if (active) return;
      patch({ installParent: path });
      try { storage?.setItem("ab.sagui.installParent.v1", path); } catch { return; }
    },
    inspect, inspectRuntime, prepareCubePrep, restore, run, install, cancel,
  };
}

export const saguiStore = createSaguiStore();
export function useSaguiState(): SaguiState { return useSyncExternalStore(saguiStore.subscribe, saguiStore.get, saguiStore.get); }
