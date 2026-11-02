import { useSyncExternalStore } from "react";
import { cancelCubePrep, onCubePrepProgress, runCubePrepRecipe, validateCubePrepRecipe } from "../../services/cubeprep";
import type { CubePrepProgress, CubePrepRecipe, CubePrepReport, CubePrepValidation } from "../../shared/types/cubeprep";
import { createCubePrepDraft, cubePrepError, parseRecipe, recipeProblems, workingDirectory, type CubePrepDraft } from "./cubePrepModel";

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
}

type CubePrepStorage = Pick<Storage, "getItem" | "setItem">;
const STORAGE_KEY = "ab.cubeprep.v1";
const API: CubePrepApi = { validate: validateCubePrepRecipe, run: runCubePrepRecipe, listen: onCubePrepProgress, cancel: cancelCubePrep };

function storageOrNull(): CubePrepStorage | null {
  try { return typeof window === "undefined" ? null : window.localStorage; } catch { return null; }
}

export function createCubePrepStore(api: CubePrepApi = API, storage: CubePrepStorage | null = storageOrNull()) {
  let state: CubePrepState = { draft: createCubePrepDraft(), status: "idle", validation: null, progress: null, report: null, outputDir: null, error: null };
  try {
    const saved = JSON.parse(storage?.getItem(STORAGE_KEY) ?? "null") as Partial<CubePrepState> | null;
    if (saved?.draft && typeof saved.draft.baseDir === "string" && typeof saved.draft.outputParent === "string") {
      const recipe = parseRecipe(JSON.stringify(saved.draft.recipe));
      state = { ...state, draft: { ...saved.draft, recipe, profile: saved.draft.profile === "scientific" ? "scientific" : "lightstack" }, report: saved.report?.recipe_version === 1 ? saved.report : null, outputDir: typeof saved.outputDir === "string" ? saved.outputDir : null };
    }
  } catch { state = { ...state }; }
  let revision = 0;
  let runId: string | null = null;
  let cancelRequested = false;
  const listeners = new Set<() => void>();
  const update = (change: Partial<CubePrepState>, persist = false) => {
    state = { ...state, ...change };
    if (persist) {
      try { storage?.setItem(STORAGE_KEY, JSON.stringify({ draft: state.draft, report: state.report, outputDir: state.outputDir })); } catch { state = { ...state }; }
    }
    listeners.forEach((listener) => listener());
  };
  const localProblems = () => {
    const problems = recipeProblems(state.draft.recipe);
    if (!state.draft.baseDir.trim()) problems.push("Choose the input folder for relative file paths.");
    return problems;
  };
  const validate = async () => {
    if (runId) return;
    const current = ++revision;
    const problems = localProblems();
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
    const problems = localProblems();
    if (!state.draft.outputParent.trim()) problems.push("Choose a working folder for this run.");
    if (problems.length) { update({ status: "error", error: problems.join(" ") }); return; }
    ++revision;
    const id = `${new Date().toISOString().replace(/[^0-9]/g, "")}-${Math.random().toString(36).slice(2, 9)}`;
    runId = id;
    cancelRequested = false;
    const draft = state.draft;
    const outputDir = workingDirectory(draft.outputParent, draft.recipe.name ?? "field", id);
    update({ status: "validating", error: null, report: null, progress: null, validation: null, outputDir }, true);
    let unlisten: (() => void) | undefined;
    try {
      const validation = await api.validate(draft.recipe, draft.baseDir);
      if (cancelRequested) { update({ status: "cancelled" }); return; }
      update({ validation });
      unlisten = await api.listen(id, (progress) => {
        if (runId === id && !cancelRequested) update({ progress });
      });
      if (cancelRequested) { update({ status: "cancelled" }); return; }
      update({ status: "running" });
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
      update({ draft, status: "idle", validation: null, report: null, outputDir: null, progress: null, error: null }, true);
    },
    validate, run, cancel,
  };
}

export const cubePrepStore = createCubePrepStore();

export function useCubePrepState(): CubePrepState {
  return useSyncExternalStore(cubePrepStore.subscribe, cubePrepStore.get, cubePrepStore.get);
}
