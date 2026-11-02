import { CUBE_PREP_STEPS, type CubePrepRecipe, type CubePrepReport, type CubePrepStep, type CubePrepStepName } from "../../shared/types/cubeprep";

export type CubePrepProfile = "lightstack" | "scientific";

export interface CubePrepDraft {
  recipe: CubePrepRecipe;
  profile: CubePrepProfile;
  baseDir: string;
  outputParent: string;
}

export function createCubePrepDraft(): CubePrepDraft {
  return {
    profile: "lightstack", baseDir: "", outputParent: "",
    recipe: {
      recipe_version: 1, name: "field", inputs: [],
      steps: [
        { step: "units", to: "Jy/pixel" },
        { step: "align", reference: "", method: "bilinear", border_crop: 1, area_rule: "lightstack" },
        { step: "cube", reference: "", order: "wavelength", output: "cube.fits", layout: "lightstack" },
      ],
    },
  };
}

export function setRecipeStep(recipe: CubePrepRecipe, step: CubePrepStep): CubePrepRecipe {
  const index = recipe.steps.findIndex((entry) => entry.step === step.step);
  if (index >= 0) return { ...recipe, steps: recipe.steps.map((entry, i) => i === index ? step : entry) };
  const rank = CUBE_PREP_STEPS.indexOf(step.step);
  const next = recipe.steps.findIndex((entry) => CUBE_PREP_STEPS.indexOf(entry.step) > rank);
  const steps = [...recipe.steps];
  steps.splice(next < 0 ? steps.length : next, 0, step);
  return { ...recipe, steps };
}

export function defaultStep(name: CubePrepStepName, recipe: CubePrepRecipe, profile: CubePrepProfile): CubePrepStep {
  const reference = recipe.inputs.at(-1)?.filter ?? "";
  const physical = profile === "scientific";
  switch (name) {
    case "crop": return { step: name, mode: "common", rounding: physical ? "astroburst" : "lightstack" };
    case "units": return { step: name, to: "Jy/pixel" };
    case "align": return { step: name, reference, method: "bilinear", border_crop: 1, area_rule: physical ? "scale" : "lightstack" };
    case "cube": return { step: name, reference, order: "wavelength", output: "cube.fits", layout: physical ? "mef" : "lightstack" };
    case "edit": return { step: name, action: "drop_invalid", threshold: 0, frac_valid: 0.01, use_dq: physical };
    case "psfs": return { step: name, source: "files", reference, files: recipe.inputs.map((input) => ({ filter: input.filter, path: "" })) };
    case "kernels": return { step: name, reference, method: "wiener", eps: 0.001, eps_mode: "absolute", shape: "reference", auto: false };
    case "match": return { step: name, reference, mode: physical ? "astroburst" : "lightstack", allow_unmatched: false };
    case "export": return { step: name, output: "prepared_cube.fits" };
  }
}

export function applyProfile(recipe: CubePrepRecipe, profile: CubePrepProfile): CubePrepRecipe {
  const physical = profile === "scientific";
  const surfaceBrightness = recipe.steps.find((s) => s.step === "units")?.to === "MJy/sr";
  return { ...recipe, steps: recipe.steps.map((step) => {
    if (step.step === "crop") return { ...step, rounding: physical ? "astroburst" : "lightstack" };
    if (step.step === "align") return { ...step, area_rule: physical ? surfaceBrightness ? "keep" : "scale" : "lightstack" };
    if (step.step === "cube") return { ...step, layout: physical ? "mef" : "lightstack" };
    if (step.step === "match") return { ...step, mode: physical ? "astroburst" : "lightstack" };
    if (step.step === "edit" && step.action === "drop_invalid") return { ...step, use_dq: physical };
    return step;
  }) };
}

