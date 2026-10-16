import type { MastColumn, MastMissionId, MastRow } from "../../../shared/types/mast";

export const MAST_PUBLIC_BASE = "https://mast.stsci.edu";

const TOKEN_MISSIONS: readonly MastMissionId[] = ["roman", "roman_cgi"];

export function requestJson(body: unknown): string {
  return JSON.stringify(body ?? null, null, 2);
}

function shellQuote(text: string): string {
  return `'${text.replace(/'/g, `'\\''`)}'`;
}

export function curlCommand(mission: MastMissionId, body: unknown): string {
  const parts = [`curl -X POST ${MAST_PUBLIC_BASE}/search/${mission}/api/v0.1/search`, `-H "Content-Type: application/json"`];
  if (TOKEN_MISSIONS.includes(mission)) parts.push(`-H "Authorization: token $MAST_API_TOKEN"`);
  parts.push(`-d ${shellQuote(JSON.stringify(body ?? null))}`);
  return parts.join(" ");
}

function flat(text: string): string {
  return text.replace(/[\t\r\n]+/g, " ");
}

function cell(value: string | number | boolean | null | undefined): string {
  return value === null || value === undefined ? "" : flat(String(value));
}

export function rowTsv(row: MastRow, columns: readonly MastColumn[]): string {
  const header = columns.map((c) => flat(c.label)).join("\t");
  const values = columns.map((c) => cell(row.values[c.name])).join("\t");
  return `${header}\n${values}`;
}

export function coordinatesText(raDeg: number, decDeg: number): string {
  return `${raDeg.toFixed(5)} ${decDeg.toFixed(5)}`;
}
