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
const SPACED_PAIR_EXTRA_CELLS = 2 * (SEXAGESIMAL_FIELDS - 1);
const WHITESPACE_CELL = /"[^"]*"|\S+/g;
const SEXAGESIMAL_PATTERN = /^([+-]?)(\d{1,3})\s*([hd:°\s])\s*(\d{1,2})\s*[m':\s]\s*(\d{1,2}(?:\.\d*)?)\s*["s]?$/i;
const DECIMAL_PATTERN = /^[+-]?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?$/i;
const COMMENT_MARK = "#";
const LIST_SEPARATOR = /[,\s]+/;
const DEGREE_SIGN = "°";
const LON_ALIASES = ["ra", "ra_deg", "radeg", "raj2000", "_raj2000", "ra_icrs", "lon", "l", "glon", "elon"];
const LAT_ALIASES = ["dec", "de", "dec_deg", "dedeg", "dej2000", "_dej2000", "dec_icrs", "de_icrs", "lat", "b", "glat", "elat"];
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

function stripDegreeSign(token: string): string {
  return token.endsWith(DEGREE_SIGN) ? token.slice(0, -DEGREE_SIGN.length).trimEnd() : token;
}

export function parseAngle(
  text: string,
  axis: SkyAxis,
  lonInHours = true,
): { value: number | null; error: string | null } {
  const trimmed = text.trim();
  if (!trimmed) return { value: null, error: `${axis === "lon" ? "RA" : "Dec"} is empty` };
  const decimal = stripDegreeSign(trimmed);
  if (isDecimalToken(decimal)) {
    const value = Number(decimal);
    if (!Number.isFinite(value)) return { value: null, error: `${trimmed} is not a finite number` };
    return checkRange(value, axis);
  }
  const parts = splitSexagesimal(trimmed);
  if (!parts) return { value: null, error: `${trimmed} is neither decimal degrees nor sexagesimal` };
  if (parts.minutes >= 60 || parts.seconds >= 60) {
    return { value: null, error: `${trimmed} has minutes or seconds of 60 or more` };
  }
  const total = parts.whole + parts.minutes / 60 + parts.seconds / 3600;
  const degreeUnit = parts.unitLetter === "d" || parts.unitLetter === DEGREE_SIGN;
  const inHours = axis === "lon" && (parts.unitLetter === "h" || (lonInHours && !degreeUnit));
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

interface LineParse {
  target: Omit<SkyTarget, "raw"> | null;
  error: string | null;
}

function parseListLine(line: string, lonInHours: boolean): LineParse {
  const tokens = line.split(LIST_SEPARATOR).filter((t) => t.length > 0);
  if (tokens.length < 2) return { target: null, error: "needs at least RA and Dec" };
  let leadingNumeric = 0;
  while (leadingNumeric < tokens.length && isDecimalToken(tokens[leadingNumeric])) leadingNumeric++;
  const fieldsPerAngle = leadingNumeric >= 2 * SEXAGESIMAL_FIELDS ? SEXAGESIMAL_FIELDS : 1;
  if (tokens.length < 2 * fieldsPerAngle) return { target: null, error: "needs at least RA and Dec" };
  const lonText = tokens.slice(0, fieldsPerAngle).join(" ");
  const latText = tokens.slice(fieldsPerAngle, 2 * fieldsPerAngle).join(" ");
  const label = tokens.slice(2 * fieldsPerAngle).join(" ");
  const lon = parseAngle(lonText, "lon", lonInHours);
  if (lon.value === null) return { target: null, error: lon.error };
  const lat = parseAngle(latText, "lat");
  if (lat.value === null) return { target: null, error: lat.error };
  return { target: { label, lon: lon.value, lat: lat.value }, error: null };
}

function collectTargets(lines: string[], start: number, parseLine: (line: string) => LineParse): SkyListParse {
  const targets: SkyTarget[] = [];
  const errors: string[] = [];
  let skipped = 0;
  for (let i = start; i < lines.length; i++) {
    const line = stripComment(lines[i]);
    if (!line) continue;
    const parsed = parseLine(line);
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
  if (skipped > 0) errors.push(`only the first ${MAX_TARGETS} targets are kept, ${skipped} more were ignored`);
  return { targets, errors };
}

export function parseSkyList(text: string, lonInHours = true): SkyListParse {
  return collectTargets(text.split(/\r?\n/), 0, (line) => parseListLine(line, lonInHours));
}

function detectDelimiter(line: string): string {
  return CSV_DELIMITERS.find((d) => line.includes(d)) ?? " ";
}

function splitCells(line: string, delimiter: string): string[] {
  const cells = delimiter === " " ? (line.match(WHITESPACE_CELL) ?? []) : line.split(delimiter);
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

function spacedFieldStart(column: number, otherCoordinate: number): number {
  return otherCoordinate < column ? column + SEXAGESIMAL_FIELDS - 1 : column;
}

function spacedAngleText(cells: string[], start: number): string | null {
  const fields = cells.slice(start, start + SEXAGESIMAL_FIELDS);
  return fields.length === SEXAGESIMAL_FIELDS && fields.every(isDecimalToken) ? fields.join(" ") : null;
}

function readsAsSpacedSexagesimal(cells: string[], header: SkyCsvHeader, columns: number, lonInHours: boolean): boolean {
  if (header.delimiter !== " ") return false;
  const surplus = cells.length - columns;
  const spacedMisfits = Math.abs(SPACED_PAIR_EXTRA_CELLS - surplus);
  if (spacedMisfits >= surplus) return false;
  const lonText = spacedAngleText(cells, spacedFieldStart(header.lon, header.lat));
  const latText = spacedAngleText(cells, spacedFieldStart(header.lat, header.lon));
  if (lonText === null || latText === null) return false;
  return parseAngle(lonText, "lon", lonInHours).value !== null && parseAngle(latText, "lat").value !== null;
}

function labelCell(cells: string[], header: SkyCsvHeader, columns: number): string {
  if (header.label < 0) return "";
  if (header.delimiter === " " && header.label === columns - 1) return cells.slice(header.label).join(" ");
  return cells[header.label] ?? "";
}

function parseCsvRow(line: string, header: SkyCsvHeader, columns: number, lonInHours: boolean): LineParse {
  const cells = splitCells(line, header.delimiter);
  if (readsAsSpacedSexagesimal(cells, header, columns, lonInHours)) {
    if (header.lon === 0 && header.lat === 1) return parseListLine(cells.join(" "), lonInHours);
    return {
      target: null,
      error: `has ${cells.length} space-separated fields but the header has ${columns}; separate the columns with commas or tabs, or write sexagesimal with colons`,
    };
  }
  const lon = parseAngle(cells[header.lon] ?? "", "lon", lonInHours);
  if (lon.value === null) return { target: null, error: lon.error };
  const lat = parseAngle(cells[header.lat] ?? "", "lat");
  if (lat.value === null) return { target: null, error: lat.error };
  return { target: { label: labelCell(cells, header, columns), lon: lon.value, lat: lat.value }, error: null };
}

export function parseSkyCsv(text: string, lonInHours = true): SkyListParse {
  const lines = text.split(/\r?\n/);
  const headerIndex = lines.findIndex((l) => stripComment(l).length > 0);
  if (headerIndex < 0) return { targets: [], errors: [] };
  const headerLine = stripComment(lines[headerIndex]);
  const header = detectSkyCsvHeader(headerLine);
  if (!header) return parseSkyList(text, lonInHours);
  const columns = splitCells(headerLine, header.delimiter).length;
  return collectTargets(lines, headerIndex + 1, (line) => parseCsvRow(line, header, columns, lonInHours));
}

export function targetProjectionPath(fileKey: string | null, displayedPath: string | null): string | null {
  return fileKey === null ? null : (displayedPath ?? fileKey);
}
