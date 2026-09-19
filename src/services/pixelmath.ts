import { typedInvoke, withPreview } from "../infrastructure/tauri";
import type { PixelMathResult, PixelMathSlot, PixelMathValidation } from "../shared/types/pixelmath";

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

export function validatePixelMath(expression: string, slotNames: string[]): Promise<PixelMathValidation> {
  return typedInvoke<PixelMathValidation>("pixelmath_validate_cmd", { expression, slotNames });
}
