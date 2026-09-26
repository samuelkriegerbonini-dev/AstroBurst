import { filterCodeAndWavelengthNm } from "./filterWavelengths";
import type { FrequencyBin } from "./wizard";

export interface ChannelSource {
  name?: string;
  path?: string;
  result?: { header?: Record<string, string> | null } | null;
}

export const FILTER_TO_BIN: Record<string, string> = {
  "Halpha": "ha", "Ha": "ha", "H_alpha": "ha", "H-alpha": "ha",
  "OIII": "oiii", "O3": "oiii", "[OIII]": "oiii",
  "SII": "sii", "S2": "sii", "[SII]": "sii",
  "NII": "nii",
  "Red": "r", "R": "r",
  "Green": "g", "G": "g", "V": "g",
  "Blue": "b", "B": "b",
  "Luminance": "l", "Lum": "l", "Clear": "l", "CLR": "l", "L": "l",
};

export const FILTER_PATTERNS: [string, RegExp][] = [
  ["ha", /(?:(?:^|[^A-Za-z])H[-_]?(?:alpha|a)(?![A-Za-z])|\b656\s*(?:nm)?|H_?α|F656N)/i],
  ["oiii", /(?:O\s*III|\[?OIII\]?|50[012](?:\.\d+)?\s*nm|\b50[12]\b|\b5007\b|(?:^|[^A-Za-z0-9])O3(?![A-Za-z0-9])|F50[123]N)/i],
  ["sii", /(?:S\s*II|\[?SII\]?|\b673\s*(?:nm)?|S2\b|F673N)/i],
  ["r", /\b(?:Red|R['_-]?band|Sloan[_-]?r)\b/i],
  ["g", /\b(?:Green|G['_-]?band|Sloan[_-]?g|V[_-]?band)\b/i],
  ["b", /\b(?:Blue|B['_-]?band|Sloan[_-]?b)\b/i],
  ["l", /\b(?:Lum(?:inance)?|L['_-]?band|Clear|CLR)\b/i],
];

export const FILENAME_PATTERNS: [string, RegExp][] = [
  ["ha", /(?:^|[_\-.\s])(?:HA|HALPHA|H_?ALPHA|656(?:NM)?)(?=[_\-.\s]|$)/i],
  ["oiii", /(?:^|[_\-.\s])(?:OIII|O3|50[12](?:NM)?)(?=[_\-.\s]|$)/i],
  ["sii", /(?:^|[_\-.\s])(?:SII|S2|673(?:NM)?)(?=[_\-.\s]|$)/i],
  ["r", /(?:^|[_\-.\s])(?:RED|R)(?=[_\-.\s]|$)/i],
  ["g", /(?:^|[_\-.\s])(?:GREEN|G)(?=[_\-.\s]|$)/i],
  ["b", /(?:^|[_\-.\s])(?:BLUE|B)(?=[_\-.\s]|$)/i],
  ["l", /(?:^|[_\-.\s])(?:LUMINANCE|LUM|L)(?=[_\-.\s]|$)/i],
];

const BIN_TO_SLOT: Record<string, "L" | "R" | "G" | "B"> = {
  l: "L", r: "R", g: "G", b: "B", ha: "R", oiii: "G", sii: "B",
};

const FILTER_HEADER_KEYS = ["FILTER", "FILTER1", "FILTER2", "FILTNAM1", "FILTNAM2", "PUPIL"];

const WHEEL_POSITION = /^\d{1,2}$/;

const CLEAR_TOKEN = /(?:^|[_\-.\s])CLEAR(?=[_\-.\s]|$)/i;

const CLEAR_PUPIL_PAIR = /(?:^|[_\-.\s])(?:CLEAR-([A-Za-z0-9]+)|([A-Za-z0-9]+)-CLEAR)(?=[_\-.\s]|$)/i;

export const REUSABLE_BIN_IDS = new Set(["l"]);

export const COLOR_BIN_IDS = ["r", "g", "b"] as const;

export function isClearToken(value: string): boolean {
  return value.trim().toUpperCase() === "CLEAR";
}

export function clearIsPupilSlot(name: string): boolean {
  const match = CLEAR_PUPIL_PAIR.exec(name);
  if (!match) return false;
  return filterCodeAndWavelengthNm(match[1] ?? match[2]) !== null;
}

export function headerFilterValues(file: ChannelSource): string[] {
  const header = file.result?.header;
  if (!header) return [];
  const values: string[] = [];
  for (const key of FILTER_HEADER_KEYS) {
    const raw = header[key];
    if (raw == null) continue;
    const value = String(raw).trim();
    if (value && !WHEEL_POSITION.test(value)) values.push(value);
  }
  return values;
}

export function displayFilterValue(file: ChannelSource): string | null {
  const values = headerFilterValues(file);
  if (values.length === 0) return null;
  const resolvable = values.find((v) => filterCodeAndWavelengthNm(v) !== null);
  return resolvable ?? values[0];
}

export function resolveFilterFromName(name: string): { code: string; nm: number } | null {
  return filterCodeAndWavelengthNm(name.replace(/\.(fits?|fts|asdf)$/i, ""));
}

export function resolveFileFilter(file: ChannelSource): { code: string; nm: number } | null {
  for (const value of headerFilterValues(file)) {
    const info = filterCodeAndWavelengthNm(value);
    if (info) return info;
  }
  return null;
}

export function detectChannelByHeader(file: ChannelSource): string | null {
  const values = headerFilterValues(file);
  if (values.length === 0) return null;
  const hasResolvableFilter = values.some((v) => filterCodeAndWavelengthNm(v) !== null);
  for (const value of values) {
    if (hasResolvableFilter && isClearToken(value)) continue;
    const direct = FILTER_TO_BIN[value];
    if (direct) return direct;
    for (const [binId, pattern] of FILTER_PATTERNS) {
      if (pattern.test(value)) return binId;
    }
  }
  return null;
}

export function detectChannelByFilename(file: ChannelSource): string | null {
  const name = file.name || file.path || "";
  for (const [binId, pattern] of FILENAME_PATTERNS) {
    if (pattern.test(name)) return binId;
  }
  if (CLEAR_TOKEN.test(name) && !clearIsPupilSlot(name)) return "l";
  return null;
}

export function detectChannel(file: ChannelSource): string | null {
  return detectChannelByHeader(file) ?? detectChannelByFilename(file);
}

export function filenameChannelSlot(name: string): "L" | "R" | "G" | "B" | null {
  const bin = detectChannelByFilename({ name });
  return bin ? BIN_TO_SLOT[bin] ?? null : null;
}

export interface WavelengthGroup<T> {
  code: string;
  nm: number;
  items: T[];
}

export function groupByWavelength<T>(
  entries: { item: T; code: string; nm: number }[],
): WavelengthGroup<T>[] {
  const groups = new Map<number, WavelengthGroup<T>>();
  for (const entry of entries) {
    const existing = groups.get(entry.nm);
    if (existing) {
      existing.items.push(entry.item);
    } else {
      groups.set(entry.nm, { code: entry.code, nm: entry.nm, items: [entry.item] });
    }
  }
  return Array.from(groups.values()).sort((a, b) => a.nm - b.nm);
}

export function splitSpectralThirds<T>(groups: WavelengthGroup<T>[]): { r: T[]; g: T[]; b: T[] } {
  const sorted = [...groups].sort((a, b) => a.nm - b.nm);
  if (sorted.length < 2) return { r: [], g: [], b: [] };
  const flatten = (from: number, to: number) => sorted.slice(from, to).flatMap((group) => group.items);
  if (sorted.length === 2) return { r: flatten(1, 2), g: [], b: flatten(0, 1) };
  const blueEnd = Math.floor(sorted.length / 3);
  const greenEnd = Math.floor((sorted.length * 2) / 3);
  return { r: flatten(greenEnd, sorted.length), g: flatten(blueEnd, greenEnd), b: flatten(0, blueEnd) };
}

export function shortName(path: string): string {
  return path.split(/[/\\]/).pop()?.replace(/\.(fits?|asdf)$/i, "") ?? path;
}

export interface UnmappedFile {
  name: string;
  filter: string | null;
}

export interface WavelengthMapResult {
  bins: FrequencyBin[];
  headerMapped: number;
  wavelengthMapped: number;
  filenameMapped: number;
  spectralMapped: number;
  dynamicBinCount: number;
  unmapped: UnmappedFile[];
}

export function mapFilesByWavelength(
  bins: FrequencyBin[],
  files: (ChannelSource & { path: string })[],
  assigned: Set<string>,
): WavelengthMapResult {
  const next = bins.map((b) => ({ ...b, files: [...b.files] }));
  const dynamicBins: FrequencyBin[] = [];
  const entries: { item: string; code: string; nm: number }[] = [];
  const unmapped: UnmappedFile[] = [];
  let headerMapped = 0;
  let wavelengthMapped = 0;
  let filenameMapped = 0;
  let spectralMapped = 0;

  const addToBin = (binId: string, path: string): boolean => {
    const bin = next.find((b) => b.id === binId);
    if (!bin || bin.files.includes(path)) return false;
    bin.files.push(path);
    return true;
  };

  for (const file of files) {
    if (assigned.has(file.path)) continue;

    const headerBin = detectChannelByHeader(file);
    if (headerBin && addToBin(headerBin, file.path)) {
      headerMapped++;
      continue;
    }

    let info = resolveFileFilter(file);
    if (!info) {
      const nameBin = detectChannelByFilename(file);
      if (nameBin && addToBin(nameBin, file.path)) {
        filenameMapped++;
        continue;
      }
      info = resolveFilterFromName(file.name || file.path);
    }
    if (!info) {
      unmapped.push({
        name: file.name || shortName(file.path),
        filter: headerFilterValues(file)[0] ?? null,
      });
      continue;
    }

    entries.push({ item: file.path, code: info.code, nm: info.nm });
  }

  const groups = groupByWavelength(entries);
  const spectral = splitSpectralThirds(groups);
  const spectralTargets: [string, string[]][] = [["r", spectral.r], ["g", spectral.g], ["b", spectral.b]];
  const spectrallyPlaced = new Set<string>();
  for (const [binId, paths] of spectralTargets) {
    for (const path of paths) {
      if (addToBin(binId, path)) {
        spectrallyPlaced.add(path);
        spectralMapped++;
      }
    }
  }

  for (const group of groups) {
    const remaining = group.items.filter((path) => !spectrallyPlaced.has(path));
    if (remaining.length === 0) continue;
    const binId = `wl${group.nm}`;
    let bin = next.find((b) => b.id === binId) ?? dynamicBins.find((b) => b.id === binId);
    if (!bin) {
      const hue = Math.round((group.nm * 0.18) % 360);
      bin = {
        id: binId,
        label: `${group.code} (${group.nm}nm)`,
        shortLabel: group.code.slice(0, 5),
        wavelength: group.nm,
        color: `hsl(${hue}, 70%, 55%)`,
        files: [],
      };
      dynamicBins.push(bin);
    }
    for (const path of remaining) {
      if (bin.files.includes(path)) continue;
      bin.files.push(path);
      wavelengthMapped++;
    }
  }

  return {
    bins: dynamicBins.length > 0 ? [...next, ...dynamicBins] : next,
    headerMapped,
    wavelengthMapped,
    filenameMapped,
    spectralMapped,
    dynamicBinCount: dynamicBins.length,
    unmapped,
  };
}

export function assignedPaths(bins: FrequencyBin[]): Set<string> {
  const paths = new Set<string>();
  for (const bin of bins) for (const file of bin.files) paths.add(file);
  return paths;
}

export function exclusivelyAssignedPaths(bins: FrequencyBin[]): Set<string> {
  const paths = new Set<string>();
  for (const bin of bins) {
    if (REUSABLE_BIN_IDS.has(bin.id)) continue;
    for (const file of bin.files) paths.add(file);
  }
  return paths;
}

export function colorBinsEmpty(bins: FrequencyBin[]): boolean {
  return COLOR_BIN_IDS.every((id) => (bins.find((b) => b.id === id)?.files.length ?? 0) === 0);
}

export interface AutoMapPalette {
  palette_name?: string | null;
  is_complete?: boolean;
  r_file?: { file_path: string } | null;
  g_file?: { file_path: string } | null;
  b_file?: { file_path: string } | null;
}

export interface AutoMapDetection {
  path: string;
  filter: string | null;
  hubble_channel?: string | null;
}

export interface AutoMapInput {
  bins: FrequencyBin[];
  files: (ChannelSource & { path: string })[];
  assigned: Set<string>;
  palette?: AutoMapPalette | null;
  detections?: AutoMapDetection[];
}

export interface AutoMapResult {
  bins: FrequencyBin[];
  sources: string[];
  unmapped: UnmappedFile[];
  mappedCount: number;
}

export type RgbChannelLetter = "R" | "G" | "B";

export function assignableChannel(hubbleChannel: string | null | undefined): RgbChannelLetter | null {
  return hubbleChannel === "R" || hubbleChannel === "G" || hubbleChannel === "B" ? hubbleChannel : null;
}

export function detectionTargetBin(detection: AutoMapDetection): string | null {
  if (!detection.filter) return null;
  const filterValue = String(detection.filter);
  const direct = FILTER_TO_BIN[filterValue];
  if (direct) return direct;
  for (const [binId, pattern] of FILTER_PATTERNS) {
    if (pattern.test(filterValue)) return binId;
  }
  const hubbleChannel = detection.hubble_channel ? String(detection.hubble_channel).toLowerCase() : null;
  if (hubbleChannel === "r" || hubbleChannel === "red") return "r";
  if (hubbleChannel === "g" || hubbleChannel === "green") return "g";
  if (hubbleChannel === "b" || hubbleChannel === "blue") return "b";
  return null;
}

export function runAutoMap({ bins, files, assigned, palette, detections }: AutoMapInput): AutoMapResult {
  const next = bins.map((b) => ({ ...b, files: [...b.files] }));
  const mappedNow = new Set(assigned);
  const sources: string[] = [];
  let mappedCount = 0;

  const addToBin = (binId: string, path: string): boolean => {
    const bin = next.find((b) => b.id === binId);
    if (!bin || bin.files.includes(path)) return false;
    bin.files.push(path);
    mappedNow.add(path);
    return true;
  };

  const stageIsSufficient = (mapped: number) =>
    mapped > 0 && (!colorBinsEmpty(next) || files.every((f) => mappedNow.has(f.path)));

  if (palette?.is_complete) {
    const paletteMap: Record<string, string> = {};
    if (palette.r_file?.file_path) paletteMap[palette.r_file.file_path] = "r";
    if (palette.g_file?.file_path) paletteMap[palette.g_file.file_path] = "g";
    if (palette.b_file?.file_path) paletteMap[palette.b_file.file_path] = "b";

    let mapped = 0;
    for (const file of files) {
      if (mappedNow.has(file.path)) continue;
      const target = paletteMap[file.path];
      if (target && addToBin(target, file.path)) mapped++;
    }
    if (mapped > 0) sources.push(palette.palette_name ?? "Palette");
    mappedCount += mapped;
    if (stageIsSufficient(mapped)) return { bins: next, sources, unmapped: [], mappedCount };
  }

  if (detections && detections.length > 0) {
    let mapped = 0;
    for (const detection of detections) {
      if (mappedNow.has(detection.path)) continue;
      const targetBin = detectionTargetBin(detection);
      if (targetBin && addToBin(targetBin, detection.path)) mapped++;
    }
    if (mapped > 0) sources.push("FITS Headers (Rust)");
    mappedCount += mapped;
    if (stageIsSufficient(mapped)) return { bins: next, sources, unmapped: [], mappedCount };
  }

  const result = mapFilesByWavelength(next, files, mappedNow);
  if (result.headerMapped > 0) sources.push("FITS Headers");
  if (result.wavelengthMapped > 0) sources.push("Wavelength");
  if (result.spectralMapped > 0) sources.push("Spectral RGB");
  if (result.filenameMapped > 0) sources.push("Filename");
  mappedCount += result.headerMapped + result.wavelengthMapped + result.filenameMapped + result.spectralMapped;

  return { bins: result.bins, sources, unmapped: result.unmapped, mappedCount };
}
