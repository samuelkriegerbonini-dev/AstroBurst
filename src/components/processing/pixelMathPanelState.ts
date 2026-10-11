import type { PixelMathSlot } from "../../shared/types/pixelmath";
import type { ChainStep, ProcessingChain } from "../../shared/types/preview";
import { CHAIN_ORDER, normalizeOutputPath } from "../../utils/processingChain";
import { fileBaseName } from "../../utils/spectrumExport";
import { TARGET_SYMBOL } from "../../utils/pixelmathSlots";

export interface ChainOutput {
  label: string;
  fitsPath: string;
}

export interface FileOption {
  value: string;
  label: string;
}

const CHAIN_OUTPUT_LABELS: Record<ChainStep, string> = {
  background: "Background",
  denoise: "Denoise",
  deconv: "Deconvolution",
  stretch: "Stretch",
  maskedStretch: "Masked stretch",
  localContrast: "LHE / HDRMT",
  tone: "Curves",
  pixelMath: "PixelMath",
};

export function chainOutputs(chain: ProcessingChain): ChainOutput[] {
  return CHAIN_ORDER.flatMap((step) => {
    const entry = chain.steps[step];
    return entry ? [{ label: `${CHAIN_OUTPUT_LABELS[step]} output`, fitsPath: entry.fitsPath }] : [];
  });
}

export function pixelMathFileOptions(
  target: string | null,
  doneFiles: readonly { path: string; name: string }[],
  slots: readonly { path: string }[],
  chainEntries: readonly ChainOutput[],
): FileOption[] {
  const seen = new Set<string>();
  const options: FileOption[] = [];
  const offer = (value: string, label: string) => {
    const key = normalizeOutputPath(value);
    if (!value || seen.has(key)) return;
    seen.add(key);
    options.push({ value, label });
  };
  if (target) offer(target, `${fileBaseName(target)} (${TARGET_SYMBOL})`);
  for (const f of doneFiles) offer(f.path, f.name || fileBaseName(f.path));
  for (const c of chainEntries) offer(c.fitsPath, c.label);
  for (const s of slots) offer(s.path, fileBaseName(s.path));
  return options;
}

const MAX_RETAINED_PANELS = 32;

export interface RetainedPanelState {
  expression: string;
  slots: PixelMathSlot[];
  truncate: boolean;
  rescale: boolean;
  outputName: string;
  slotsTouched: boolean;
}

const retained = new Map<string, RetainedPanelState>();

export function retainedFor(key: string | null): RetainedPanelState | undefined {
  return key ? retained.get(key) : undefined;
}

export function retain(key: string | null, state: RetainedPanelState): void {
  if (!key) return;
  retained.delete(key);
  retained.set(key, state);
  while (retained.size > MAX_RETAINED_PANELS) {
    const oldest = retained.keys().next().value;
    if (oldest === undefined) break;
    retained.delete(oldest);
  }
}

export function markSlotsTouched(key: string): void {
  const saved = retained.get(key);
  if (saved) retain(key, { ...saved, slotsTouched: true });
}

export function slotsTouchedFor(key: string | null): boolean {
  return retainedFor(key)?.slotsTouched ?? false;
}
