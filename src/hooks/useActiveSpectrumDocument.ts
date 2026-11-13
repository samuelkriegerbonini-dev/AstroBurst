import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";
import { fileKeyOf, useCubeContext, useFileContext } from "../context/PreviewContext";
import {
  listSpectrumSeries,
  listSpectrumTables,
  type SpectrumRequest,
  type SpectrumSeriesSet,
  type SpectrumTableOption,
  type WavelengthUnitOverride,
} from "../services/spectrum1d";
import type { Spectrum1dKind, TsoSeriesPick } from "../shared/types/spectrum1d";
import { spectrumHdu } from "../utils/fileKinds";
import { cubeToolContent, type CubeToolContent, type SpectrumDocument } from "../utils/spectrumDocument";
import { errorText, requestKey, useOpenSpectrumCsv, useSpectrum1dRequest, type Spectrum1dDoc } from "./useSpectrum1dStore";
import { documentViewKey } from "./useSpectrumViewStore";

export type SpectrumSelection =
  | { kind: "table"; hdu: number | null; wavelengthUnit: WavelengthUnitOverride | null }
  | { kind: "series"; detector: string; sporder: number | null; pick: TsoSeriesPick };

export interface SpectrumOptions {
  tables: SpectrumTableOption[] | null;
  series: SpectrumSeriesSet | null;
  loading: boolean;
  error: string | null;
}

type Listener = () => void;

const selections = new Map<string, SpectrumSelection>();
const selectionListeners = new Set<Listener>();

export const spectrumSelectionStore = {
  subscribe(listener: Listener): () => void {
    selectionListeners.add(listener);
    return () => {
      selectionListeners.delete(listener);
    };
  },
  get(fileKey: string): SpectrumSelection | null {
    return selections.get(fileKey) ?? null;
  },
  set(fileKey: string, selection: SpectrumSelection | null): void {
    if (selection === null) selections.delete(fileKey);
    else selections.set(fileKey, selection);
    selectionListeners.forEach((l) => l());
  },
  reset(): void {
    selections.clear();
    selectionListeners.forEach((l) => l());
  },
};

const PLAIN_TABLE: SpectrumSelection = { kind: "table", hdu: null, wavelengthUnit: null };

function requestOf(selection: SpectrumSelection, path: string): SpectrumRequest {
  if (selection.kind === "series") {
    return { kind: "series", paths: [path], detector: selection.detector, sporder: selection.sporder, pick: selection.pick };
  }
  return { kind: "file", path, hdu: selection.hdu, wavelengthUnit: selection.wavelengthUnit };
}

export function selectionKey(selection: SpectrumSelection): string {
  return requestKey(requestOf(selection, "")).slice(0, -1);
}

const MAX_LISTED_PATHS = 16;
const NO_SERIES_ERROR = "this file holds no series of spectra";
const NO_OPTIONS: SpectrumOptions = Object.freeze({ tables: null, series: null, loading: false, error: null }) as SpectrumOptions;
const LISTING: SpectrumOptions = Object.freeze({ tables: null, series: null, loading: true, error: null }) as SpectrumOptions;
const READING_SERIES: Spectrum1dDoc = Object.freeze({ loading: true, spectrum: null, error: null }) as Spectrum1dDoc;

type Listing = "tables" | "series";

const listed = new Map<string, SpectrumOptions>();
const listedListeners = new Set<Listener>();

function subscribeListed(listener: Listener): () => void {
  listedListeners.add(listener);
  return () => {
    listedListeners.delete(listener);
  };
}

function writeListed(key: string, options: SpectrumOptions): void {
  listed.set(key, options);
  listedListeners.forEach((l) => l());
}

function listOnce(listing: Listing, path: string): void {
  const key = `${listing}|${path}`;
  if (listed.has(key)) return;
  writeListed(key, LISTING);
  for (const oldest of listed.keys()) {
    if (listed.size <= MAX_LISTED_PATHS) break;
    listed.delete(oldest);
  }
  const found =
    listing === "series"
      ? listSpectrumSeries([path]).then((series): SpectrumOptions => ({ ...NO_OPTIONS, series }))
      : listSpectrumTables(path).then((tables): SpectrumOptions => ({ ...NO_OPTIONS, tables }));
  void found.then(
    (options) => {
      if (listed.has(key)) writeListed(key, options);
    },
    (reason: unknown) => {
      if (listed.has(key)) writeListed(key, { ...NO_OPTIONS, error: errorText(reason) });
    },
  );
}

