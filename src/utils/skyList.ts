export const MAX_TARGETS = 20000;

export interface SkyTarget {
  label: string;
  lon: number;
  lat: number;
  raw: string;
}

export interface SkyListParse {
  targets: SkyTarget[];
  errors: string[];
}

export type SkyAxis = "lon" | "lat";

const DEG_PER_HOUR = 15;
const HOURS_PER_TURN = 24;
const DEG_PER_TURN = 360;
const MAX_LAT_DEG = 90;
const SEXAGESIMAL_FIELDS = 3;
const SEXAGESIMAL_PATTERN = /^([+-]?)(\d{1,3})\s*([hd:°\s])\s*(\d{1,2})\s*[m':\s]\s*(\d{1,2}(?:\.\d*)?)\s*["s]?$/i;
const DECIMAL_PATTERN = /^[+-]?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?$/i;
const COMMENT_MARK = "#";
const LIST_SEPARATOR = /[,\s]+/;
const LON_ALIASES = ["ra", "ra_deg", "raj2000", "ra_icrs", "lon", "l", "glon", "elon"];
const LAT_ALIASES = ["dec", "de", "dec_deg", "dej2000", "dec_icrs", "lat", "b", "glat", "elat"];
const LABEL_ALIASES = ["id", "name", "label", "source_id", "designation", "main_id"];
const CSV_DELIMITERS = [",", "\t", ";"];

function isDecimalToken(token: string): boolean {
  return DECIMAL_PATTERN.test(token);
}

function stripComment(line: string): string {
  const at = line.indexOf(COMMENT_MARK);
  return (at >= 0 ? line.slice(0, at) : line).trim();
}

interface SexagesimalParts {
  sign: number;
  whole: number;
  minutes: number;
  seconds: number;
  unitLetter: string;
}

function splitSexagesimal(text: string): SexagesimalParts | null {
  const m = SEXAGESIMAL_PATTERN.exec(text.trim());
  if (!m) return null;
  return {
    sign: m[1] === "-" ? -1 : 1,
    whole: Number(m[2]),
    minutes: Number(m[4]),
    seconds: Number(m[5]),
    unitLetter: m[3].toLowerCase(),
  };
}

export function parseAngle(text: string, axis: SkyAxis): { value: number | null; error: string | null } {
  const trimmed = text.trim();
  if (!trimmed) return { value: null, error: `${axis === "lon" ? "RA" : "Dec"} is empty` };
  if (isDecimalToken(trimmed)) {
    const value = Number(trimmed);
    if (!Number.isFinite(value)) return { value: null, error: `${trimmed} is not a finite number` };
    return checkRange(value, axis);
  }
  const parts = splitSexagesimal(trimmed);
  if (!parts) return { value: null, error: `${trimmed} is neither decimal degrees nor sexagesimal` };
  if (parts.minutes >= 60 || parts.seconds >= 60) {
    return { value: null, error: `${trimmed} has minutes or seconds of 60 or more` };
  }
  const total = parts.whole + parts.minutes / 60 + parts.seconds / 3600;
  const inHours = axis === "lon" && parts.unitLetter !== "d";
  if (inHours) {
    if (total > HOURS_PER_TURN) return { value: null, error: `RA ${trimmed} exceeds ${HOURS_PER_TURN} hours` };
    return { value: wrapLongitude(parts.sign * total * DEG_PER_HOUR), error: null };
  }
  return checkRange(parts.sign * total, axis);
}

function wrapLongitude(deg: number): number {
  return ((deg % DEG_PER_TURN) + DEG_PER_TURN) % DEG_PER_TURN;
}

function checkRange(value: number, axis: SkyAxis): { value: number | null; error: string | null } {
  if (axis === "lat") {
    if (Math.abs(value) > MAX_LAT_DEG) return { value: null, error: `Dec ${value} is outside -90..90 degrees` };
    return { value, error: null };
  }
  if (value < -DEG_PER_TURN || value > DEG_PER_TURN) {
    return { value: null, error: `RA ${value} is outside -360..360 degrees` };
  }
  return { value: wrapLongitude(value), error: null };
}

function parseListLine(line: string): { target: Omit<SkyTarget, "raw"> | null; error: string | null } {
  const tokens = line.split(LIST_SEPARATOR).filter((t) => t.length > 0);
  if (tokens.length < 2) return { target: null, error: "needs at least RA and Dec" };
  let leadingNumeric = 0;
  while (leadingNumeric < tokens.length && isDecimalToken(tokens[leadingNumeric])) leadingNumeric++;
  const fieldsPerAngle = leadingNumeric >= 2 * SEXAGESIMAL_FIELDS ? SEXAGESIMAL_FIELDS : 1;
  if (tokens.length < 2 * fieldsPerAngle) return { target: null, error: "needs at least RA and Dec" };
  const lonText = tokens.slice(0, fieldsPerAngle).join(" ");
  const latText = tokens.slice(fieldsPerAngle, 2 * fieldsPerAngle).join(" ");
  const label = tokens.slice(2 * fieldsPerAngle).join(" ");
  const lon = parseAngle(lonText, "lon");
  if (lon.value === null) return { target: null, error: lon.error };
  const lat = parseAngle(latText, "lat");
  if (lat.value === null) return { target: null, error: lat.error };
  return { target: { label, lon: lon.value, lat: lat.value }, error: null };
}

function finish(targets: SkyTarget[], errors: string[], skipped: number): SkyListParse {
  if (skipped > 0) errors.push(`only the first ${MAX_TARGETS} targets are kept, ${skipped} more were ignored`);
  return { targets, errors };
}

export function parseSkyList(text: string): SkyListParse {
  const targets: SkyTarget[] = [];
  const errors: string[] = [];
  let skipped = 0;
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = stripComment(lines[i]);
    if (!line) continue;
    const parsed = parseListLine(line);
    if (!parsed.target) {
      errors.push(`line ${i + 1}: ${parsed.error}`);
      continue;
    }
    if (targets.length >= MAX_TARGETS) {
      skipped++;
      continue;
    }
    targets.push({ ...parsed.target, raw: line });
  }
  return finish(targets, errors, skipped);
}

