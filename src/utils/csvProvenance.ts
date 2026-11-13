import { APP_VERSION } from "./constants";

export type ProvenanceValue = string | number | null | undefined;
export type ProvenanceEntry = readonly [key: string, value: ProvenanceValue];

export const PROVENANCE_FILE_SEPARATOR = "; ";
export const PIXEL_SUM_SUFFIX = " x pix";

const LINE_BREAK_RUN = /\s*[\r\n]\s*/g;
const KEY_BREAK_RUN = /\s*[\r\n:][\s:]*/g;
const PER_PIXEL_AREA_UNIT = /\/\s*(sr|arcsec2|arcsec\^2|beam)$/i;

function valueText(value: ProvenanceValue): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "number") return Number.isFinite(value) ? String(value) : "";
  return value.replace(LINE_BREAK_RUN, " ").trim();
}

function keyText(key: string): string {
  return key.replace(KEY_BREAK_RUN, " ").trim();
}

export function provenanceLines(entries: readonly ProvenanceEntry[]): string[] {
  const lines: string[] = [];
  for (const [key, value] of entries) {
    const cleanKey = keyText(key);
    if (cleanKey === "") continue;
    const cleanValue = valueText(value);
    lines.push(cleanValue === "" ? `# ${cleanKey}:` : `# ${cleanKey}: ${cleanValue}`);
  }
  return lines;
}

export function unitLines(units: Readonly<Record<string, string | null | undefined>>): string[] {
  const lines: string[] = [];
  for (const [column, unit] of Object.entries(units)) {
    const cleanColumn = keyText(column);
    const cleanUnit = valueText(unit);
    if (cleanColumn === "" || cleanUnit === "") continue;
    lines.push(`# unit ${cleanColumn}: ${cleanUnit}`);
  }
  return lines;
}

export function withProvenance(lines: readonly string[], body: string, lineEnd: string): string {
  return lines.map((line) => line + lineEnd).join("") + body;
}

export function exportEntries(
  file: string | null | undefined | readonly (string | null | undefined)[],
  exportedUtc: string = new Date().toISOString(),
): ProvenanceEntry[] {
  const given = typeof file === "string" || file === null || file === undefined ? [file] : file;
  const names: string[] = [];
  for (const name of given) {
    const cleanName = valueText(name);
    if (cleanName !== "" && !names.includes(cleanName)) names.push(cleanName);
  }
  return [
    ["software", "AstroBurst " + APP_VERSION],
    ["exported_utc", exportedUtc],
    ["file", names.length === 0 ? null : names.join(PROVENANCE_FILE_SEPARATOR)],
  ];
}

export function summedPixelUnit(pixelUnit: string | null | undefined): string | null {
  const unit = valueText(pixelUnit);
  if (unit === "") return null;
  return PER_PIXEL_AREA_UNIT.test(unit) ? unit + PIXEL_SUM_SUFFIX : unit;
}