export function parseRecipe(text: string): CubePrepRecipe {
  const value: unknown = JSON.parse(text);
  if (!value || typeof value !== "object") throw new Error("Choose a Cube Prep recipe JSON file.");
  const recipe = value as CubePrepRecipe;
  if (recipe.recipe_version !== 1 || !Array.isArray(recipe.inputs) || !Array.isArray(recipe.steps)) throw new Error("Expected recipe_version 1, inputs and steps.");
  if (!recipe.inputs.every((input) => input && typeof input.path === "string" && typeof input.dataset === "string" && typeof input.filter === "string")) throw new Error("Each input needs a path, dataset and filter.");
  if (!recipe.steps.every((step) => step && CUBE_PREP_STEPS.includes(step.step))) throw new Error("The recipe contains an unknown step.");
  return recipe;
}

export function recipeProblems(recipe: CubePrepRecipe): string[] {
  const problems: string[] = [];
  if (recipe.inputs.length === 0) problems.push("Add at least one input image.");
  const filters = new Set<string>();
  for (const [i, input] of recipe.inputs.entries()) {
    if (![input.path, input.dataset, input.filter].every((value) => value.trim())) problems.push(`Input ${i + 1} needs a file, dataset and filter.`);
    const filter = input.filter.trim().toLowerCase();
    if (filter && filters.has(filter)) problems.push(`Duplicate filter: ${input.filter}.`);
    filters.add(filter);
  }
  if (recipe.steps.length === 0) problems.push("Enable at least one preparation step.");
  for (const step of recipe.steps) {
    if (["align", "kernels", "match"].includes(step.step) && !String(step.reference ?? "").trim()) problems.push(`Choose a reference filter for ${step.step}.`);
  }
  return problems;
}

export function inputDirectory(path: string): string {
  const index = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return index === 0 ? "/" : index < 0 ? "" : path.slice(0, index);
}

export function joinProductPath(parent: string, child: string): string {
  if (/^(?:[A-Za-z]:[\\/]|[\\/])/.test(child)) return child;
  const separator = parent.includes("\\") ? "\\" : "/";
  return `${parent.replace(/[\\/]+$/, "")}${separator}${child.replace(/^[\\/]+/, "")}`;
}

export function cubePrepPsfTarget(report: CubePrepReport, filters: string[]): string {
  let index = report.steps.length - 1;
  while (index >= 0 && report.steps[index].step === "export") index--;
  if (index < 0 || report.steps[index].step !== "match") return "";
  const detail = report.steps[index].report as { reference?: unknown; planes?: { filter?: unknown; status?: unknown }[] } | null;
  if (!detail || typeof detail.reference !== "string" || !detail.reference.trim() || !Array.isArray(detail.planes) || detail.planes.length !== filters.length || filters.length < 2) return "";
  const names = new Set(filters.map((filter) => filter.toLowerCase()));
  let references = 0;
  for (const plane of detail.planes) {
    if (typeof plane.filter !== "string" || !names.delete(plane.filter.toLowerCase()) || !["matched", "reference"].includes(String(plane.status))) return "";
    if (plane.status === "reference") {
      if (plane.filter.toLowerCase() !== detail.reference.toLowerCase()) return "";
      references++;
    }
  }
  return names.size === 0 && references === 1 ? detail.reference : "";
}

export function workingDirectory(parent: string, name: string, id: string): string {
  const label = name.trim().replace(/[^a-zA-Z0-9_-]+/g, "-").replace(/^-+|-+$/g, "") || "field";
  return joinProductPath(parent, `cubeprep-${label}-${id}`);
}

export function recipeForSave(recipe: CubePrepRecipe, baseDir: string): CubePrepRecipe {
  const resolve = (path: string) => path && baseDir ? joinProductPath(baseDir, path) : path;
  return {
    ...recipe,
    inputs: recipe.inputs.map((input) => ({ ...input, path: resolve(input.path) })),
    steps: recipe.steps.map((step) => {
      if (step.step === "crop" && typeof step.region_file === "string") return { ...step, region_file: resolve(step.region_file) };
      if (step.step === "psfs" && step.source === "files" && Array.isArray(step.files)) return { ...step, files: (step.files as { filter: string; path: string }[]).map((file) => ({ ...file, path: resolve(file.path) })) };
      if (step.step === "psfs" && step.source === "empirical") return { ...step, data_dir: resolve(typeof step.data_dir === "string" ? step.data_dir : ".") };
      return step;
    }),
  };
}

export function cubePrepError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String(error.message);
  return String(error);
}
