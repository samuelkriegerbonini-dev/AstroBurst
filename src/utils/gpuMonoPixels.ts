export type MonoPixelsAction = "load" | "reload" | "clear" | "keep";

export interface HeldPixelsSource {
  key: string;
  dims: [number, number] | null;
}

export function heldPixelsSourceKey(path: string, version: number): string {
  return `${path}|${version}`;
}

export function heldPixelsDrawable(
  held: HeldPixelsSource | null,
  shownKey: string | null,
  shownDims: [number, number] | null,
): boolean {
  if (held === null || shownKey === null) return false;
  if (held.key === shownKey) return true;
  return held.dims !== null && shownDims !== null && held.dims[0] === shownDims[0] && held.dims[1] === shownDims[1];
}

export function monoPixelsAction(
  pngOnly: boolean,
  loadKey: string | null,
  loadedKey: string | null,
  clearOnSourceChange: boolean,
): MonoPixelsAction {
  if (pngOnly) return loadedKey === null ? "keep" : "clear";
  if (!loadKey || loadedKey === loadKey) return "keep";
  return clearOnSourceChange && loadedKey !== null ? "reload" : "load";
}
