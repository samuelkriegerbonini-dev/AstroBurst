import { useEffect, useMemo, useSyncExternalStore } from "react";
import { isTauri } from "../infrastructure/tauri";
import { onDataFolderChanged } from "../services/config";
import {
  cancelSampleDownload,
  downloadSampleSet,
  isSampleCancelled,
  isSampleNotDownloaded,
  listSampleSets,
  onSampleDownloadProgress,
  openSampleSet,
  sampleErrorMessage,
  type SampleOpen,
  type SampleProgress,
  type SampleSetReport,
  type SampleState,
} from "../services/sampleData";

export const PRIMARY_SAMPLE_SET = "hst-m16-wfpc2";

export interface SampleEntry extends SampleSetReport {
  file: string | null;
  error: string | null;
}

export interface SampleDataApi {
  list(): Promise<SampleSetReport[]>;
  download(id: string): Promise<SampleOpen>;
  cancel(id: string): Promise<boolean>;
  open(id: string): Promise<SampleOpen>;
}

export type OpenSampleFrames = (opened: SampleOpen) => void;

export interface SampleDataStore {
  get(): readonly SampleEntry[];
  subscribe(cb: () => void): () => void;
  refresh(): Promise<void>;
  progress(update: SampleProgress): void;
  download(id: string, onOpen: OpenSampleFrames): Promise<SampleFailure>;
  cancel(id: string): Promise<void>;
  open(id: string, onOpen: OpenSampleFrames): Promise<SampleFailure>;
}

export type SampleFailure = string | null;

export interface SampleActions {
  download: () => Promise<SampleFailure>;
  cancel: () => Promise<void>;
  open: () => Promise<SampleFailure>;
}

export interface SampleControls extends SampleActions {
  id: string;
  title: string;
  credit: string;
  licence: string;
  fileCount: number;
  state: SampleState;
  receivedBytes: number;
  totalBytes: number;
  file: string | null;
  error: string | null;
}

const SERVICE_API: SampleDataApi = {
  list: listSampleSets,
  download: downloadSampleSet,
  cancel: cancelSampleDownload,
  open: openSampleSet,
};

const NO_ENTRIES: readonly SampleEntry[] = [];
const UNOWNED_DOWNLOAD_POLL_MS = 1_000;
const SAMPLE_FRAMES_GONE = "The sample frames are no longer in the data folder";

