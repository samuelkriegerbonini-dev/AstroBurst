const VALID_EXTENSIONS = [".fits", ".fit", ".fts", ".asdf"];

export function isValidFitsFile(nameOrPath: string): boolean {
  const lower = nameOrPath.toLowerCase();
  return VALID_EXTENSIONS.some((ext) => lower.endsWith(ext));
}
