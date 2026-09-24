import type { ChainEntry, ChainStep, FileRenderState, ProcessingChain } from "../shared/types/preview";

export const CHAIN_ORDER: readonly ChainStep[] = [
  "background",
  "denoise",
  "deconv",
  "stretch",
  "maskedStretch",
  "localContrast",
  "pixelMath",
];

const STAGE: Record<ChainStep, number> = {
  background: 0,
  denoise: 1,
  deconv: 2,
  stretch: 3,
  maskedStretch: 3,
  localContrast: 4,
  pixelMath: 5,
};

export const EMPTY_CHAIN: ProcessingChain = Object.freeze({ steps: Object.freeze({}), psfKernel: null });

export function normalizeOutputPath(path: string): string {
  return path.replace(/\\/g, "/").toLowerCase();
}

export function samePath(a: string, b: string): boolean {
  return normalizeOutputPath(a) === normalizeOutputPath(b);
}

export function withStep(chain: ProcessingChain, step: ChainStep, entry: ChainEntry): ProcessingChain {
  const stage = STAGE[step];
  const steps: Partial<Record<ChainStep, ChainEntry>> = {};
  for (const s of CHAIN_ORDER) {
    const existing = chain.steps[s];
    if (existing && STAGE[s] < stage) steps[s] = existing;
  }
  steps[step] = entry;
  return { steps, psfKernel: chain.psfKernel };
}

export function inputFor(chain: ProcessingChain, step: ChainStep, originalPath: string): string {
  const stage = STAGE[step];
  for (let i = CHAIN_ORDER.length - 1; i >= 0; i--) {
    const s = CHAIN_ORDER[i];
    if (STAGE[s] >= stage) continue;
    const existing = chain.steps[s];
    if (existing) return existing.fitsPath;
  }
  return originalPath;
}

export function lastStep(chain: ProcessingChain): ChainStep | null {
  for (let i = CHAIN_ORDER.length - 1; i >= 0; i--) {
    const s = CHAIN_ORDER[i];
    if (chain.steps[s]) return s;
  }
  return null;
}

export function chainHoldsOutput(chain: ProcessingChain, step: ChainStep, fitsPath: string | null | undefined): boolean {
  if (!fitsPath) return true;
  const entry = chain.steps[step];
  return entry !== undefined && samePath(entry.fitsPath, fitsPath);
}

export function hasAnyStep(chain: ProcessingChain): boolean {
  return CHAIN_ORDER.some((s) => chain.steps[s] !== undefined);
}

export function dropPaths(chain: ProcessingChain, paths: readonly string[]): ProcessingChain {
  if (paths.length === 0) return chain;
  const doomed = new Set(paths.map(normalizeOutputPath));
  let changed = false;
  const steps: Partial<Record<ChainStep, ChainEntry>> = {};
  for (const s of CHAIN_ORDER) {
    const existing = chain.steps[s];
    if (!existing) continue;
    if (doomed.has(normalizeOutputPath(existing.fitsPath))) {
      changed = true;
      continue;
    }
    steps[s] = existing;
  }
  return changed ? { steps, psfKernel: chain.psfKernel } : chain;
}

export function pruneRecord<T extends FileRenderState>(record: T, paths: readonly string[]): T | null {
  if (paths.length === 0) return record;
  const fitsPath = record.processed?.fitsPath;
  if (fitsPath && paths.some((p) => samePath(p, fitsPath))) return null;
  const chain = dropPaths(record.chain, paths);
  return chain === record.chain ? record : { ...record, chain };
}

export function withVersionParam(url: string, version: number): string {
  const hashAt = url.indexOf("#");
  const beforeHash = hashAt >= 0 ? url.slice(0, hashAt) : url;
  const hash = hashAt >= 0 ? url.slice(hashAt) : "";
  const queryAt = beforeHash.indexOf("?");
  const base = queryAt >= 0 ? beforeHash.slice(0, queryAt) : beforeHash;
  const query = queryAt >= 0 ? beforeHash.slice(queryAt + 1) : "";
  const params = query.split("&").filter((p) => p !== "" && !p.startsWith("v="));
  params.push(`v=${version}`);
  return `${base}?${params.join("&")}${hash}`;
}

export function putCapped<K, V>(map: Map<K, V>, key: K, value: V, cap: number): void {
  map.delete(key);
  while (map.size >= cap) {
    const oldest = map.keys().next();
    if (oldest.done) break;
    map.delete(oldest.value);
  }
  map.set(key, value);
}
