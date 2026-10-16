export type QueryKind = "name" | "coordinates" | "programme" | "dataset";

export interface Interpretation {
  kind: QueryKind | "empty";
  text: string;
  raDeg: number | null;
  decDeg: number | null;
  program: number | null;
  datasetId: string | null;
  name: string | null;
  error: string | null;
}

export const QUERY_KINDS: readonly QueryKind[] = ["name", "coordinates", "programme", "dataset"];

const SAFE_ID = /^[A-Za-z0-9._-]{1,200}$/;
const NUMBER = "[+-]?(?:\\d+(?:\\.\\d*)?|\\.\\d+)";
const DECIMAL_PAIR = new RegExp(`^(${NUMBER})\\s*[,\\s]\\s*(${NUMBER})$`);
const SEXA_COLON = /^(\d{1,2})[:\s](\d{1,2})[:\s](\d{1,2}(?:\.\d*)?)\s*[,\s]\s*([+-]?)(\d{1,2})[:\s](\d{1,2})[:\s](\d{1,2}(?:\.\d*)?)$/;
const SEXA_LETTERS = /^(\d{1,2})h\s*(\d{1,2})m\s*(\d{1,2}(?:\.\d*)?)s?\s*[,\s]\s*([+-]?)(\d{1,2})[d°]\s*(\d{1,2})['m]\s*(\d{1,2}(?:\.\d*)?)(?:"|''|s)?$/i;
const PROGRAMME = /^\d{4,5}$/;
const JWST_DATASET = /^jw\d{5}[a-z0-9._-]*$/i;
const HST_ROOTNAME = /^[a-z][a-z0-9]{8}$/i;

const COORD_HINT = "Coordinates: type RA and Dec, for example 274.688 -13.792";
const PROGRAMME_HINT = "Programme: type a number, for example 2739";
const DATASET_HINT = "Dataset id: use letters, digits, dot, underscore or hyphen";

function blank(text: string): Interpretation {
  return { kind: "empty", text, raDeg: null, decDeg: null, program: null, datasetId: null, name: null, error: null };
}

function withKind(text: string, kind: QueryKind, patch: Partial<Interpretation>): Interpretation {
  return { ...blank(text), kind, ...patch };
}

interface Parsed {
  raDeg: number;
  decDeg: number;
  error: string | null;
}

function rangeError(raDeg: number, decDeg: number): string | null {
  if (!(raDeg >= 0 && raDeg < 360)) return "RA must be at least 0 and below 360 degrees";
  if (!(decDeg >= -90 && decDeg <= 90)) return "Dec must be between -90 and +90 degrees";
  return null;
}

function sexagesimal(m: RegExpMatchArray): Parsed {
  const [h, rm, rs, sign, d, dm, ds] = [Number(m[1]), Number(m[2]), Number(m[3]), m[4], Number(m[5]), Number(m[6]), Number(m[7])];
  if (h >= 24) return { raDeg: Number.NaN, decDeg: Number.NaN, error: "RA hours must be below 24" };
  if (rm >= 60 || rs >= 60 || dm >= 60 || ds >= 60) return { raDeg: Number.NaN, decDeg: Number.NaN, error: "Minutes and seconds must be below 60" };
  const raDeg = (h + rm / 60 + rs / 3600) * 15;
  const magnitude = d + dm / 60 + ds / 3600;
  const decDeg = sign === "-" ? -magnitude : magnitude;
  return { raDeg, decDeg, error: rangeError(raDeg, decDeg) };
}

function parseCoordinates(text: string): Parsed | null {
  const dec = DECIMAL_PAIR.exec(text);
  if (dec) {
    const raDeg = Number(dec[1]);
    const decDeg = Number(dec[2]);
    return { raDeg, decDeg, error: rangeError(raDeg, decDeg) };
  }
  const colon = SEXA_COLON.exec(text);
  if (colon) return sexagesimal(colon);
  const letters = SEXA_LETTERS.exec(text);
  if (letters) return sexagesimal(letters);
  return null;
}

function digitCount(text: string): number {
  return text.replace(/\D/g, "").length;
}

function datasetIdOf(text: string): string | null {
  if (JWST_DATASET.test(text) && SAFE_ID.test(text)) return text.toLowerCase();
  if (HST_ROOTNAME.test(text) && digitCount(text) >= 4) return text.toUpperCase();
  return null;
}

function isSafeDatasetId(text: string): boolean {
  return SAFE_ID.test(text) && !text.startsWith(".") && !text.includes("..");
}

function coordinates(text: string, parsed: Parsed | null): Interpretation {
  if (!parsed) return withKind(text, "coordinates", { error: COORD_HINT });
  if (parsed.error) return withKind(text, "coordinates", { error: parsed.error });
  return withKind(text, "coordinates", { raDeg: parsed.raDeg, decDeg: parsed.decDeg });
}

function forced(text: string, kind: QueryKind): Interpretation {
  switch (kind) {
    case "name":
      return withKind(text, "name", { name: text });
    case "coordinates":
      return coordinates(text, parseCoordinates(text));
    case "programme":
      return /^\d{1,6}$/.test(text) ? withKind(text, "programme", { program: Number(text) }) : withKind(text, "programme", { error: PROGRAMME_HINT });
    case "dataset":
      return isSafeDatasetId(text) ? withKind(text, "dataset", { datasetId: datasetIdOf(text) ?? text }) : withKind(text, "dataset", { error: DATASET_HINT });
  }
}

export function interpretQuery(raw: string, override: QueryKind | null = null): Interpretation {
  const text = raw.trim();
  if (!text) return blank(text);
  if (override) return forced(text, override);
  const parsed = parseCoordinates(text);
  if (parsed) return coordinates(text, parsed);
  if (PROGRAMME.test(text)) return withKind(text, "programme", { program: Number(text) });
  const datasetId = datasetIdOf(text);
  if (datasetId) return withKind(text, "dataset", { datasetId });
  return withKind(text, "name", { name: text });
}
