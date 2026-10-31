import type { MarkMode, MaskKind, MaskMods, MaskRecipe, MaskShapesSystem } from "../../shared/types/masks";
import type { Region, RegionShape } from "../../shared/types/regions";
import { decimalFieldState, firstFieldError } from "../../utils/decimalText";
import { isExcludeRegion } from "../../utils/regionExclude";
import { MARK_RANGE_ORDER_REASON, NO_MASK_FILE_REASON, NO_REGIONS_REASON, RANGE_ORDER_REASON, REGIONS_ON_COMPOSITE_NOTE } from "./maskText";

export interface MaskForm {
  kind: MaskKind;
  autoStf: boolean;
  lo: string;
  hi: string;
  fuzz: string;
  sigma: number;
  growth: number;
  softness: number;
  minFwhm: number;
  maxFwhm: number;
  maxEccentricity: number;
  luminanceProtect: boolean;
  luminanceCeiling: number;
  filePath: string | null;
  markLo: string;
  markHi: string;
  featherPx: number;
  invert: boolean;
}

export const DEFAULT_MASK_FORM: MaskForm = {
  kind: "luminance",
  autoStf: false,
  lo: "",
  hi: "",
  fuzz: "",
  sigma: 5,
  growth: 2.5,
  softness: 4,
  minFwhm: 1.5,
  maxFwhm: 30,
  maxEccentricity: 0.85,
  luminanceProtect: false,
  luminanceCeiling: 0.85,
  filePath: null,
  markLo: "",
  markHi: "",
  featherPx: 0,
  invert: false,
};

export interface MaskFormContext {
  mark: MarkMode;
  regions: readonly Region[];
  compositeMode: boolean;
}

export type MaskRequest =
  | { ok: true; recipe: MaskRecipe; mods: MaskMods; shapesSystem: MaskShapesSystem | null }
  | { ok: false; reason: string };

export interface RegionSplit {
  shapes: RegionShape[];
  excludeShapes: RegionShape[];
  skipped: number;
}

const AREALESS: ReadonlySet<RegionShape["shape"]> = new Set(["line", "point"]);

export function regionSplit(regions: readonly Region[]): RegionSplit {
  const split: RegionSplit = { shapes: [], excludeShapes: [], skipped: 0 };
  for (const region of regions) {
    if (isExcludeRegion(region)) split.excludeShapes.push(region.shape);
    else if (AREALESS.has(region.shape.shape)) split.skipped++;
    else split.shapes.push(region.shape);
  }
  return split;
}

function rangeRecipe(form: MaskForm): MaskRecipe | string {
  const lo = decimalFieldState(form.lo);
  const hi = decimalFieldState(form.hi);
  const fuzz = decimalFieldState(form.fuzz, { min: 0, optional: true });
  const invalid = firstFieldError([["Low", lo], ["High", hi], ["Fuzz", fuzz]]);
  if (invalid) return invalid;
  if (lo.value === null || hi.value === null || lo.value > hi.value) return RANGE_ORDER_REASON;
  return { kind: "range", lo: lo.value, hi: hi.value, fuzz: fuzz.value ?? 0 };
}

function fileRecipe(form: MaskForm, mark: MarkMode): MaskRecipe | string {
  if (!form.filePath) return NO_MASK_FILE_REASON;
  if (mark !== "range") return { kind: "file", path: form.filePath, mark };
  const lo = decimalFieldState(form.markLo);
  const hi = decimalFieldState(form.markHi);
  const invalid = firstFieldError([["Mark Low", lo], ["Mark High", hi]]);
  if (invalid) return invalid;
  if (lo.value === null || hi.value === null || lo.value > hi.value) return MARK_RANGE_ORDER_REASON;
  return { kind: "file", path: form.filePath, mark, lo: lo.value, hi: hi.value };
}

function regionsRecipe(context: MaskFormContext): MaskRecipe | string {
  if (context.compositeMode) return REGIONS_ON_COMPOSITE_NOTE;
  const split = regionSplit(context.regions);
  if (split.shapes.length === 0) return NO_REGIONS_REASON;
  return { kind: "regions", shapes: split.shapes, exclude_shapes: split.excludeShapes };
}

function recipeOf(form: MaskForm, context: MaskFormContext): MaskRecipe | string {
  switch (form.kind) {
    case "luminance":
      return { kind: "luminance", auto_stf: form.autoStf };
    case "range":
      return rangeRecipe(form);
    case "stars":
      return {
        kind: "stars",
        sigma: form.sigma,
        config: {
          growth_factor: form.growth,
          softness: form.softness,
          min_fwhm: form.minFwhm,
          max_fwhm: form.maxFwhm,
          max_eccentricity: form.maxEccentricity,
          luminance_protect: form.luminanceProtect,
          luminance_ceiling: form.luminanceCeiling,
        },
      };
    case "regions":
      return regionsRecipe(context);
    case "file":
      return fileRecipe(form, context.mark);
  }
}

export function maskRequestOf(form: MaskForm, context: MaskFormContext): MaskRequest {
  const recipe = recipeOf(form, context);
  if (typeof recipe === "string") return { ok: false, reason: recipe };
  return {
    ok: true,
    recipe,
    mods: { feather_px: form.featherPx, invert: form.invert },
    shapesSystem: recipe.kind === "regions" ? "image" : null,
  };
}
