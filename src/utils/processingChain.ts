import type { ChainBase, ChainEntry, ChainStep, FileRenderState, ProcessedKind, ProcessedResult, ProcessingChain } from "../shared/types/preview";
import type { ProcessResult } from "../shared/types";
import type { PsfSource } from "../shared/types/compositeChain";
import { fileBaseName } from "./spectrumExport";
import { toDims } from "./stackingOutputs";

export type PsfUse = Pick<ChainEntry, "psfUsed">;

export const CHAIN_ORDER: readonly ChainStep[] = [
  "background",
  "denoise",
  "deconv",
  "stretch",
  "maskedStretch",
  "localContrast",
  "tone",
  "pixelMath",
];

const STAGE: Record<ChainStep, number> = {
  background: 0,
  denoise: 1,
  deconv: 2,
  stretch: 3,
  maskedStretch: 3,
  localContrast: 4,
  tone: 5,
  pixelMath: 6,
};

export const EMPTY_CHAIN: ProcessingChain = Object.freeze({ steps: Object.freeze({}), psfKernel: null, base: null });

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
  return { steps, psfKernel: chain.psfKernel, base: chain.base };
}

export function inputFor(chain: ProcessingChain, step: ChainStep, originalPath: string): string {
  const stage = STAGE[step];
  for (let i = CHAIN_ORDER.length - 1; i >= 0; i--) {
    const s = CHAIN_ORDER[i];
    if (STAGE[s] >= stage) continue;
    const existing = chain.steps[s];
    if (existing) return existing.fitsPath;
  }
  return chainBasePath(chain, originalPath);
}

export function chainBasePath(chain: ProcessingChain, filePath: string): string {
  return chain.base?.path ?? filePath;
}

const REBASING_KINDS: ReadonlySet<ProcessedKind> = new Set(["stacking", "cube"]);
const PNG_ONLY_NOTICE_KINDS: ReadonlySet<ProcessedKind> = new Set(["stacking", "cube", "debayer"]);

function chainBaseOf(result: ProcessedResult): ChainBase | null {
  if (!result.fitsPath) return null;
  return {
    path: result.fitsPath,
    label: result.label,
    previewUrl: result.previewUrl,
    dimensions: result.dimensions,
    kind: result.kind,
    inputPath: result.inputPath,
  };
}

export function chainForPublish(
  baseChain: ProcessingChain,
  result: ProcessedResult,
  chainUpdate?: (c: ProcessingChain) => ProcessingChain,
): ProcessingChain {
  if (chainUpdate) return chainUpdate(baseChain);
  if (!REBASING_KINDS.has(result.kind)) return baseChain;
  return { steps: {}, psfKernel: null, base: chainBaseOf(result) };
}

export function pngOnlyInputNotice(processed: ProcessedResult | null, inputName: string): string | null {
  if (!processed || processed.fitsPath !== null || processed.previewUrl === null) return null;
  if (!PNG_ONLY_NOTICE_KINDS.has(processed.kind)) return null;
  return `Input: ${inputName}, not the image on screen (${processed.label} is a PNG-only result).`;
}

const FILE_INPUT_SECTIONS: ReadonlySet<string> = new Set(["debayer", "pixelmath", "geometry"]);

export function sectionInputName(section: string, base: ChainBase | null, fileName: string): string {
  return FILE_INPUT_SECTIONS.has(section) ? fileName : base?.label ?? fileName;
}

const CHAIN_RESULT_KINDS: ReadonlySet<ProcessedKind> = new Set(["processing", "pixelmath"]);

export function gridSourceOf<P extends Pick<ProcessedResult, "kind" | "inputPath">>(
  processed: P | null,
  chain: ProcessingChain,
): P | ChainBase | null {
  const base = chain.base;
  if (!processed || !base || !CHAIN_RESULT_KINDS.has(processed.kind)) return processed;
  const input = processed.inputPath;
  const fromChain = samePath(input, base.path) || CHAIN_ORDER.some((s) => {
    const entry = chain.steps[s];
    return entry !== undefined && samePath(entry.fitsPath, input);
  });
  return fromChain ? base : processed;
}

export function debayerPreviewRecord(runPath: string, previewUrl: string, dims: readonly number[] | null | undefined): ProcessedResult {
  return { fitsPath: null, previewUrl, dimensions: toDims(dims), label: "Debayer", kind: "debayer", inputPath: runPath };
}

export function backgroundInputFor<F extends { path: string; result: ProcessResult | null }>(file: F | null, base: ChainBase | null): F | null {
  if (!file || !base) return file;
  const result = file.result
    ? { ...file.result, previewUrl: base.previewUrl ?? "", dimensions: base.dimensions ?? file.result.dimensions }
    : null;
  return { ...file, path: base.path, result };
}

export function pixelMathInputPreview(
  processed: Pick<ProcessedResult, "kind" | "fitsPath" | "previewUrl" | "inputPreviewUrl"> | null,
  filePreviewUrl: string | null,
): string | null {
  if (!processed) return filePreviewUrl;
  if (processed.kind === "pixelmath") return processed.inputPreviewUrl ?? null;
  return processed.fitsPath === null ? filePreviewUrl : processed.previewUrl;
}

export function pixelMathTarget(processed: Pick<ProcessedResult, "kind" | "fitsPath" | "inputPath"> | null, filePath: string): string {
  return processed?.kind === "pixelmath" ? processed.inputPath : processed?.fitsPath ?? filePath;
}

export interface CompareBase {
  previewUrl: string | null;
  label: string;
}

export function pixelMathCompareBase(
  chain: ProcessingChain,
  processed: Pick<ProcessedResult, "kind" | "previewUrl" | "label" | "inputPath" | "inputPreviewUrl"> | null,
  original: { path: string; previewUrl: string | null },
  stepLabels: Readonly<Record<ChainStep, string>>,
): CompareBase {
  if (processed?.kind !== "pixelmath") return { previewUrl: processed?.previewUrl ?? original.previewUrl, label: processed?.label ?? "Original" };
  const input = processed.inputPath;
  const base = chain.base;
  if (base && samePath(input, base.path)) return { previewUrl: base.previewUrl, label: base.label };
  if (samePath(input, original.path)) return { previewUrl: original.previewUrl, label: "Original" };
  for (const step of CHAIN_ORDER) {
    const entry = chain.steps[step];
    if (step !== "pixelMath" && entry && samePath(entry.fitsPath, input)) return { previewUrl: entry.previewUrl, label: stepLabels[step] };
  }
  return { previewUrl: processed.inputPreviewUrl ?? null, label: fileBaseName(input) };
}

export function lastStep(chain: ProcessingChain): ChainStep | null {
  for (let i = CHAIN_ORDER.length - 1; i >= 0; i--) {
    const s = CHAIN_ORDER[i];
    if (chain.steps[s]) return s;
  }
  return null;
}

export function psfUseOf(step: ChainStep, psfSource: PsfSource | undefined): PsfUse {
  return step === "deconv" ? { psfUsed: psfSource === "provided" } : {};
}

export function showsPsfCrumb(psfKernel: number[][] | null, deconv: PsfUse | undefined): boolean {
  return psfKernel !== null && (deconv === undefined || deconv.psfUsed === true);
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
  return changed ? { steps, psfKernel: chain.psfKernel, base: chain.base } : chain;
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
