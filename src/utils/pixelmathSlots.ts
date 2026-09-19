export const TARGET_SYMBOL = "$T";
export const MAX_SLOTS = 26;

const IDENTIFIER_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;
const LETTER_A = 65;
const ALPHABET_SIZE = 26;
const OVERFLOW_PREFIX = "I";

export const RESERVED_NAMES: readonly string[] = [
  "abs", "sqrt", "exp", "ln", "log", "log2", "pow", "min", "max",
  "floor", "ceil", "round", "trunc", "sign", "clip", "rescale", "iif",
  "pi", "e", "mean", "med", "mdev", "sdev", "adev",
];

export interface ExampleExpression {
  label: string;
  expression: string;
}

export const EXAMPLE_EXPRESSIONS: readonly ExampleExpression[] = [
  { label: "Subtract median background", expression: "$T - med($T)" },
  { label: "Fill NaN with 0", expression: "iif($T != $T, 0, $T)" },
  { label: "Average three images", expression: "(A + B + C) / 3" },
  { label: "Invert", expression: "~$T" },
  { label: "Normalize to [0, 1]", expression: "rescale($T, min($T), max($T), 0, 1)" },
  { label: "Flat-field by A", expression: "$T * (A / med(A))" },
];

export function isValidSlotIdentifier(name: string): boolean {
  return IDENTIFIER_RE.test(name);
}

export function validateSlotName(name: string, otherNames: readonly string[]): string | null {
  const trimmed = name.trim();
  if (trimmed.length === 0) return "Slot name is required";
  if (trimmed === TARGET_SYMBOL) return `${TARGET_SYMBOL} is reserved for the target image`;
  if (trimmed.startsWith("$")) return "Names starting with $ are reserved";
  if (!isValidSlotIdentifier(trimmed)) return "Use letters, digits and underscores, starting with a letter";
  if (RESERVED_NAMES.includes(trimmed)) return `'${trimmed}' is a function name`;
  if (otherNames.includes(trimmed)) return `'${trimmed}' is already used`;
  return null;
}

export function nextSlotName(existing: readonly string[]): string {
  for (let i = 0; i < ALPHABET_SIZE; i++) {
    const candidate = String.fromCharCode(LETTER_A + i);
    if (!existing.includes(candidate)) return candidate;
  }
  let n = 1;
  while (existing.includes(`${OVERFLOW_PREFIX}${n}`)) n++;
  return `${OVERFLOW_PREFIX}${n}`;
}

export function slotErrors(slots: readonly { name: string }[]): (string | null)[] {
  const names = slots.map((s) => s.name.trim());
  return names.map((name, i) => validateSlotName(name, names.filter((_, j) => j !== i)));
}

export interface CaretLines {
  line: string;
  marker: string;
}

export function caretLines(expression: string, position: number, length: number): CaretLines {
  const clamped = Math.max(0, Math.min(position, expression.length));
  const lineStart = clamped === 0 ? 0 : expression.lastIndexOf("\n", clamped - 1) + 1;
  const newlineAfter = expression.indexOf("\n", clamped);
  const lineEnd = newlineAfter === -1 ? expression.length : newlineAfter;
  const column = clamped - lineStart;
  const available = lineEnd - clamped;
  const width = Math.max(1, Math.min(Math.max(1, length), Math.max(1, available)));
  return {
    line: expression.slice(lineStart, lineEnd),
    marker: " ".repeat(column) + "^".repeat(width),
  };
}
