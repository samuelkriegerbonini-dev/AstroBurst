import { CUBE_PREP_STEPS, type CubePrepInput, type CubePrepProgress, type CubePrepRecipe, type CubePrepReport, type CubePrepStep, type CubePrepStepName } from "../../shared/types/cubeprep";
import { formatTime } from "../../utils/format";

export type CubePrepProfile = "lightstack" | "scientific";

export const CUBE_PREP_LABELS: Record<CubePrepStepName, string> = { crop: "Crop", units: "Units", align: "Align", cube: "Cube", edit: "Edit", psfs: "PSFs", kernels: "Kernels", match: "Match", export: "Export" };
const NULLABLE_KEYS = new Set(["reference", "redshift", "ra_center", "dec_center", "pixel_scale_arcsec", "noise_threshold", "oversampling", "oversampling_order", "data_dir"]);
const FILE_STAGES = new Map([["hashing_inputs", "Hashing inputs"], ["verifying_inputs", "Verifying inputs"], ["hashing_products", "Hashing products"]]);

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
    case "align": return { step: name, reference, method: physical ? "exact" : "bilinear", border_crop: 1, area_rule: physical ? "scale" : "lightstack" };
    case "cube": return { step: name, reference, order: "wavelength", output: "cube.fits", layout: physical ? "mef" : "lightstack" };
    case "edit": return { step: name, action: "drop_invalid", threshold: 0, frac_valid: 0.01, use_dq: physical };
    case "psfs": return { step: name, source: "files", reference, files: recipe.inputs.map((input) => ({ filter: input.filter, path: "" })) };
    case "kernels": return { step: name, reference, method: "wiener", eps: 0.001, eps_mode: "absolute", shape: "reference", auto: physical };
    case "match": return { step: name, reference, mode: physical ? "astroburst" : "lightstack", allow_unmatched: false };
    case "export": return { step: name, output: "prepared_cube.fits" };
  }
}

