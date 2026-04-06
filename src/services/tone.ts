import { typedInvoke, getOutputDir } from "../infrastructure/tauri";
import { getPreviewUrl } from "../infrastructure/tauri/client";
import type { StfParams } from "../shared/types";

export interface LevelsParams {
  black: number;
  gamma: number;
  white: number;
}

export interface CurvePoint {
  x: number;
  y: number;
}

export interface CurveInput {
  points: [number, number][];
}

export interface ToneResult {
  png_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  composite_dims: [number, number];
  stf_applied: boolean;
  levels_applied: boolean;
  curves_applied: boolean;
  scnr_applied: boolean;
  stf: {
    r: StfParams;
    g: StfParams;
    b: StfParams;
  };
  elapsed_ms: number;
}

function isLevelsIdentity(p: LevelsParams): boolean {
  return Math.abs(p.black) < 1e-6
    && Math.abs(p.gamma - 1) < 1e-6
    && Math.abs(p.white - 1) < 1e-6;
}

function isCurveIdentity(points: CurvePoint[]): boolean {
  if (points.length === 0) return true;
  if (points.length > 2) return false;
  if (points.length === 1) return Math.abs(points[0].x - points[0].y) < 1e-6;
  const nearStart = Math.abs(points[0].x) < 1e-6 && Math.abs(points[0].y) < 1e-6;
  const nearEnd = Math.abs(points[1].x - 1) < 1e-6 && Math.abs(points[1].y - 1) < 1e-6;
  return nearStart && nearEnd;
}

function toCurveInput(points: CurvePoint[]): CurveInput {
  return { points: points.map((p) => [p.x, p.y]) };
}

function toStfArray(stf: StfParams): [number, number, number] {
  return [stf.shadow, stf.midtone, stf.highlight];
}

export async function applyToneComposite(options: {
  outputDir?: string;
  stfR?: StfParams;
  stfG?: StfParams;
  stfB?: StfParams;
  linkedStf?: boolean;
  levelsR?: LevelsParams;
  levelsG?: LevelsParams;
  levelsB?: LevelsParams;
  curvesR?: CurvePoint[];
  curvesG?: CurvePoint[];
  curvesB?: CurvePoint[];
  scnr?: { method: string; amount: number; preserveLuminance: boolean } | null;
}): Promise<ToneResult> {
  const dir = options.outputDir ?? await getOutputDir();

  const args: Record<string, unknown> = {
    outputDir: dir,
    linkedStf: options.linkedStf ?? false,
  };

  if (options.stfR) args.stfR = toStfArray(options.stfR);
  if (options.stfG) args.stfG = toStfArray(options.stfG);
  if (options.stfB) args.stfB = toStfArray(options.stfB);

  if (options.levelsR && !isLevelsIdentity(options.levelsR)) args.levelsR = options.levelsR;
  if (options.levelsG && !isLevelsIdentity(options.levelsG)) args.levelsG = options.levelsG;
  if (options.levelsB && !isLevelsIdentity(options.levelsB)) args.levelsB = options.levelsB;

  if (options.curvesR && !isCurveIdentity(options.curvesR)) args.curvesR = toCurveInput(options.curvesR);
  if (options.curvesG && !isCurveIdentity(options.curvesG)) args.curvesG = toCurveInput(options.curvesG);
  if (options.curvesB && !isCurveIdentity(options.curvesB)) args.curvesB = toCurveInput(options.curvesB);

  if (options.scnr) args.scnr = options.scnr;

  const res = await typedInvoke<ToneResult>("apply_tone_composite_cmd", args);

  if (res.png_path) {
    res.previewUrl = await getPreviewUrl(res.png_path);
  }

  return res;
}
