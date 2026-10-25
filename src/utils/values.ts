export function isFiniteNumber(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function finiteOrNull(value: number | null | undefined): number | null {
  return isFiniteNumber(value) ? value : null;
}

export function fileBaseName(filePath: string): string {
  return filePath.split(/[\\/]/).pop() ?? filePath;
}