export function createSampleDataStore(api: SampleDataApi = SERVICE_API): SampleDataStore {
  let entries = NO_ENTRIES;
  let pollScheduled = false;
  let listSeq = 0;
  const running = new Set<string>();
  const cancelRequested = new Set<string>();
  const listeners = new Set<() => void>();

  const entryOf = (id: string) => entries.find((e) => e.id === id);
  const patch = (id: string, change: Partial<SampleEntry>) => {
    entries = entries.map((e) => (e.id === id ? { ...e, ...change } : e));
    listeners.forEach((l) => l());
  };

  function pollUnownedDownloads(): void {
    if (pollScheduled || !entries.some((e) => e.state === "downloading" && !running.has(e.id))) return;
    pollScheduled = true;
    setTimeout(() => {
      pollScheduled = false;
      void refresh();
    }, UNOWNED_DOWNLOAD_POLL_MS);
  }

  async function refresh(): Promise<void> {
    const seq = ++listSeq;
    let sets: SampleSetReport[];
    try {
      sets = await api.list();
    } catch (err) {
      console.error("[AstroBurst] Sample data:", err);
      if (seq === listSeq) pollUnownedDownloads();
      return;
    }
    if (seq !== listSeq) return;
    const before = new Map(entries.map((e) => [e.id, e]));
    entries = sets.map((set): SampleEntry => {
      const previous = before.get(set.id);
      if (previous && running.has(set.id)) return { ...set, state: "downloading", received_bytes: previous.received_bytes, file: previous.file, error: null };
      return { ...set, file: null, error: previous?.error ?? null };
    });
    listeners.forEach((l) => l());
    pollUnownedDownloads();
  }

  function progress(update: SampleProgress): void {
    if (entryOf(update.id)?.state !== "downloading") return;
    patch(update.id, { received_bytes: update.received_bytes, total_bytes: update.total_bytes, file: update.file });
  }

  async function download(id: string, onOpen: OpenSampleFrames): Promise<SampleFailure> {
    const entry = entryOf(id);
    if (!entry || entry.state === "downloading") return null;
    running.add(id);
    cancelRequested.delete(id);
    patch(id, { state: "downloading", error: null });
    let opened: SampleOpen;
    try {
      opened = await api.download(id);
    } catch (err) {
      running.delete(id);
      cancelRequested.delete(id);
      const cancelled = isSampleCancelled(err);
      if (!cancelled) console.error("[AstroBurst] Sample data download:", err);
      const received = entryOf(id)?.received_bytes ?? 0;
      const error = cancelled ? null : sampleErrorMessage(err);
      patch(id, { state: received > 0 ? "partial" : "absent", file: null, error });
      await refresh();
      return error;
    }
    running.delete(id);
    const userCancelled = cancelRequested.delete(id);
    patch(id, { state: "ready", received_bytes: entryOf(id)?.total_bytes ?? 0, file: null, error: null });
    if (!userCancelled) onOpen(opened);
    await refresh();
    return null;
  }

  async function cancel(id: string): Promise<void> {
    if (entryOf(id)?.state !== "downloading") return;
    if (running.has(id)) cancelRequested.add(id);
    try {
      await api.cancel(id);
    } catch (err) {
      console.error("[AstroBurst] Sample data cancel:", err);
    }
  }

  async function open(id: string, onOpen: OpenSampleFrames): Promise<SampleFailure> {
    if (running.has(id) || !entryOf(id)) return null;
    let opened: SampleOpen;
    try {
      opened = await api.open(id);
    } catch (err) {
      const error = isSampleNotDownloaded(err) ? SAMPLE_FRAMES_GONE : sampleErrorMessage(err);
      patch(id, { error });
      await refresh();
      return error;
    }
    patch(id, { error: null });
    onOpen(opened);
    return null;
  }

  return {
    get: () => entries,
    subscribe: (cb: () => void) => {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    refresh,
    progress,
    download,
    cancel,
    open,
  };
}

export const sampleDataStore = createSampleDataStore();

export function sampleActions(store: SampleDataStore, id: string, onOpen: OpenSampleFrames): SampleActions {
  return {
    download: () => store.download(id, onOpen),
    cancel: () => store.cancel(id),
    open: () => store.open(id, onOpen),
  };
}

export function sampleControls(entry: SampleEntry | undefined, actions: SampleActions): SampleControls | null {
  if (!entry) return null;
  return {
    id: entry.id,
    title: entry.title,
    credit: entry.credit,
    licence: entry.licence,
    fileCount: entry.file_count,
    state: entry.state,
    receivedBytes: entry.received_bytes,
    totalBytes: entry.total_bytes,
    file: entry.file,
    error: entry.error,
    ...actions,
  };
}

export function startSampleData(store: SampleDataStore): () => void {
  void store.refresh();
  const unlisten = onSampleDownloadProgress(store.progress).catch((err: unknown) => {
    console.error("[AstroBurst] Sample data progress:", err);
    return () => {};
  });
  const offDataFolder = onDataFolderChanged(() => void store.refresh());
  return () => {
    offDataFolder();
    void unlisten.then((off) => off());
  };
}

export function useSampleData(onOpen: OpenSampleFrames, id: string = PRIMARY_SAMPLE_SET): SampleControls | null {
  const enabled = isTauri();
  const entries = useSyncExternalStore(sampleDataStore.subscribe, sampleDataStore.get, sampleDataStore.get);
  useEffect(() => (enabled ? startSampleData(sampleDataStore) : undefined), [enabled]);
  const actions = useMemo(() => sampleActions(sampleDataStore, id, onOpen), [id, onOpen]);
  const entry = enabled ? entries.find((e) => e.id === id) : undefined;
  return useMemo(() => sampleControls(entry, actions), [entry, actions]);
}
