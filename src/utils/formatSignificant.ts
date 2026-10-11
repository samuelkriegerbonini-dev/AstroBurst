const DEFAULT_DIGITS = 4;

export function formatSignificant(value: number, digits: number = DEFAULT_DIGITS): string {
  if (!Number.isFinite(value)) return "—";
  if (value === 0) return "0";
  if (Math.abs(value) >= 10 ** (digits - 1)) return String(Math.round(value));
  return String(Number(value.toPrecision(digits)));
}

export function formatSignificantFixed(value: number, digits: number = DEFAULT_DIGITS): string {
  if (!Number.isFinite(value) || value === 0) return formatSignificant(value, digits);
  const magnitude = Math.floor(Math.log10(Math.abs(value)));
  if (magnitude < digits) return formatSignificant(value, digits);
  const scale = 10 ** (magnitude - digits + 1);
  return String(Math.sign(value) * Math.round(Math.abs(value) / scale) * scale);
}
