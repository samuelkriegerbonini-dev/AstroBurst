import { withPreview } from "../infrastructure/tauri";
import { isCurveIdentity, isLevelsIdentity, toCurveInput, type CurvePoint, type LevelsParams } from "./tone";
import type { HistoryScope, OpSpec } from "../shared/types/ops";
import { recordOp } from "./ops";
import type { RecordedResult } from "./processing";

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

export function toneOp(options: ToneFileOptions): OpSpec {
  const levels = activeLevels(options);
  const curve = activeCurve(options);
  return {
    op: "tone",
    v: 1,
    ...(levels ? { levels: { black: levels.black, gamma: levels.gamma, white: levels.white } } : {}),
    ...(curve ? { curve: { points: toCurveInput(curve).points as [number, number][] } } : {}),
  };
}

export interface ToneSettings {
  levels: LevelsParams;
  curve: CurvePoint[];
}

export function toneOptionsOf(op: OpSpec): ToneSettings | null {
  if (op.op !== "tone") return null;
  return {
    levels: op.levels ? { black: op.levels.black, gamma: op.levels.gamma, white: op.levels.white } : { ...IDENTITY_LEVELS },
    curve: op.curve ? op.curve.points.map(([x, y]) => ({ x, y })) : IDENTITY_CURVE.map((p) => ({ ...p })),
  };
}

export async function recordToneFile(scope: HistoryScope, path: string, options: ToneFileOptions): Promise<RecordedResult<ToneFileResult>> {
  return (await recordOp(scope, path, toneOp(options))) as RecordedResult<ToneFileResult>;
}
