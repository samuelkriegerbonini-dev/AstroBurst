export type MonoPixelsAction = "load" | "reload" | "clear" | "keep";

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
