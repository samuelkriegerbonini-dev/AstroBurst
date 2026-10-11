export function parseDecimalText(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : null;
}

export interface DecimalFieldOptions {
  min?: number;
  max?: number;
  integer?: boolean;
  optional?: boolean;
}

export interface DecimalFieldState {
  value: number | null;
  error: string | null;
}

const EMPTY_ERROR = "Enter a number";
const COMMA_ERROR = "Use a dot (.) as the decimal separator";
const NOT_A_NUMBER_ERROR = "Not a number";
const INTEGER_ERROR = "Use a whole number";

function rangeError(value: number, min: number | undefined, max: number | undefined): string | null {
  const below = min !== undefined && value < min;
  const above = max !== undefined && value > max;
  if (!below && !above) return null;
  if (min !== undefined && max !== undefined) return `Use a value from ${min} to ${max}`;
  return below ? `Use a value of at least ${min}` : `Use a value of at most ${max}`;
}

function invalid(error: string): DecimalFieldState {
  return { value: null, error };
}

export function decimalFieldState(text: string, options: DecimalFieldOptions = {}): DecimalFieldState {
  const { min, max, integer = false, optional = false } = options;
  if (text.trim() === "") return optional ? { value: null, error: null } : invalid(EMPTY_ERROR);
  if (text.includes(",")) return invalid(COMMA_ERROR);
  const value = parseDecimalText(text);
  if (value === null) return invalid(NOT_A_NUMBER_ERROR);
  if (integer && !Number.isInteger(value)) return invalid(INTEGER_ERROR);
  const outOfRange = rangeError(value, min, max);
  return outOfRange === null ? { value, error: null } : invalid(outOfRange);
}

export function decimalDraftText(value: number | null): string {
  return value === null ? "" : String(value);
}

export function decimalDraftFor(draft: string, value: number | null, options: DecimalFieldOptions = {}): string {
  return decimalFieldState(draft, options).value === value ? draft : decimalDraftText(value);
}

export function decimalEditValue(text: string, current: number | null, options: DecimalFieldOptions = {}): number | null | undefined {
  const { value, error } = decimalFieldState(text, options);
  return error === null && value !== current ? value : undefined;
}

export function firstFieldError(fields: ReadonlyArray<readonly [string, DecimalFieldState]>): string | null {
  const invalidField = fields.find(([, state]) => state.error !== null);
  return invalidField ? `${invalidField[0]}: ${invalidField[1].error}` : null;
}
