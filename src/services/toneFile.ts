import { withPreview } from "../infrastructure/tauri";
import { isCurveIdentity, isLevelsIdentity, toCurveInput, type CurvePoint, type LevelsParams } from "./tone";

export const TONE_IDENTITY_MESSAGE = "Nothing to apply: the levels and the curve are both identity.";

export const IDENTITY_LEVELS: LevelsParams = { black: 0, gamma: 1, white: 1 };
export const IDENTITY_CURVE: readonly CurvePoint[] = [{ x: 0, y: 0 }, { x: 1, y: 1 }];

export interface ToneFileOptions {
  levels?: LevelsParams;
  curve?: CurvePoint[];
}

export interface ToneFileResult {
  png_path: string;
  fits_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  is_rgb: boolean;
  levels_applied: boolean;
  curves_applied: boolean;
  input_normalized: { min: number; max: number } | null;
  elapsed_ms: number;
}

function activeLevels(options: ToneFileOptions): LevelsParams | null {
  return options.levels && !isLevelsIdentity(options.levels) ? options.levels : null;
}

function activeCurve(options: ToneFileOptions): CurvePoint[] | null {
  return options.curve && !isCurveIdentity(options.curve) ? options.curve : null;
}

export function isToneIdentity(options: ToneFileOptions): boolean {
  return activeLevels(options) === null && activeCurve(options) === null;
}

export function applyToneFile(path: string, outputDir: string | undefined, options: ToneFileOptions): Promise<ToneFileResult> {
  const args: Record<string, unknown> = { path };
  const levels = activeLevels(options);
  const curve = activeCurve(options);
  if (levels) args.levels = levels;
  if (curve) args.curve = toCurveInput(curve);
  return withPreview<ToneFileResult>("apply_tone_cmd", outputDir, args);
}
