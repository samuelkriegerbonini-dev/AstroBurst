import { Download, RotateCcw, Sparkles, type LucideIcon } from "lucide-react";
import type { PaletteAction } from "../components/CommandPalette";
import { formatBytes } from "../components/archive/logic/productView";
import { PRIMARY_SAMPLE_SET, type SampleControls, type SampleEntry } from "../hooks/useSampleData";
import { ingestUnlessLoaded, showOrOpen } from "../hooks/useFileIngest";
import type { SampleOpen } from "../services/sampleData";

export const IFU_SAMPLE_SET = "jwst-gc-nirspec-ifu";
export const MIRI_LRS_SAMPLE_SET = "jwst-miri-lrs";
export const CUBE_SAMPLE_SETS = [IFU_SAMPLE_SET, MIRI_LRS_SAMPLE_SET] as const;

const SHOWN_PRODUCTS = [/_s3d\.fits$/i, /_x1d\.fits$/i];
const SAMPLE_SET_KEYWORDS = ["Sample", "Example", "Download", "JWST", "Cube", "Spectrum"];

export function shownSamplePath(paths: readonly string[]): string | null {
  for (const product of SHOWN_PRODUCTS) {
    const hit = paths.find((path) => product.test(path));
    if (hit) return hit;
  }
  return paths[0] ?? null;
}

export function openSampleProducts(opened: Pick<SampleOpen, "paths">): void {
  const shown = shownSamplePath(opened.paths);
  for (const path of opened.paths) {
    if (path === shown) showOrOpen(path);
    else ingestUnlessLoaded(path);
  }
}

export function samplePercent(sample: Pick<SampleControls, "receivedBytes" | "totalBytes">): number {
  if (sample.totalBytes <= 0) return 0;
  return Math.min(100, Math.max(0, Math.floor((sample.receivedBytes * 100) / sample.totalBytes)));
}

export interface SampleOfferAction {
  label: string;
  icon: LucideIcon;
  run: () => void;
}

export function sampleOfferAction(sample: SampleControls): SampleOfferAction | null {
  if (sample.state === "downloading") return null;
  if (sample.state === "ready") return { label: "Open sample", icon: Sparkles, run: () => void sample.open() };
  if (sample.error) return { label: "Retry sample download", icon: RotateCcw, run: () => void sample.retry() };
  if (sample.state === "partial" && sample.receivedBytes > 0) {
    return { label: `Resume sample download (${formatBytes(sample.receivedBytes)} of ${formatBytes(sample.totalBytes)})`, icon: Download, run: () => void sample.download() };
  }
  return { label: `Download sample (${formatBytes(sample.totalBytes)})`, icon: Download, run: () => void sample.download() };
}

export interface SampleSetRun {
  download: (id: string) => void;
  open: (id: string) => void;
}

export type PaletteSampleSet = Pick<SampleEntry, "id" | "title" | "state" | "total_bytes" | "received_bytes" | "error">;

export function paletteSampleSetsKey(entries: readonly SampleEntry[]): string {
  return JSON.stringify(
    entries.map(({ id, title, state, total_bytes, received_bytes, error }): PaletteSampleSet => ({
      id,
      title,
      state,
      total_bytes,
      received_bytes: state === "downloading" ? 0 : received_bytes,
      error,
    })),
  );
}

export function paletteSampleSets(key: string): PaletteSampleSet[] {
  return JSON.parse(key) as PaletteSampleSet[];
}

function sampleDownloadEntry(set: PaletteSampleSet): Pick<PaletteAction, "label" | "icon"> {
  if (set.state === "downloading") return { label: `Show sample download (${set.title})`, icon: Download };
  if (set.error) return { label: `Retry sample download (${set.title})`, icon: RotateCcw };
  if (set.state === "partial" && set.received_bytes > 0) {
    return { label: `Resume sample download (${set.title}, ${formatBytes(set.received_bytes)} of ${formatBytes(set.total_bytes)})`, icon: Download };
  }
  return { label: `Download sample data (${set.title}, ${formatBytes(set.total_bytes)})`, icon: Download };
}

export function sampleSetPaletteActions(sets: readonly PaletteSampleSet[], fileShown: boolean, run: SampleSetRun): PaletteAction[] {
  return sets.flatMap((set): PaletteAction[] => {
    if (set.id === PRIMARY_SAMPLE_SET) return [];
    if (set.state === "ready") {
      return [{ id: `open-sample-${set.id}`, label: `Open sample data (${set.title})`, keywords: SAMPLE_SET_KEYWORDS, icon: Sparkles, run: () => run.open(set.id) }];
    }
    if (!fileShown) return [];
    return [{ id: `download-sample-${set.id}`, ...sampleDownloadEntry(set), keywords: SAMPLE_SET_KEYWORDS, run: () => run.download(set.id) }];
  });
}
