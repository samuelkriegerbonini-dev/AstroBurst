import { normalizeOutputPath } from "./processingChain";

export interface CosmeticCopy {
  original: string;
  corrected: string;
}

interface CosmeticOutput {
  path: string;
  fits_path?: string | null;
  error?: string | null;
}

export interface CorrectedCopySwap {
  paths: string[];
  count: number;
}

export function cosmeticCopiesOf(items: readonly CosmeticOutput[]): CosmeticCopy[] {
  const copies: CosmeticCopy[] = [];
  for (const item of items) {
    if (item.error || !item.fits_path) continue;
    copies.push({ original: item.path, corrected: item.fits_path });
  }
  return copies;
}

export function withCosmeticCopies(current: CosmeticCopy[], incoming: readonly CosmeticCopy[]): CosmeticCopy[] {
  if (incoming.length === 0) return current;
  const next = [...current];
  for (const copy of incoming) {
    const key = normalizeOutputPath(copy.corrected);
    const at = next.findIndex((c) => normalizeOutputPath(c.corrected) === key);
    if (at >= 0) next[at] = copy;
    else next.push(copy);
  }
  return next;
}

export function listedCosmeticCopies(copies: readonly CosmeticCopy[], loadedPaths: readonly string[]): CosmeticCopy[] {
  const loaded = new Set(loadedPaths.map(normalizeOutputPath));
  return copies.filter((c) => !loaded.has(normalizeOutputPath(c.corrected)));
}

export function correctedCopySwap(
  selected: readonly string[],
  copies: readonly CosmeticCopy[],
  rejected: readonly string[],
  loadedPaths: readonly string[] = [],
): CorrectedCopySwap {
  const loadedByKey = new Map(loadedPaths.map((p) => [normalizeOutputPath(p), p] as const));
  const pathOf = (copy: CosmeticCopy) => loadedByKey.get(normalizeOutputPath(copy.corrected)) ?? copy.corrected;
  const copyByOriginal = new Map(copies.map((c) => [normalizeOutputPath(c.original), c] as const));

  if (selected.some((p) => copyByOriginal.has(normalizeOutputPath(p)))) {
    const paths: string[] = [];
    const taken = new Set<string>();
    let count = 0;
    for (const p of selected) {
      const copy = copyByOriginal.get(normalizeOutputPath(p));
      if (copy) count += 1;
      const next = copy ? pathOf(copy) : p;
      const key = normalizeOutputPath(next);
      if (taken.has(key)) continue;
      taken.add(key);
      paths.push(next);
    }
    return { paths, count };
  }

  const selectedKeys = new Set(selected.map(normalizeOutputPath));
  const rejectedKeys = new Set(rejected.map(normalizeOutputPath));
  const added = copies
    .filter((c) => !rejectedKeys.has(normalizeOutputPath(c.original)) && !selectedKeys.has(normalizeOutputPath(c.corrected)))
    .map(pathOf);
  return { paths: [...selected, ...added], count: added.length };
}
