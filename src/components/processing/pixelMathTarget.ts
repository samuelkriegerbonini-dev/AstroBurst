export type PixelMathTarget = "file" | "composite";

export const PIXELMATH_TARGETS: readonly PixelMathTarget[] = ["composite", "file"];
const ARROW_STEPS: ReadonlyMap<string, number> = new Map([
  ["ArrowDown", 1],
  ["ArrowRight", 1],
  ["ArrowUp", -1],
  ["ArrowLeft", -1],
]);

export function initialPixelMathTarget(compositeMode: boolean): PixelMathTarget {
  return compositeMode ? "composite" : "file";
}

export function pixelMathTargetAfterKey(current: PixelMathTarget, key: string): PixelMathTarget | null {
  const step = ARROW_STEPS.get(key);
  if (step === undefined) return null;
  const count = PIXELMATH_TARGETS.length;
  return PIXELMATH_TARGETS[(PIXELMATH_TARGETS.indexOf(current) + step + count) % count];
}

