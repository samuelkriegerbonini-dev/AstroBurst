import { CUBE_PREP_STEPS, type CubePrepFilter, type CubePrepInput, type CubePrepProgress, type CubePrepRecipe, type CubePrepReport, type CubePrepStep, type CubePrepStepName, type CubePrepStepReport } from "../../shared/types/cubeprep";
import { formatTime } from "../../utils/format";
import { fwhmChange, psfFwhmRows, signedPercent } from "./cubePrepInspect";

export type CubePrepProfile = "lightstack" | "scientific";
export type CubePrepTab = "inputs" | CubePrepStepName;
export interface CubePrepProblem { tab: CubePrepTab | "folders"; text: string }
export const MATCHED_FWHM_TOLERANCE = 0.02;

export const CUBE_PREP_LABELS: Record<CubePrepStepName, string> = { crop: "Crop", units: "Units", align: "Align", cube: "Cube", edit: "Edit", psfs: "PSFs", kernels: "Kernels", match: "Match", export: "Export" };
const NULLABLE_KEYS = new Set(["reference", "redshift", "ra_center", "dec_center", "pixel_scale_arcsec", "noise_threshold", "oversampling", "oversampling_order", "data_dir"]);
const FILE_STAGES = new Map([["hashing_inputs", "Hashing inputs"], ["verifying_inputs", "Verifying inputs"], ["hashing_products", "Hashing products"]]);
const SAGUI_STEPS: readonly CubePrepStepName[] = ["units", "align", "cube", "psfs", "kernels", "match"];
const MATCHING_STEPS = ["psfs", "kernels", "match"] as const;
const PSF_NEUTRAL_EDITS = new Set(["cut", "cut_sky", "header", "reference_nan_mask"]);
const BACKEND_DEFAULTS: Partial<Record<CubePrepStepName, Record<string, unknown>>> = { crop: { rounding: "lightstack" }, align: { method: "bilinear", area_rule: "lightstack" }, cube: { layout: "lightstack" }, match: { mode: "lightstack" } };
const BACKEND_EDIT_DEFAULTS: Record<string, Record<string, unknown>> = { drop_invalid: { use_dq: false }, cut_sky: { rounding: "astroburst" } };

export interface CubePrepDraft {
  recipe: CubePrepRecipe;
  profile: CubePrepProfile;
  baseDir: string;
  outputParent: string;
}

export function createCubePrepDraft(): CubePrepDraft {
  const empty: CubePrepRecipe = { recipe_version: 1, name: "field", inputs: [], steps: [] };
  return { profile: "scientific", baseDir: "", outputParent: "", recipe: { ...empty, steps: SAGUI_STEPS.map((name) => defaultStep(name, empty, "scientific")) } };
}

export function replaceRecipeStepAt(recipe: CubePrepRecipe, index: number, step: CubePrepStep, profile?: CubePrepProfile): CubePrepRecipe {
  const steps = recipe.steps.map((entry, i) => i === index ? step : entry);
  const previous = recipe.steps[index];
  if (profile !== "scientific" || step.step !== "units" || previous?.step !== "units" || previous.to === step.to) return { ...recipe, steps };
  const areaRule = (unit: unknown) => unit === "MJy/sr" ? "keep" : "scale";
  return { ...recipe, steps: steps.map((entry) => entry.step === "align" && entry.area_rule === areaRule(previous.to) ? { ...entry, area_rule: areaRule(step.to) } : entry) };
}

export function removeRecipeStepAt(recipe: CubePrepRecipe, index: number): CubePrepRecipe {
  return { ...recipe, steps: recipe.steps.filter((_, i) => i !== index) };
}

export function addRecipeStep(recipe: CubePrepRecipe, step: CubePrepStep): CubePrepRecipe {
  const last = recipe.steps.map((entry) => entry.step).lastIndexOf(step.step);
  if (last < 0) return setRecipeStep(recipe, step);
  const steps = [...recipe.steps];
  steps.splice(last + 1, 0, step);
  return { ...recipe, steps };
}

function holdsReference(step: CubePrepStep): boolean {
  if (step.step === "crop") return step.mode === "reference";
  if (step.step === "cube") return step.reference !== undefined && step.reference !== null;
  return step.step === "align" || step.step === "psfs" || step.step === "kernels" || step.step === "match";
}

export function commonReference(recipe: CubePrepRecipe): string | null {
  const names = recipe.steps.filter(holdsReference).map((step) => typeof step.reference === "string" ? step.reference.trim() : "").filter(Boolean);
  if (names.length === 0) return "";
  return names.every((name) => sameFilter(name, names[0])) ? names[0] : null;
}

