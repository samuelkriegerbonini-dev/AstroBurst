import type { CosmeticDefect } from "../shared/types/cosmetic";

export interface DefectLineError {
  line: number;
  message: string;
}

export interface DefectListParse {
  defects: CosmeticDefect[];
  errors: DefectLineError[];
}

const COORDINATE_RE = /^[0-9]+$/;

function stripComment(line: string): string {
  const cuts = [line.indexOf("#"), line.indexOf("//")].filter((i) => i >= 0);
  return cuts.length === 0 ? line : line.slice(0, Math.min(...cuts));
}

function parseCoordinates(tokens: string[]): number[] | string {
  const values: number[] = [];
  for (const token of tokens) {
    if (!COORDINATE_RE.test(token)) return `'${token}' is not a non-negative integer`;
    values.push(Number(token));
  }
  return values;
}

function orderedSpan(start: number, end: number): [number, number] | string {
  if (start > end) return `span start ${start} is greater than span end ${end}`;
  return [start, end];
}

function parseLine(line: string): CosmeticDefect | string {
  const [rawKeyword, ...rest] = line.split(/\s+/);
  const keyword = rawKeyword.toLowerCase();
  const coords = parseCoordinates(rest);
  if (typeof coords === "string") return coords;
  switch (keyword) {
    case "point":
      if (coords.length !== 2) return "expected 'Point x y'";
      return { kind: "point", x: coords[0], y: coords[1] };
    case "col":
    case "column": {
      if (coords.length === 1) return { kind: "column", x: coords[0], y0: null, y1: null };
      if (coords.length !== 3) return "expected 'Col x' or 'Col x y0 y1'";
      const span = orderedSpan(coords[1], coords[2]);
      if (typeof span === "string") return span;
      return { kind: "column", x: coords[0], y0: span[0], y1: span[1] };
    }
    case "row": {
      if (coords.length === 1) return { kind: "row", y: coords[0], x0: null, x1: null };
      if (coords.length !== 3) return "expected 'Row y' or 'Row y x0 x1'";
      const span = orderedSpan(coords[1], coords[2]);
      if (typeof span === "string") return span;
      return { kind: "row", y: coords[0], x0: span[0], x1: span[1] };
    }
    default:
      return `unknown keyword '${rawKeyword}'; expected Point, Col or Row`;
  }
}

export function parseDefectList(text: string): DefectListParse {
  const defects: CosmeticDefect[] = [];
  const errors: DefectLineError[] = [];
  text.split(/\r?\n/).forEach((raw, index) => {
    const line = stripComment(raw).trim();
    if (line.length === 0) return;
    const parsed = parseLine(line);
    if (typeof parsed === "string") {
      errors.push({ line: index + 1, message: parsed });
    } else {
      defects.push(parsed);
    }
  });
  return { defects, errors };
}

export function formatDefectError(error: DefectLineError): string {
  return `Line ${error.line}: ${error.message}`;
}
