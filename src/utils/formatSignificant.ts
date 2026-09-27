const DEFAULT_DIGITS = 4;

export function formatSignificant(value: number, digits: number = DEFAULT_DIGITS): string {
  if (!Number.isFinite(value)) return "—";
  if (value === 0) return "0";
  if (Math.abs(value) >= 10 ** (digits - 1)) return String(Math.round(value));
  return String(Number(value.toPrecision(digits)));
}