export function applyProfile(recipe: CubePrepRecipe, profile: CubePrepProfile): CubePrepRecipe {
  const physical = profile === "scientific";
  const surfaceBrightness = recipe.steps.find((s) => s.step === "units")?.to === "MJy/sr";
  return { ...recipe, steps: recipe.steps.map((step) => {
    if (step.step === "crop") return { ...step, rounding: physical ? "astroburst" : "lightstack" };
    if (step.step === "align") return { ...step, method: physical ? "exact" : "bilinear", area_rule: physical ? surfaceBrightness ? "keep" : "scale" : "lightstack" };
    if (step.step === "cube") return { ...step, layout: physical ? "mef" : "lightstack" };
    if (step.step === "match") return { ...step, mode: physical ? "astroburst" : "lightstack" };
    if (step.step === "edit" && step.action === "drop_invalid") return { ...step, use_dq: physical };
    if (step.step === "edit" && step.action === "cut_sky") return { ...step, rounding: physical ? "astroburst" : "lightstack" };
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
  const isInput = (name: string) => recipe.inputs.some((input) => sameFilter(input.filter, name));
  const reference = (label: string, value: unknown): string | null => {
    const name = typeof value === "string" ? value.trim() : "";
    if (name && isInput(name)) return name;
    problems.push(name ? `${label}: reference ${name} is not an input filter.` : `Choose a reference filter for ${label}.`);
    return null;
  };
  let aligned = false;
  let cube = false;
  let kernels: string | null = null;
  const psfs: string[] = [];
  for (const step of recipe.steps) {
    const label = CUBE_PREP_LABELS[step.step];
    const needsCube = () => { if (!cube) problems.push(`${label} needs an earlier Cube step.`); };
    for (const [key, value] of Object.entries(step)) {
      if (value === null && !NULLABLE_KEYS.has(key)) problems.push(`${label}: enter a value for ${key}.`);
    }
    if (step.step === "crop" && step.mode === "reference") reference(label, step.reference);
    if (step.step === "align") { reference(label, step.reference); aligned = true; }
    if (step.step === "cube") {
      if (step.reference !== undefined && step.reference !== null) reference(label, step.reference);
      else if (!aligned) problems.push("Cube needs a reference filter or an earlier Align step.");
      cube = true;
    }
    if (step.step === "edit") {
      needsCube();
      if (step.action === "remove_filter") {
        const filter = typeof step.filter === "string" ? step.filter.trim() : "";
        if (!filter) problems.push("Choose the filter to remove in Edit.");
        else if (!isInput(filter)) problems.push(`Edit: ${filter} is not an input filter.`);
      }
    }
    if (step.step === "psfs") {
      const names = psfFilters(step);
      for (const name of names) if (!isInput(name)) problems.push(`PSFs: ${name} is not an input filter.`);
      const target = typeof step.reference === "string" ? step.reference : "";
      if (typeof step.pixel_scale_arcsec === "number" && !names.some((name) => sameFilter(name, target))) problems.push("PSFs: resampling to a pixel scale needs a reference filter among the PSFs.");
      psfs.push(...names);
    }
    if (step.step === "kernels") {
      const name = reference(label, step.reference);
      if (name && !psfs.some((psf) => sameFilter(psf, name))) problems.push(`Kernels need a PSF for ${name} in an earlier PSFs step.`);
      kernels = typeof step.reference === "string" ? step.reference : null;
    }
    if (step.step === "match") {
      needsCube();
      const name = reference(label, step.reference);
      if (name && !(kernels !== null && sameFilter(kernels, name))) problems.push(`Match needs an earlier Kernels step to ${name}.`);
    }
    if (step.step === "export") needsCube();
  }
  return [...new Set(problems)];
}

function sameFilter(a: string, b: string): boolean {
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

function psfFilters(step: CubePrepStep): string[] {
  const files: unknown[] = Array.isArray(step.files) ? step.files : [];
  const listed: unknown[] = step.source === "files" ? files.map((file) => file && typeof file === "object" ? (file as { filter?: unknown }).filter : undefined) : Array.isArray(step.filters) ? step.filters : [];
  return listed.filter((name): name is string => typeof name === "string" && name.trim() !== "").map((name) => name.trim());
}

export function changeRecipeInput(recipe: CubePrepRecipe, index: number, change: Partial<CubePrepInput>): CubePrepRecipe {
  const inputs = recipe.inputs.map((input, i) => i === index ? { ...input, ...change } : input);
  const from = recipe.inputs[index]?.filter ?? "";
  const to = change.filter ?? from;
  const others = recipe.inputs.filter((_, i) => i !== index);
  if (to === from || !from.trim() || !to.trim() || others.some((input) => sameFilter(input.filter, from) || sameFilter(input.filter, to))) return { ...recipe, inputs };
  const renamed = (value: unknown) => typeof value === "string" && sameFilter(value, from);
  const steps = recipe.steps.map((step): CubePrepStep => {
    const next: CubePrepStep = { ...step };
    if (renamed(step.reference)) next.reference = to;
    if (step.step === "edit" && step.action === "remove_filter" && renamed(step.filter)) next.filter = to;
    if (step.step === "psfs" && Array.isArray(step.files)) next.files = (step.files as unknown[]).map((file) => file && typeof file === "object" && renamed((file as { filter?: unknown }).filter) ? { ...file, filter: to } : file);
    if (step.step === "psfs" && Array.isArray(step.filters)) next.filters = (step.filters as unknown[]).map((filter) => renamed(filter) ? to : filter);
    return next;
  });
  return { ...recipe, inputs, steps };
}

function humanise(id: string): string {
  const words = id.replace(/[_-]+/g, " ").trim();
  return words ? `${words[0].toUpperCase()}${words.slice(1)}` : "";
}

export function cubePrepStageText(progress: CubePrepProgress): string {
  const known = FILE_STAGES.get(progress.stage);
  if (known) return known;
  const folder = /^\d+_(.+)$/.exec(progress.stage);
  if (folder) {
    const kind = folder[1];
    const label = (CUBE_PREP_STEPS as readonly string[]).includes(kind) ? CUBE_PREP_LABELS[kind as CubePrepStepName] : humanise(kind);
    return `Step ${progress.current + 1} of ${progress.total}: ${label}`;
  }
  return humanise(progress.stage) || "Starting";
}

export function cubePrepProgressDetail(progress: CubePrepProgress, elapsedMs: number | null): string {
  const parts = [`${Math.round(progress.percent)}%`];
  if (FILE_STAGES.has(progress.stage)) parts.push(`${progress.current} of ${progress.total} ${progress.total === 1 ? "file" : "files"}`);
  if (elapsedMs !== null) parts.push(`${elapsedMs < 1000 ? "0s" : formatTime(Math.floor(elapsedMs / 1000) * 1000)} elapsed`);
  return parts.join(" · ");
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

function comparable(text: string): string {
  return text.toLowerCase().replace(/\s+/g, " ").trim().replace(/\.$/, "");
}

export function cubePrepError(error: unknown): string {
  const message = error instanceof Error ? error.message : error && typeof error === "object" && "message" in error ? String(error.message) : String(error);
  const hint = error && typeof error === "object" && "hint" in error && typeof error.hint === "string" ? error.hint.trim() : "";
  return hint && !comparable(message).includes(comparable(hint)) ? `${message} (${hint})` : message;
}
