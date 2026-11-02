import { typedInvoke } from "../infrastructure/tauri";
import { listen } from "@tauri-apps/api/event";
import { cancelProgress } from "./progress";
import type { CubePrepFilters, CubePrepProgress, CubePrepRecipe, CubePrepReport, CubePrepValidation } from "../shared/types/cubeprep";

export function cubePrepFilters(): Promise<CubePrepFilters> {
  return typedInvoke("cubeprep_filters_cmd");
}

export function validateCubePrepRecipe(recipe: CubePrepRecipe, baseDir: string): Promise<CubePrepValidation> {
  return typedInvoke("validate_cubeprep_recipe_cmd", { recipe, baseDir });
}

export function runCubePrepRecipe(recipe: CubePrepRecipe, baseDir: string, outputDir: string, runId: string): Promise<CubePrepReport> {
  return typedInvoke("run_cubeprep_recipe_cmd", { recipe, baseDir, outputDir, runId });
}

export function onCubePrepProgress(runId: string, callback: (progress: CubePrepProgress) => void): Promise<() => void> {
  return listen<CubePrepProgress>(`cubeprep-progress-${runId}`, (event) => callback(event.payload));
}

export function cancelCubePrep(runId: string): Promise<boolean> {
  return cancelProgress(`cubeprep-progress-${runId}`);
}
