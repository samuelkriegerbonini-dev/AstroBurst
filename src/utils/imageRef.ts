export type PlaneSelector =
  | { kind: "auto" }
  | { kind: "hdu"; index: number }
  | { kind: "array"; key: string };

export interface ImageRef {
  path: string;
  plane: PlaneSelector;
}

const FRAGMENT_HDU = "hdu=";
const FRAGMENT_ARRAY = "array=";
const HDU_INDEX_RE = /^[0-9]+$/;
const UPPER_HEX_PAIR_RE = /^[0-9A-F]{2}$/;

function isArrayKeyByte(b: number): boolean {
  return (
    (b >= 0x30 && b <= 0x39) ||
    (b >= 0x41 && b <= 0x5a) ||
    (b >= 0x61 && b <= 0x7a) ||
    b === 0x5f ||
    b === 0x2e ||
    b === 0x2d
  );
}

function encodeArrayKey(key: string): string {
  let out = "";
  for (const b of new TextEncoder().encode(key)) {
    out += isArrayKeyByte(b) ? String.fromCharCode(b) : `%${b.toString(16).toUpperCase().padStart(2, "0")}`;
  }
  return out;
}

function decodeArrayKey(encoded: string): string | null {
  const bytes: number[] = [];
  let i = 0;
  while (i < encoded.length) {
    const c = encoded.charCodeAt(i);
    if (isArrayKeyByte(c)) {
      bytes.push(c);
      i += 1;
      continue;
    }
    if (c !== 0x25) return null;
    const hex = encoded.slice(i + 1, i + 3);
    if (!UPPER_HEX_PAIR_RE.test(hex)) return null;
    const value = parseInt(hex, 16);
    if (isArrayKeyByte(value)) return null;
    bytes.push(value);
    i += 3;
  }
  try {
    const key = new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(bytes));
    return key.length > 0 ? key : null;
  } catch {
    return null;
  }
}

function parseFragment(fragment: string): PlaneSelector | null {
  if (fragment.startsWith(FRAGMENT_HDU)) {
    const n = fragment.slice(FRAGMENT_HDU.length);
    if (!HDU_INDEX_RE.test(n)) return null;
    const index = Number(n);
    return Number.isSafeInteger(index) ? { kind: "hdu", index } : null;
  }
  if (fragment.startsWith(FRAGMENT_ARRAY)) {
    const key = decodeArrayKey(fragment.slice(FRAGMENT_ARRAY.length));
    return key === null ? null : { kind: "array", key };
  }
  return null;
}

export function parseImageRef(key: string): ImageRef {
  if (key.startsWith("__")) return { path: key, plane: { kind: "auto" } };
  const pos = key.lastIndexOf("#");
  if (pos > 0) {
    const plane = parseFragment(key.slice(pos + 1));
    if (plane) return { path: key.slice(0, pos), plane };
  }
  return { path: key, plane: { kind: "auto" } };
}

export function formatImageRef(ref: ImageRef): string {
  switch (ref.plane.kind) {
    case "auto":
      return ref.path;
    case "hdu":
      return `${ref.path}#${FRAGMENT_HDU}${ref.plane.index}`;
    case "array":
      return `${ref.path}#${FRAGMENT_ARRAY}${encodeArrayKey(ref.plane.key)}`;
  }
}

export function hduRef(path: string, index: number): string {
  return formatImageRef({ path, plane: { kind: "hdu", index } });
}

export function arrayRef(path: string, key: string): string {
  return formatImageRef({ path, plane: { kind: "array", key } });
}

export interface BandPlaneRef {
  file: string;
  hdu: number;
  plane: number;
}

const FRAGMENT_PLANE = "&plane=";
const BAND_PLANE_FRAGMENT_RE = /^hdu=(0|[1-9][0-9]*)&plane=(0|[1-9][0-9]*)$/;

export function parseBandPlaneRef(key: string): BandPlaneRef | null {
  if (key.startsWith("__")) return null;
  const pos = key.lastIndexOf("#");
  if (pos <= 0) return null;
  const match = BAND_PLANE_FRAGMENT_RE.exec(key.slice(pos + 1));
  if (!match) return null;
  const hdu = Number(match[1]);
  const plane = Number(match[2]);
  return Number.isSafeInteger(hdu) && Number.isSafeInteger(plane) ? { file: key.slice(0, pos), hdu, plane } : null;
}

export function bandPlaneRef(file: string, hdu: number, plane: number): string {
  return `${file}#${FRAGMENT_HDU}${hdu}${FRAGMENT_PLANE}${plane}`;
}

export function sourceFile(pathOrRef: string): string {
  const path = parseImageRef(pathOrRef).path;
  return parseBandPlaneRef(path)?.file ?? path;
}

const SOURCE_EXTENSION = /\.(fits?|fts)(\.(fz|gz|bz2))?$|\.(asdf|zip|gz|bz2)$/i;

export function exportStem(pathOrRef: string, fallback = "output"): string {
  const path = parseImageRef(pathOrRef).path;
  const band = parseBandPlaneRef(path);
  if (band) return `${exportStem(band.file, fallback)}_hdu${band.hdu}_plane${band.plane}`;
  const stem = path
    .split(/[/\\]/)
    .pop()
    ?.split("#")[0]
    .replace(SOURCE_EXTENSION, "");
  return stem && stem.length > 0 ? stem : fallback;
}

export function planeLabel(ref: ImageRef, extname?: string | null): string | null {
  switch (ref.plane.kind) {
    case "auto": {
      const band = parseBandPlaneRef(ref.path);
      if (!band) return null;
      return extname ? `HDU ${band.hdu} · ${extname} · plane ${band.plane}` : `HDU ${band.hdu} · plane ${band.plane}`;
    }
    case "hdu":
      return extname ? `HDU ${ref.plane.index} · ${extname}` : `HDU ${ref.plane.index}`;
    case "array":
      return `array ${ref.plane.key}`;
  }
}
