import { Download, RotateCcw, Sparkles, type LucideIcon } from "lucide-react";
import type { PaletteAction } from "../components/CommandPalette";
import { formatBytes } from "../components/archive/logic/productView";
import { PRIMARY_SAMPLE_SET, type SampleControls, type SampleDataStore, type SampleEntry, type SampleFailure } from "../hooks/useSampleData";
import { ingestUnlessLoaded, showOrOpen } from "../hooks/useFileIngest";
import type { SampleOpen } from "../services/sampleData";
import { partitionIncoming, rejectionReport, type IngestReport } from "./validation";

export const IFU_SAMPLE_SET = "jwst-gc-nirspec-ifu";
export const MIRI_LRS_SAMPLE_SET = "jwst-miri-lrs";
export const CUBE_SAMPLE_SETS = [IFU_SAMPLE_SET, MIRI_LRS_SAMPLE_SET] as const;

export const JWST_SAMPLE_PROGRAMS: Readonly<Record<string, string>> = {
  [IFU_SAMPLE_SET]: "1266",
  [MIRI_LRS_SAMPLE_SET]: "2072",
};

const SHOWN_PRODUCTS = [/_s3d\.fits$/i, /_x1d\.fits$/i];
const PRIMARY_KEYWORDS = ["Sample", "Example", "M16", "Eagle", "Download"];
const SAMPLE_SET_KEYWORDS = ["Sample", "Example", "Download", "JWST", "Cube", "Spectrum"];

export function shownSamplePath(paths: readonly string[]): string | null {
  for (const product of SHOWN_PRODUCTS) {
    const hit = paths.find((path) => product.test(path));
    if (hit) return hit;
  }
  return paths[0] ?? null;
}

export function openSampleProducts(opened: Pick<SampleOpen, "paths">): IngestReport | null {
  const partition = partitionIncoming(opened.paths);
  const shown = shownSamplePath(partition.accepted);
  for (const path of partition.accepted) {
    if (path === shown) showOrOpen(path);
    else ingestUnlessLoaded(path);
  }
  return rejectionReport(partition);
}

export const MAST_DATA_USE_URL = "https://archive.stsci.edu/publishing/data-use";

export function jwstSampleTerms(id: string): string | null {
  if (!Object.hasOwn(JWST_SAMPLE_PROGRAMS, id)) return null;
  return `Used under the MAST data use terms (${MAST_DATA_USE_URL}); publications acknowledge JWST program #${JWST_SAMPLE_PROGRAMS[id]} and STScI.`;
}

export function sampleDataHint(sample: Pick<SampleControls, "title" | "fileCount">): string {
  const files = sample.fileCount === 1 ? "1 file" : `${sample.fileCount} files`;
  return `${sample.title}: ${files}, downloaded once and kept in the data folder`;
}

type OfferSource = Pick<SampleControls, "state" | "receivedBytes" | "totalBytes" | "error">;

export function samplePercent(sample: Pick<SampleControls, "receivedBytes" | "totalBytes">): number {
  if (sample.totalBytes <= 0) return 0;
  return Math.min(100, Math.max(0, Math.floor((sample.receivedBytes * 100) / sample.totalBytes)));
}

function receivedText(sample: OfferSource): string {
  const received = formatBytes(sample.receivedBytes);
  return received === formatBytes(sample.totalBytes) ? `${samplePercent(sample)}%` : received;
}

export interface SampleOffer {
  opens: boolean;
  verb: string;
  amount: string | null;
  icon: LucideIcon;
}

export function sampleOffer(sample: OfferSource): SampleOffer | null {
  if (sample.state === "downloading") return null;
  if (sample.state === "ready") return { opens: true, verb: "Open sample data", amount: null, icon: Sparkles };
  if (sample.error) return { opens: false, verb: "Retry sample download", amount: null, icon: RotateCcw };
  if (sample.state === "partial" && sample.receivedBytes > 0) {
    return { opens: false, verb: "Resume sample download", amount: `${receivedText(sample)} of ${formatBytes(sample.totalBytes)}`, icon: Download };
  }
  return { opens: false, verb: "Download sample data", amount: formatBytes(sample.totalBytes), icon: Download };
}

export function sampleOfferLabel(offer: SampleOffer, subject: string | null = null): string {
  const detail = [subject, offer.amount].filter(Boolean).join(", ");
  return detail ? `${offer.verb} (${detail})` : offer.verb;
}

