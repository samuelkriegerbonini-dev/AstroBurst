import { typedInvoke, withPreview } from "../infrastructure/tauri";
import type { PixelMathResult, PixelMathSlot, PixelMathValidation } from "../shared/types/pixelmath";
import type { HistoryScope, OpSpec } from "../shared/types/ops";
import { recordOp } from "./ops";
import type { RecordedResult } from "./processing";

export interface PixelMathOptions {
  slots?: PixelMathSlot[];
  truncate?: boolean;
  rescale?: boolean;
  name?: string | null;
}

export function runPixelMath(
  path: string,
  outputDir: string | undefined,
  expression: string,
  options: PixelMathOptions = {},
): Promise<PixelMathResult> {
  const name = options.name?.trim();
  return withPreview<PixelMathResult>("pixelmath_cmd", outputDir, {
    path,
    expression,
    slots: (options.slots ?? []).map((s) => ({ name: s.name.trim(), path: s.path })),
    truncate: options.truncate ?? false,
    rescale: options.rescale ?? false,
    name: name ? name : null,
  });
}

export interface PixelMathSettings {
  expression: string;
  slots: PixelMathSlot[];
  truncate: boolean;
  rescale: boolean;
}

export function pixelMathOp(expression: string, options: PixelMathOptions = {}): OpSpec {
  return {
    op: "pixelmath",
    v: 1,
    expression,
    slots: (options.slots ?? []).map((s) => ({ name: s.name.trim(), ref: s.path })),
    truncate: options.truncate ?? false,
    rescale: options.rescale ?? false,
  };
}

export function pixelMathOptionsOf(op: OpSpec): PixelMathSettings | null {
  if (op.op !== "pixelmath") return null;
  return { expression: op.expression, slots: op.slots.map((s) => ({ name: s.name, path: s.ref })), truncate: op.truncate, rescale: op.rescale };
}

export async function recordPixelMath(
  scope: HistoryScope,
  path: string,
  expression: string,
  options: PixelMathOptions = {},
): Promise<RecordedResult<PixelMathResult>> {
  return (await recordOp(scope, path, pixelMathOp(expression, options))) as RecordedResult<PixelMathResult>;
}

export function validatePixelMath(expression: string, slotNames: string[]): Promise<PixelMathValidation> {
  return typedInvoke<PixelMathValidation>("pixelmath_validate_cmd", { expression, slotNames });
}
