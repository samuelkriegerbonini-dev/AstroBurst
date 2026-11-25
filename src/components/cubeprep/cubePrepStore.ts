import { useSyncExternalStore } from "react";
import { cancelCubePrep, onCubePrepProgress, runCubePrepRecipe, validateCubePrepRecipe } from "../../services/cubeprep";
import type { CubePrepProgress, CubePrepRecipe, CubePrepReport, CubePrepValidation } from "../../shared/types/cubeprep";
import { createCubePrepDraft, cubePrepError, cubePrepProblems, parseRecipe, workingDirectory, type CubePrepDraft } from "./cubePrepModel";

export interface CubePrepApi {
  validate(recipe: CubePrepRecipe, baseDir: string): Promise<CubePrepValidation>;
  run(recipe: CubePrepRecipe, baseDir: string, outputDir: string, runId: string): Promise<CubePrepReport>;
  listen(runId: string, callback: (progress: CubePrepProgress) => void): Promise<() => void>;
  cancel(runId: string): Promise<boolean>;
}

export interface CubePrepState {
  draft: CubePrepDraft;
  status: "idle" | "validating" | "running" | "cancelling" | "complete" | "cancelled" | "error";
  validation: CubePrepValidation | null;
  progress: CubePrepProgress | null;
  report: CubePrepReport | null;
  outputDir: string | null;
  error: string | null;
  startedAt: number | null;
}

type CubePrepStorage = Pick<Storage, "getItem" | "setItem">;
const STORAGE_KEY = "ab.cubeprep.v1";
const DRAFT_KEY = "ab.cubeprep.draft.v1";
const API: CubePrepApi = { validate: validateCubePrepRecipe, run: runCubePrepRecipe, listen: onCubePrepProgress, cancel: cancelCubePrep };

function storageOrNull(): CubePrepStorage | null {
  try { return typeof window === "undefined" ? null : window.localStorage; } catch { return null; }
}

function savedDraft(value: unknown): CubePrepDraft | null {
  const draft = value as Partial<CubePrepDraft> | null | undefined;
  if (!draft || typeof draft.baseDir !== "string" || typeof draft.outputParent !== "string") return null;
  return { baseDir: draft.baseDir, outputParent: draft.outputParent, recipe: parseRecipe(JSON.stringify(draft.recipe)), profile: draft.profile === "scientific" ? "scientific" : "lightstack" };
}

export function createCubePrepStore(api: CubePrepApi = API, storage: CubePrepStorage | null = storageOrNull()) {
  let state: CubePrepState = { draft: createCubePrepDraft(), status: "idle", validation: null, progress: null, report: null, outputDir: null, error: null, startedAt: null };
  try {
    const saved = JSON.parse(storage?.getItem(STORAGE_KEY) ?? "null") as Partial<CubePrepState> | null;
    const draft = savedDraft(JSON.parse(storage?.getItem(DRAFT_KEY) ?? "null")) ?? savedDraft(saved?.draft);
    if (draft) state = { ...state, draft, report: saved?.report?.recipe_version === 1 ? saved.report : null, outputDir: typeof saved?.outputDir === "string" ? saved.outputDir : null };
  } catch { state = { ...state }; }
  let revision = 0;
  let runId: string | null = null;
  let cancelRequested = false;
  const listeners = new Set<() => void>();
  const write = (key: string, value: unknown) => { try { storage?.setItem(key, JSON.stringify(value)); } catch { return; } };
  const update = (change: Partial<CubePrepState>, persist = false) => {
    const before = state;
    state = { ...state, ...change };
    if (persist) {
      if (state.draft !== before.draft) write(DRAFT_KEY, state.draft);
      if (state.report !== before.report || state.outputDir !== before.outputDir) write(STORAGE_KEY, { report: state.report, outputDir: state.outputDir });
    }
    listeners.forEach((listener) => listener());
  };
  const localProblems = (forRun: boolean) => cubePrepProblems(state.draft, forRun).map((problem) => problem.text);
  const validate = async () => {
    if (runId) return;
    const current = ++revision;
    const problems = localProblems(false);
    if (problems.length) { update({ status: "error", validation: null, error: problems.join(" ") }); return; }
    const draft = state.draft;
    update({ status: "validating", validation: null, error: null });
    try {
      const validation = await api.validate(draft.recipe, draft.baseDir);
      if (current === revision) update({ validation, status: "idle" });
    } catch (error) {
      if (current === revision) update({ error: cubePrepError(error), status: "error" });
    }
  };
  const run = async () => {
    if (runId) return;
    const problems = localProblems(true);
    if (problems.length) { update({ status: "error", error: problems.join(" ") }); return; }
    ++revision;
    const id = `${new Date().toISOString().replace(/[^0-9]/g, "")}-${Math.random().toString(36).slice(2, 9)}`;
    runId = id;
    cancelRequested = false;
    const draft = state.draft;
    const outputDir = workingDirectory(draft.outputParent, draft.recipe.name ?? "field", id);
    update({ status: "validating", error: null, report: null, progress: null, validation: null, outputDir: null, startedAt: Date.now() }, true);
    let unlisten: (() => void) | undefined;
    try {
      const validation = await api.validate(draft.recipe, draft.baseDir);
      if (cancelRequested) { update({ status: "cancelled" }); return; }
      update({ validation });
      unlisten = await api.listen(id, (progress) => {
        if (runId === id && !cancelRequested) update({ progress });
      });
      if (cancelRequested) { update({ status: "cancelled" }); return; }
      update({ status: "running", outputDir }, true);
      const report = await api.run(validation.recipe, draft.baseDir, outputDir, id);
      update({ report, status: "complete", outputDir: report.output_dir, progress: null }, true);
    } catch (error) {
      const code = error && typeof error === "object" && "code" in error ? String(error.code) : "";
      const cancelled = /cancel/i.test(code) || /cancelled|canceled/i.test(cubePrepError(error));
      update({ status: cancelled ? "cancelled" : "error", error: cancelled ? null : cubePrepError(error), progress: null }, true);
    } finally {
      unlisten?.();
      runId = null;
      cancelRequested = false;
    }
  };
  const cancel = async () => {
    if (!runId || cancelRequested) return;
    const id = runId;
    cancelRequested = true;
    const wasRunning = state.status === "running";
    update({ status: "cancelling" });
    if (!wasRunning) return;
    try {
      const accepted = await api.cancel(id);
      if (!accepted && runId === id) {
        cancelRequested = false;
        update({ status: "running", error: "Cancellation was not accepted. Please retry while the job is running." });
      }
    } catch (error) {
      if (runId === id) {
        cancelRequested = false;
        update({ status: "running", error: `Cancellation failed: ${cubePrepError(error)}` });
      }
    }
  };
  return {
    get: () => state,
    isJobActive: () => runId !== null,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    edit: (draft: CubePrepDraft) => {
      if (runId) return;
      ++revision;
      const products = (next: CubePrepDraft) => JSON.stringify([next.recipe.inputs, next.recipe.steps, next.baseDir]);
      const stale = products(draft) !== products(state.draft);
      update({ draft, status: "idle", validation: null, progress: null, error: null, ...(stale ? { report: null, outputDir: null } : {}) }, true);
    },
    validate, run, cancel,
  };
}

export const cubePrepStore = createCubePrepStore();

export function useCubePrepState(): CubePrepState {
  return useSyncExternalStore(cubePrepStore.subscribe, cubePrepStore.get, cubePrepStore.get);
}