export function setCommonReference(recipe: CubePrepRecipe, filter: string): CubePrepRecipe {
  return { ...recipe, steps: recipe.steps.map((step) => holdsReference(step) ? { ...step, reference: filter } : step) };
}

function registryPivot(input: CubePrepInput, filters: readonly CubePrepFilter[]): number | null {
  const entry = filters.find((filter) => sameFilter(filter.dataset, input.dataset) && [filter.filter, ...filter.aliases].some((name) => sameFilter(name, input.filter)));
  return entry && entry.pivot_angstrom > 0 ? entry.pivot_angstrom : null;
}

export function broadestBand(inputs: readonly CubePrepInput[], filters: readonly CubePrepFilter[], measuredFwhm: Readonly<Record<string, number | null | undefined>> = {}): string {
  const named = inputs.filter((input) => input.filter.trim());
  const widest = (score: (input: CubePrepInput) => number | null | undefined): string => {
    const scored = named.map((input) => ({ filter: input.filter.trim(), value: score(input) }));
    if (scored.length === 0 || scored.some(({ value }) => typeof value !== "number" || !Number.isFinite(value) || value <= 0)) return "";
    return scored.reduce((best, entry) => (entry.value as number) > (best.value as number) ? entry : best).filter;
  };
  return widest((input) => measuredFwhm[input.path]) || widest((input) => registryPivot(input, filters));
}

export function withDefaultReference(recipe: CubePrepRecipe, filters: readonly CubePrepFilter[], measuredFwhm: Readonly<Record<string, number | null | undefined>> = {}): CubePrepRecipe {
  if (commonReference(recipe) !== "") return recipe;
  const band = broadestBand(recipe.inputs, filters, measuredFwhm);
  return band ? setCommonReference(recipe, band) : recipe;
}

export function syncPsfFiles(recipe: CubePrepRecipe): CubePrepRecipe {
  const filters = recipe.inputs.map((input) => input.filter.trim()).filter(Boolean);
  return { ...recipe, steps: recipe.steps.map((step) => {
    if (step.step !== "psfs" || step.source !== "files" || !Array.isArray(step.files)) return step;
    const listed = step.files as { filter: string; path: string }[];
    const kept = listed.filter((file) => (typeof file.path === "string" && file.path.trim() !== "") || filters.some((filter) => sameFilter(filter, String(file.filter ?? ""))));
    const missing = filters.filter((filter) => !kept.some((file) => sameFilter(String(file.filter ?? ""), filter)));
    return missing.length === 0 && kept.length === listed.length ? step : { ...step, files: [...kept, ...missing.map((filter) => ({ filter, path: "" }))] };
  }) };
}

export function afterInputChange(recipe: CubePrepRecipe, filters: readonly CubePrepFilter[], measuredFwhm: Readonly<Record<string, number | null | undefined>> = {}): CubePrepRecipe {
  return withDefaultReference(syncPsfFiles(recipe), filters, measuredFwhm);
}

