export function isValidFitsFile(name: string): boolean {
  return /\.(fits?|fts)$/i.test(name);
}
