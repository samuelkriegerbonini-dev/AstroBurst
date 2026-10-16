import type { MastLevelClass, MastRow } from "../../../shared/types/mast";

const INSTRUMENT_NAMES: Record<string, string> = {
  NIRCAM: "NIRCam",
  NIRSPEC: "NIRSpec",
};

const WAVELENGTH_FILTER = /^F(\d{3,4})[A-Z]{1,2}$/i;
const RANGE_MIN_FILTERS = 3;

export function instrumentText(instrument: string | null): string {
  if (!instrument) return "";
  const trimmed = instrument.trim();
  return INSTRUMENT_NAMES[trimmed.toUpperCase()] ?? trimmed;
}

export function levelBadge(levelClass: MastLevelClass): string | null {
  if (levelClass === "combined") return "L3";
  if (levelClass === "exposure") return "L1-2";
  return null;
}

export function dateText(value: string | null): string {
  if (!value) return "";
  const m = /^\d{4}-\d{2}-\d{2}/.exec(value.trim());
  return m ? m[0] : value.trim();
}

export function exposureText(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds) || seconds < 0) return "";
  const minutes = Math.round(seconds / 60);
  if (minutes >= 60) return `${(seconds / 3600).toFixed(1)} h`;
  if (Math.round(seconds) >= 60) return `${minutes} min`;
  return `${Math.round(seconds)} s`;
}

function wavelengthOf(token: string): number | null {
  const m = WAVELENGTH_FILTER.exec(token.trim());
  return m ? Number(m[1]) : null;
}

function componentsOf(filter: string): string[] {
  return filter.split(/[-/+]/).map((t) => t.trim()).filter(Boolean);
}

function entryKey(filter: string): number | null {
  const parts = componentsOf(filter);
  if (parts.length === 0) return null;
  const values = parts.map(wavelengthOf);
  if (values.some((v) => v === null)) return null;
  return values[values.length - 1];
}

export function filterRangeText(filters: readonly string[]): string {
  const clean = filters.map((f) => f.trim()).filter(Boolean);
  if (clean.length === 0) return "";
  const keyed = clean.map((text, index) => ({ text, index, key: entryKey(text) }));
  const measured = keyed.filter((k): k is { text: string; index: number; key: number } => k.key !== null);
  if (measured.length === keyed.length && clean.length >= RANGE_MIN_FILTERS) {
    const tokens = clean.flatMap(componentsOf).map((t) => ({ t, v: wavelengthOf(t) as number }));
    const low = tokens.reduce((a, b) => (b.v < a.v ? b : a));
    const high = tokens.reduce((a, b) => (b.v > a.v ? b : a));
    return low.t === high.t ? low.t : `${low.t}..${high.t}`;
  }
  const sorted = [...measured].sort((a, b) => a.key - b.key || a.index - b.index);
  const others = keyed.filter((k) => k.key === null);
  return [...sorted, ...others].map((k) => k.text).join(", ");
}

export function isPublic(row: Pick<MastRow, "access">): boolean {
  return row.access === null || row.access.trim().toUpperCase() === "PUBLIC";
}

export function exclusiveText(row: Pick<MastRow, "access" | "releaseDate">): string | null {
  if (isPublic(row)) return null;
  const until = dateText(row.releaseDate);
  return until ? `Exclusive until ${until}` : "Exclusive access";
}

function filtersPart(filters: readonly string[]): string | null {
  if (filters.length === 0) return null;
  return filters.length === 1 ? `filter ${filters[0]}` : `${filters.length} filters`;
}

export function cardAccessibleName(row: MastRow, missionLabel: string): string {
  const parts: string[] = [row.target?.trim() || row.datasetId];
  parts.push([missionLabel, instrumentText(row.instrument)].filter(Boolean).join(" "));
  if (row.level) parts.push(`level ${row.level}`);
  const filters = filtersPart(row.filters);
  if (filters) parts.push(filters);
  const date = dateText(row.dateObs);
  if (date) parts.push(date);
  if (row.access !== null) {
    const exclusive = exclusiveText(row);
    parts.push(exclusive ? exclusive.charAt(0).toLowerCase() + exclusive.slice(1) : "public");
  }
  return parts.join(", ");
}