export interface SampleOfferAction {
  label: string;
  icon: LucideIcon;
  run: () => void;
}

export function sampleOfferAction(sample: SampleControls): SampleOfferAction | null {
  const offer = sampleOffer(sample);
  if (!offer) return null;
  return { label: sampleOfferLabel(offer), icon: offer.icon, run: () => void (offer.opens ? sample.open() : sample.download()) };
}

export type SampleNotice = (message: string) => void;

export function sampleFailureNotice(title: string, notify: SampleNotice): (failure: SampleFailure) => void {
  return (failure) => {
    if (failure) notify(`${title}: ${failure}`);
  };
}

export interface SampleSetRun {
  download: (id: string) => void;
  open: (id: string) => void;
}

export function sampleSetRun(store: Pick<SampleDataStore, "get" | "download" | "open">, show: () => void, notify: SampleNotice): SampleSetRun {
  const report = (id: string) => sampleFailureNotice(store.get().find((e) => e.id === id)?.title ?? id, notify);
  return {
    download: (id) => {
      show();
      void store.download(id, openSampleProducts).then(report(id));
    },
    open: (id) => {
      show();
      void store.open(id, openSampleProducts).then(report(id));
    },
  };
}

export type PaletteSampleSet = Pick<SampleEntry, "id" | "title" | "state" | "total_bytes" | "received_bytes" | "error">;

function paletteSet({ id, title, state, total_bytes, received_bytes, error }: SampleEntry): PaletteSampleSet {
  return { id, title, state, total_bytes, received_bytes: state === "downloading" ? 0 : received_bytes, error };
}

const PALETTE_FIELDS = ["id", "title", "state", "total_bytes", "received_bytes", "error"] as const satisfies readonly (keyof PaletteSampleSet)[];

function samePaletteSets(a: readonly PaletteSampleSet[], b: readonly PaletteSampleSet[]): boolean {
  return a.length === b.length && a.every((set, i) => PALETTE_FIELDS.every((field) => set[field] === b[i][field]));
}

export function paletteSampleSetsSelector(store: Pick<SampleDataStore, "get">): () => readonly PaletteSampleSet[] {
  let source: readonly SampleEntry[] | null = null;
  let sets: readonly PaletteSampleSet[] = [];
  return () => {
    const entries = store.get();
    if (entries === source) return sets;
    source = entries;
    const next = entries.map(paletteSet);
    if (!samePaletteSets(sets, next)) sets = next;
    return sets;
  };
}

function offerOf(set: PaletteSampleSet): SampleOffer | null {
  return sampleOffer({ state: set.state, receivedBytes: set.received_bytes, totalBytes: set.total_bytes, error: set.error });
}

interface PaletteEntryIds {
  open: string;
  download: string;
}

function paletteEntry(set: PaletteSampleSet, downloadShown: boolean, run: SampleSetRun, ids: PaletteEntryIds, keywords: string[]): PaletteAction | null {
  const offer = offerOf(set);
  if (offer?.opens) return { id: ids.open, label: sampleOfferLabel(offer, set.title), keywords, icon: offer.icon, run: () => run.open(set.id) };
  if (!downloadShown) return null;
  const entry = offer ? { label: sampleOfferLabel(offer, set.title), icon: offer.icon } : { label: `Show sample download (${set.title})`, icon: Download };
  return { id: ids.download, ...entry, keywords, run: () => run.download(set.id) };
}

export interface SamplePaletteView {
  workspaceEmpty: boolean;
  fileShown: boolean;
}

export interface SamplePaletteRuns {
  primary: SampleSetRun;
  cube: SampleSetRun;
}

export function samplePaletteActions(sets: readonly PaletteSampleSet[], view: SamplePaletteView, runs: SamplePaletteRuns): PaletteAction[] {
  return sets.flatMap((set): PaletteAction[] => {
    const entry =
      set.id === PRIMARY_SAMPLE_SET
        ? paletteEntry(set, view.workspaceEmpty, runs.primary, { open: "open-sample", download: "download-sample" }, PRIMARY_KEYWORDS)
        : paletteEntry(set, view.fileShown, runs.cube, { open: `open-sample-${set.id}`, download: `download-sample-${set.id}` }, SAMPLE_SET_KEYWORDS);
    return entry ? [entry] : [];
  });
}