export function applyGoal(recipe: CubePrepRecipe, profile: CubePrepProfile, filters: readonly CubePrepFilter[] = []): CubePrepRecipe {
  const profiled = applyProfile(recipe, profile);
  if (profile !== "scientific") return profiled;
  return MATCHING_STEPS.reduce((next, name) => next.steps.some((step) => step.step === name) ? next : setRecipeStep(next, defaultStep(name, next, profile, filters)), profiled);
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

export function defaultStep(name: CubePrepStepName, recipe: CubePrepRecipe, profile: CubePrepProfile, filters: readonly CubePrepFilter[] = []): CubePrepStep {
  const reference = commonReference(recipe) || broadestBand(recipe.inputs, filters) || (recipe.inputs.at(-1)?.filter ?? "");
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

export const CUBE_PREP_PROFILE_NAMES: Record<CubePrepProfile, string> = { lightstack: "LIGHTSTACK 0.4.0 compatibility", scientific: "Scientific MEF" };

function valueText(value: unknown): string {
  if (value === undefined) return "unset";
  return typeof value === "string" ? value : JSON.stringify(value);
}

function backendDefault(step: CubePrepStep, key: string): unknown {
  return (step.step === "edit" ? BACKEND_EDIT_DEFAULTS[String(step.action)] : BACKEND_DEFAULTS[step.step])?.[key];
}

function effectiveValue(step: CubePrepStep, key: string): unknown {
  return step[key] === undefined ? backendDefault(step, key) : step[key];
}

function recipeValueText(step: CubePrepStep, key: string): string {
  const fallback = backendDefault(step, key);
  return step[key] === undefined && fallback !== undefined ? `unset (${valueText(fallback)})` : valueText(step[key]);
}

export function profileDifferences(recipe: CubePrepRecipe, profile: CubePrepProfile): string[] {
  const defaults = applyProfile(recipe, profile).steps;
  return recipe.steps.flatMap((step, index) => {
    const expected = defaults[index];
    return [...new Set([...Object.keys(step), ...Object.keys(expected)])]
      .filter((key) => JSON.stringify(effectiveValue(step, key)) !== JSON.stringify(effectiveValue(expected, key)))
      .map((key) => `${CUBE_PREP_LABELS[step.step]} ${key}: ${recipeValueText(step, key)} → ${valueText(expected[key])}`);
  });
}

export function closestProfile(recipe: CubePrepRecipe, current: CubePrepProfile): CubePrepProfile {
  const other: CubePrepProfile = current === "scientific" ? "lightstack" : "scientific";
  return profileDifferences(recipe, other).length < profileDifferences(recipe, current).length ? other : current;
}

export function kernelProfileNote(recipe: CubePrepRecipe, profile: CubePrepProfile): string | null {
  const kernels = recipe.steps.find((step) => step.step === "kernels");
  if (profile !== "scientific" || !kernels || kernels.auto === true) return null;
  const fixed = kernels.method === "window" ? `regularization ${valueText(kernels.regularization)}` : `eps ${valueText(kernels.eps)}`;
  return `Kernels keep this recipe's fixed ${fixed}; new Kernels steps in this profile choose it automatically. Turn on the automatic choice in Kernels if the matched FWHM misses the target.`;
}

export function importedDraft(current: CubePrepDraft, path: string, text: string): CubePrepDraft {
  const recipe = parseRecipe(text);
  return { ...current, recipe, profile: closestProfile(recipe, current.profile), baseDir: inputDirectory(path), outputParent: current.outputParent || inputDirectory(path) };
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

export function recipeProblemList(recipe: CubePrepRecipe): CubePrepProblem[] {
  const problems: CubePrepProblem[] = [];
  const add = (tab: CubePrepTab, text: string) => { if (!problems.some((problem) => problem.text === text)) problems.push({ tab, text }); };
  if (recipe.inputs.length === 0) add("inputs", "Add at least one input image.");
  const filters = new Set<string>();
  for (const [i, input] of recipe.inputs.entries()) {
    if (![input.path, input.dataset, input.filter].every((value) => value.trim())) add("inputs", `Input ${i + 1} needs a file, dataset and filter.`);
    const filter = input.filter.trim().toLowerCase();
    if (filter && filters.has(filter)) add("inputs", `Duplicate filter: ${input.filter}.`);
    filters.add(filter);
  }
  if (recipe.steps.length === 0) add("inputs", "Enable at least one preparation step.");
  const isInput = (name: string) => recipe.inputs.some((input) => sameFilter(input.filter, name));
  let aligned = false;
  let cube = false;
  let kernels: string | null = null;
  const psfs: string[] = [];
  for (const step of recipe.steps) {
    const label = CUBE_PREP_LABELS[step.step];
    const problem = (text: string) => add(step.step, text);
    const reference = (value: unknown): string | null => {
      const name = typeof value === "string" ? value.trim() : "";
      if (name && isInput(name)) return name;
      problem(name ? `${label}: reference ${name} is not an input filter.` : `Choose a reference filter for ${label}.`);
      return null;
    };
    const needsCube = () => { if (!cube) problem(`${label} needs an earlier Cube step.`); };
    for (const [key, value] of Object.entries(step)) {
      if (value === null && !NULLABLE_KEYS.has(key)) problem(`${label}: enter a value for ${key}.`);
    }
    if (step.step === "crop" && step.mode === "reference") reference(step.reference);
    if (step.step === "align") { reference(step.reference); aligned = true; }
    if (step.step === "cube") {
      if (step.reference !== undefined && step.reference !== null) reference(step.reference);
      else if (!aligned) problem("Cube needs a reference filter or an earlier Align step.");
      cube = true;
    }
    if (step.step === "edit") {
      needsCube();
      if (step.action === "remove_filter") {
        const filter = typeof step.filter === "string" ? step.filter.trim() : "";
        if (!filter) problem("Choose the filter to remove in Edit.");
        else if (!isInput(filter)) problem(`Edit: ${filter} is not an input filter.`);
      }
    }
    if (step.step === "psfs") {
      const names = psfFilters(step);
      for (const name of names) if (!isInput(name)) problem(`PSFs: ${name} is not an input filter.`);
      const unchosen = missingPsfFiles(step);
      if (unchosen.length > 0) problem(`PSFs: choose the PSF file for ${unchosen.join(", ")}.`);
      const target = typeof step.reference === "string" ? step.reference : "";
      if (typeof step.pixel_scale_arcsec === "number" && !names.some((name) => sameFilter(name, target))) problem("PSFs: resampling to a pixel scale needs a reference filter among the PSFs.");
      psfs.push(...names);
    }
    if (step.step === "kernels") {
      if (step.method === "window" && step.window === "hanning" && step.auto === true) problem("Kernels: the hanning window has no parameter to choose automatically; turn off the automatic choice or choose another window.");
      const name = reference(step.reference);
      if (name && !psfs.some((psf) => sameFilter(psf, name))) problem(`Kernels need a PSF for ${name} in an earlier PSFs step.`);
      kernels = typeof step.reference === "string" ? step.reference : null;
    }
    if (step.step === "match") {
      needsCube();
      const name = reference(step.reference);
      if (name && !(kernels !== null && sameFilter(kernels, name))) problem(`Match needs an earlier Kernels step to ${name}.`);
    }
    if (step.step === "export") needsCube();
  }
  return problems;
}

export function recipeProblems(recipe: CubePrepRecipe): string[] {
  return recipeProblemList(recipe).map((problem) => problem.text);
}

export function cubePrepProblems(draft: CubePrepDraft, forRun = true): CubePrepProblem[] {
  const problems = recipeProblemList(draft.recipe);
  if (!draft.baseDir.trim()) problems.push({ tab: "folders", text: "Choose the input folder for relative file paths." });
  if (forRun && !draft.outputParent.trim()) problems.push({ tab: "folders", text: "Choose a working folder for this run." });
  return problems;
}

export function missingPsfFiles(step: CubePrepStep): string[] {
  if (step.step !== "psfs" || step.source !== "files" || !Array.isArray(step.files)) return [];
  return (step.files as unknown[]).flatMap((file) => {
    if (!file || typeof file !== "object") return [];
    const { filter, path } = file as { filter?: unknown; path?: unknown };
    return typeof filter === "string" && filter.trim() && !(typeof path === "string" && path.trim()) ? [filter.trim()] : [];
  });
}

function sameFilter(a: string, b: string): boolean {
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

function psfFilters(step: CubePrepStep): string[] {
  const files: unknown[] = Array.isArray(step.files) ? step.files : [];
  const listed: unknown[] = step.source === "files" ? files.map((file) => file && typeof file === "object" ? (file as { filter?: unknown }).filter : undefined) : Array.isArray(step.filters) ? step.filters : [];
  return listed.filter((name): name is string => typeof name === "string" && name.trim() !== "").map((name) => name.trim());
}

export function changeRecipeInput(recipe: CubePrepRecipe, index: number, change: Partial<CubePrepInput>, renameFrom?: string): CubePrepRecipe {
  const inputs = recipe.inputs.map((input, i) => i === index ? { ...input, ...change } : input);
  const from = renameFrom ?? recipe.inputs[index]?.filter ?? "";
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function editAction(step: Pick<CubePrepStepReport, "parameters" | "report">): string {
  if (isRecord(step.parameters) && typeof step.parameters.action === "string") return step.parameters.action;
  return isRecord(step.report) && isRecord(step.report.edit) && typeof step.report.edit.action === "string" ? step.report.edit.action : "";
}

function keepsPsf(step: Pick<CubePrepStepReport, "step" | "parameters" | "report">): boolean {
  return step.step === "export" || (step.step === "edit" && PSF_NEUTRAL_EDITS.has(editAction(step)));
}

export function cubePrepPsfTarget(report: CubePrepReport, filters: string[]): string {
  let index = report.steps.length - 1;
  while (index >= 0 && keepsPsf(report.steps[index])) index--;
  if (index < 0 || report.steps[index].step !== "match") return "";
  const detail = report.steps[index].report as { reference?: unknown; planes?: { filter?: unknown; status?: unknown }[] } | null;
  if (!detail || typeof detail.reference !== "string" || !detail.reference.trim() || !Array.isArray(detail.planes) || detail.planes.length !== filters.length || filters.length < 2) return "";
  const names = new Set(filters.map((filter) => filter.toLowerCase()));
  const matched: string[] = [];
  let references = 0;
  for (const plane of detail.planes) {
    if (typeof plane.filter !== "string" || !names.delete(plane.filter.toLowerCase()) || !["matched", "reference"].includes(String(plane.status))) return "";
    if (plane.status === "matched") matched.push(plane.filter);
    if (plane.status === "reference") {
      if (plane.filter.toLowerCase() !== detail.reference.toLowerCase()) return "";
      references++;
    }
  }
  return names.size === 0 && references === 1 && matchedWithinTolerance(report, matched) ? detail.reference : "";
}

function matchedWithinTolerance(report: CubePrepReport, matched: string[]): boolean {
  const rows = psfFwhmRows(report.steps);
  return matched.every((filter) => {
    const row = rows.find((entry) => !entry.reference && sameFilter(entry.filter, filter));
    return row !== undefined && row.matchedPx !== null && row.targetPx !== null && Math.abs(row.matchedPx / row.targetPx - 1) <= MATCHED_FWHM_TOLERANCE;
  });
}

export interface CubePrepSummary {
  bands: { filter: string; pivotUm: number | null }[];
  cols: number | null;
  rows: number | null;
  layout: string | null;
  hdus: string[];
  psfTarget: string;
  psfNote: string;
}

function planeFilters(report: Record<string, unknown>): string[] | null {
  if (Array.isArray(report.filters) && report.filters.every((filter) => typeof filter === "string")) return report.filters as string[];
  if (Array.isArray(report.planes) && report.planes.every((plane) => isRecord(plane) && typeof plane.filter === "string")) return (report.planes as { filter: string }[]).map((plane) => plane.filter);
  return null;
}

function tolerancePercent(): number {
  return Math.round(100 * MATCHED_FWHM_TOLERANCE);
}

function psfTargetNote(report: CubePrepReport, target: string): string {
  if (target) return `SAGUI PSF target: ${target}, every matched band within ${tolerancePercent()} % of the target FWHM.`;
  if (!report.steps.some((step) => step.step === "match")) return "SAGUI PSF target not set: the recipe does not match PSFs. Enable PSFs, Kernels and Match.";
  const misses = psfFwhmRows(report.steps)
    .flatMap((row) => { const change = row.reference ? null : fwhmChange(row); return change !== null && Math.abs(change) > MATCHED_FWHM_TOLERANCE ? [{ filter: row.filter, change }] : []; })
    .sort((a, b) => Math.abs(b.change) - Math.abs(a.change));
  if (misses.length === 0) return "SAGUI PSF target not set: the final cube does not follow a Match step that covers every band.";
  const kernels = [...report.steps].reverse().find((step) => step.step === "kernels");
  const automatic = isRecord(kernels?.parameters) && kernels.parameters.auto === true;
  const advice = automatic ? "Choose a broader reference band or another kernel method in Kernels and run again." : "Turn on the automatic choice in Kernels and run again.";
  return `SAGUI PSF target not set: ${misses.map(({ filter, change }) => `${filter} ${signedPercent(change)}`).join(", ")} from the target FWHM (tolerance ${tolerancePercent()} %). ${advice}`;
}

export function cubePrepSummary(report: CubePrepReport): CubePrepSummary | null {
  const reports = report.steps.map((step) => step.report).filter(isRecord);
  const latest = <T>(pick: (entry: Record<string, unknown>) => T | null): T | null => {
    for (let index = reports.length - 1; index >= 0; index--) { const value = pick(reports[index]); if (value !== null) return value; }
    return null;
  };
  const filters = latest(planeFilters);
  if (filters === null) return null;
  const pivots = new Map<string, number>();
  for (const entry of reports) {
    if (!Array.isArray(entry.planes)) continue;
    for (const plane of entry.planes) if (isRecord(plane) && typeof plane.filter === "string" && typeof plane.pivot_um === "number" && Number.isFinite(plane.pivot_um)) pivots.set(plane.filter.toLowerCase(), plane.pivot_um);
  }
  const size = latest((entry) => typeof entry.cols === "number" && typeof entry.rows === "number" ? [entry.cols, entry.rows] : null);
  const psfTarget = cubePrepPsfTarget(report, filters);
  return {
    bands: filters.map((filter) => ({ filter, pivotUm: pivots.get(filter.toLowerCase()) ?? null })),
    cols: size?.[0] ?? null,
    rows: size?.[1] ?? null,
    layout: latest((entry) => typeof entry.layout === "string" ? entry.layout : null),
    hdus: latest((entry) => Array.isArray(entry.hdus) ? entry.hdus.filter((hdu): hdu is string => typeof hdu === "string") : null) ?? [],
    psfTarget,
    psfNote: psfTargetNote(report, psfTarget),
  };
}

export function runFolderLabel(name: string): string {
  return name.trim().replace(/[^a-zA-Z0-9_-]+/g, "-").replace(/^-+|-+$/g, "") || "field";
}

export function workingDirectory(parent: string, name: string, id: string): string {
  return joinProductPath(parent, `cubeprep-${runFolderLabel(name)}-${id}`);
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
