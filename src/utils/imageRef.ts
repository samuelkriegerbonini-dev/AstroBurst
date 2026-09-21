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
const MAX_ARRAY_KEY_LEN = 128;
const ARRAY_KEY_RE = /^[A-Za-z0-9_.-]+$/;
const HDU_INDEX_RE = /^[0-9]+$/;

function parseFragment(fragment: string): PlaneSelector | null {
  if (fragment.startsWith(FRAGMENT_HDU)) {
    const n = fragment.slice(FRAGMENT_HDU.length);
    if (!HDU_INDEX_RE.test(n)) return null;
    const index = Number(n);
    return Number.isSafeInteger(index) ? { kind: "hdu", index } : null;
  }
  if (fragment.startsWith(FRAGMENT_ARRAY)) {
    const key = fragment.slice(FRAGMENT_ARRAY.length);
    if (key.length === 0 || key.length > MAX_ARRAY_KEY_LEN || !ARRAY_KEY_RE.test(key)) return null;
    return { kind: "array", key };
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
      return `${ref.path}#${FRAGMENT_ARRAY}${ref.plane.key}`;
  }
}

export function hduRef(path: string, index: number): string {
  return formatImageRef({ path, plane: { kind: "hdu", index } });
}

export function arrayRef(path: string, key: string): string {
  return formatImageRef({ path, plane: { kind: "array", key } });
}

const SOURCE_EXTENSION = /\.(fits?|fts)(\.(fz|gz))?$|\.(asdf|zip|gz)$/i;

export function exportStem(pathOrRef: string, fallback = "output"): string {
  const stem = parseImageRef(pathOrRef)
    .path
    .split("#")[0]
    .split(/[/\\]/)
    .pop()
    ?.replace(SOURCE_EXTENSION, "");
  return stem && stem.length > 0 ? stem : fallback;
}

export function planeLabel(ref: ImageRef, extname?: string | null): string | null {
  switch (ref.plane.kind) {
    case "auto":
      return null;
    case "hdu":
      return extname ? `HDU ${ref.plane.index} · ${extname}` : `HDU ${ref.plane.index}`;
    case "array":
      return `array ${ref.plane.key}`;
  }
}