function listingOf(kind: Spectrum1dKind | null): Listing | null {
  if (kind?.kind === "x1dints") return "series";
  if (kind?.kind === "x1d" && (kind.n_tables === undefined || kind.n_tables > 1)) return "tables";
  return null;
}

function seriesSelection(stored: SpectrumSelection | null, set: SpectrumSeriesSet | null): SpectrumSelection | null {
  if (set === null) return null;
  if (stored?.kind === "series" && set.series.some((s) => s.key.detector === stored.detector && s.key.sporder === stored.sporder)) {
    return stored;
  }
  const first = set.series[0];
  return first ? { kind: "series", detector: first.key.detector, sporder: first.key.sporder, pick: { kind: "median" } } : null;
}

function unlistedSeriesDoc(options: SpectrumOptions): Spectrum1dDoc {
  if (options.error !== null) return { loading: false, spectrum: null, error: options.error };
  if (options.series !== null) return { loading: false, spectrum: null, error: NO_SERIES_ERROR };
  return READING_SERIES;
}

export interface ActiveSpectrumDocument {
  content: CubeToolContent;
  fileKey: string | null;
  path: string | null;
  hdu: number | null;
  viewKey: string | null;
  doc: Spectrum1dDoc;
  document: SpectrumDocument | null;
  kind: Spectrum1dKind | null;
  selection: SpectrumSelection | null;
  request: SpectrumRequest | null;
  options: SpectrumOptions;
}

export function useActiveSpectrumDocument(): ActiveSpectrumDocument {
  const { file } = useFileContext();
  const { isCube, isSpectrum, ramp } = useCubeContext();
  const csv = useOpenSpectrumCsv();
  const fileKey = fileKeyOf(file);
  const content = cubeToolContent({ isCube, isRamp: ramp !== null, isSpectrum, fileKey, csv });
  const path = content === "spectrum" ? (file?.path ?? null) : content === "csv" && csv ? csv.path : null;
  const kind = content === "spectrum" ? (file?.spectrum ?? null) : null;
  const listing = path === null ? null : listingOf(kind);

  const getStored = useCallback(() => (fileKey === null ? null : spectrumSelectionStore.get(fileKey)), [fileKey]);
  const stored = useSyncExternalStore(spectrumSelectionStore.subscribe, getStored, getStored);
  const getOptions = useCallback(
    () => (listing === null ? NO_OPTIONS : (listed.get(`${listing}|${path}`) ?? LISTING)),
    [listing, path],
  );
  const options = useSyncExternalStore(subscribeListed, getOptions, getOptions);
  useEffect(() => {
    if (listing !== null && path !== null) listOnce(listing, path);
  }, [listing, path]);

  const series = options.series;
  const selection = useMemo<SpectrumSelection | null>(() => {
    if (content !== "spectrum") return null;
    if (kind?.kind === "x1dints") return seriesSelection(stored, series);
    return stored?.kind === "table" ? stored : { kind: "table", hdu: spectrumHdu(kind), wavelengthUnit: null };
  }, [content, kind, stored, series]);
  useEffect(() => {
    if (fileKey !== null && selection?.kind === "series" && selection !== stored) spectrumSelectionStore.set(fileKey, selection);
  }, [fileKey, selection, stored]);

  const request = useMemo<SpectrumRequest | null>(() => {
    if (path === null) return null;
    if (content === "csv") return requestOf(PLAIN_TABLE, path);
    return selection === null ? null : requestOf(selection, path);
  }, [content, path, selection]);
  const requested = useSpectrum1dRequest(request);
  const waiting = useMemo(() => unlistedSeriesDoc(options), [options]);
  const doc = request === null && kind?.kind === "x1dints" ? waiting : requested;

  const hdu = selection?.kind === "table" ? selection.hdu : null;
  const spectrum = doc.spectrum;
  const document = useMemo<SpectrumDocument | null>(
    () => (spectrum ? { source: { kind: "file", hdu }, spectrum } : null),
    [spectrum, hdu],
  );
  const viewKey = path === null ? null : documentViewKey(path, selection === null ? "" : selectionKey(selection));
  return { content, fileKey, path, hdu, viewKey, doc, document, kind, selection, request, options };
}