function detectDelimiter(line: string): string {
  return CSV_DELIMITERS.find((d) => line.includes(d)) ?? " ";
}

function splitCells(line: string, delimiter: string): string[] {
  const cells = delimiter === " " ? line.split(/\s+/) : line.split(delimiter);
  return cells.map((c) => c.trim().replace(/^"(.*)"$/, "$1"));
}

function findColumn(headers: string[], aliases: string[]): number {
  const lowered = headers.map((h) => h.trim().toLowerCase());
  for (const alias of aliases) {
    const idx = lowered.indexOf(alias);
    if (idx >= 0) return idx;
  }
  return -1;
}

export interface SkyCsvHeader {
  delimiter: string;
  lon: number;
  lat: number;
  label: number;
}

export function detectSkyCsvHeader(line: string): SkyCsvHeader | null {
  const delimiter = detectDelimiter(line);
  const headers = splitCells(line, delimiter);
  const lon = findColumn(headers, LON_ALIASES);
  const lat = findColumn(headers, LAT_ALIASES);
  if (lon < 0 || lat < 0 || lon === lat) return null;
  return { delimiter, lon, lat, label: findColumn(headers, LABEL_ALIASES) };
}

export function parseSkyCsv(text: string): SkyListParse {
  const lines = text.split(/\r?\n/);
  const headerIndex = lines.findIndex((l) => stripComment(l).length > 0);
  if (headerIndex < 0) return { targets: [], errors: [] };
  const header = detectSkyCsvHeader(stripComment(lines[headerIndex]));
  if (!header) return parseSkyList(text);
  const targets: SkyTarget[] = [];
  const errors: string[] = [];
  let skipped = 0;
  for (let i = headerIndex + 1; i < lines.length; i++) {
    const line = stripComment(lines[i]);
    if (!line) continue;
    const cells = splitCells(line, header.delimiter);
    const lon = parseAngle(cells[header.lon] ?? "", "lon");
    if (lon.value === null) {
      errors.push(`line ${i + 1}: ${lon.error}`);
      continue;
    }
    const lat = parseAngle(cells[header.lat] ?? "", "lat");
    if (lat.value === null) {
      errors.push(`line ${i + 1}: ${lat.error}`);
      continue;
    }
    if (targets.length >= MAX_TARGETS) {
      skipped++;
      continue;
    }
    const label = header.label >= 0 ? (cells[header.label] ?? "") : "";
    targets.push({ label, lon: lon.value, lat: lat.value, raw: line });
  }
  return finish(targets, errors, skipped);
}
