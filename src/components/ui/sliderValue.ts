function fractionDigits(n: number): number {
  const text = String(Math.abs(n));
  if (text.includes("e")) return -1;
  const dot = text.indexOf(".");
  return dot < 0 ? 0 : text.length - dot - 1;
}

function alignToGrid(value: number, min: number, step: number): number {
  const raw = min + Math.round((value - min) / step) * step;
  const stepDigits = fractionDigits(step);
  const minDigits = fractionDigits(min);
  if (stepDigits < 0 || minDigits < 0) return raw;
  return Number(raw.toFixed(Math.max(stepDigits, minDigits)));
}

export function snapToStep(value: number, min: number, max: number, step: number): number {
  if (!Number.isFinite(value)) return value;
  const snapped = Number.isFinite(step) && step > 0 ? alignToGrid(value, min, step) : value;
  return clampToRange(snapped, min, max);
}

export function clampToRange(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return value;
  return Math.max(min, Math.min(max, value));
}

export function resolveTypedValue(
  value: number,
  min: number,
  max: number,
  step: number,
  isLog: boolean,
): number {
  return isLog ? clampToRange(value, min, max) : snapToStep(value, min, max, step);
}
