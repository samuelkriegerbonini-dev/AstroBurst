import { useSyncExternalStore, useCallback, useRef } from "react";
import { FILE_STATUS } from "../utils/constants";
import { generateId } from "../utils/format";
import { isCalibRefAsdf } from "../utils/validation";
import type { ProcessedFile, QueueStats, AstroFile, ProcessResult } from "../shared/types";

type Listener = () => void;

const enum NotifyChannel {
  All = 1,
  Stats = 2,
  Selected = 4,
  List = 8,
}

interface StoreState {
  fileIds: string[];
  fileMap: Map<string, ProcessedFile>;
  selected: string | null;
  isProcessing: boolean;
  stats: QueueStats;
  version: number;
  selectedVersion: number;
  statsVersion: number;
}

class FileStore {
  private state: StoreState = {
    fileIds: [],
    fileMap: new Map(),
    selected: null,
    isProcessing: false,
    stats: { total: 0, done: 0, failed: 0, totalBytes: 0 },
    version: 0,
    selectedVersion: 0,
    statsVersion: 0,
  };

  private listeners = new Set<Listener>();
  private fileListeners = new Map<string, Set<Listener>>();
  private statsListeners = new Set<Listener>();
  private selectedListeners = new Set<Listener>();
  private listListeners = new Set<Listener>();

  private pendingFlush = false;
  private pendingChannels = 0;
  private pendingFileIds = new Set<string>();

  private _doneFilesCache: ProcessedFile[] = [];
  private _doneFilesCacheVersion = -1;
  private _allFilesCache: ProcessedFile[] = [];
  private _allFilesCacheVersion = -1;

  subscribe = (listener: Listener) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  subscribeToFile = (id: string, listener: Listener) => {
    if (!this.fileListeners.has(id)) {
      this.fileListeners.set(id, new Set());
    }
    this.fileListeners.get(id)!.add(listener);
    return () => {
      const set = this.fileListeners.get(id);
      if (set) {
        set.delete(listener);
        if (set.size === 0) this.fileListeners.delete(id);
      }
    };
  };

  subscribeToStats = (listener: Listener) => {
    this.statsListeners.add(listener);
    return () => this.statsListeners.delete(listener);
  };

  subscribeToSelected = (listener: Listener) => {
    this.selectedListeners.add(listener);
    return () => this.selectedListeners.delete(listener);
  };

  subscribeToList = (listener: Listener) => {
    this.listListeners.add(listener);
    return () => this.listListeners.delete(listener);
  };

  getSnapshot = () => this.state;
  getFileIds = () => this.state.fileIds;
  getStats = () => this.state.stats;
  getStatsVersion = () => this.state.statsVersion;
  getSelected = () => this.state.selected;
  getSelectedVersion = () => this.state.selectedVersion;
  getVersion = () => this.state.version;
  getIsProcessing = () => this.state.isProcessing;

  getFile = (id: string): ProcessedFile | undefined => this.state.fileMap.get(id);
  getSelectedFile = (): ProcessedFile | null => {
    if (!this.state.selected) return null;
    return this.state.fileMap.get(this.state.selected) ?? null;
  };

  getFiles = (): ProcessedFile[] => {
    const v = this.state.version;
    if (v === this._allFilesCacheVersion) return this._allFilesCache;
    this._allFilesCache = this.state.fileIds.map((id) => this.state.fileMap.get(id)!);
    this._allFilesCacheVersion = v;
    return this._allFilesCache;
  };

  getDoneFiles = (): ProcessedFile[] => {
    const v = this.state.statsVersion;
    if (v === this._doneFilesCacheVersion) return this._doneFilesCache;
    const result: ProcessedFile[] = [];
    for (const id of this.state.fileIds) {
      const f = this.state.fileMap.get(id);
      if (f && f.status === FILE_STATUS.DONE) result.push(f);
    }
    this._doneFilesCache = result;
    this._doneFilesCacheVersion = v;
    return result;
  };

  getDoneCount = () => this.state.stats.done;
  getIsComplete = () => {
    const s = this.state.stats;
    return s.total > 0 && s.done + s.failed === s.total && !this.state.isProcessing;
  };
  getProgress = () => {
    const s = this.state.stats;
    return s.total > 0 ? Math.round(((s.done + s.failed) / s.total) * 100) : 0;
  };

  private scheduleFlush(channels: number, fileId?: string) {
    this.pendingChannels |= channels;
    if (fileId) this.pendingFileIds.add(fileId);

    if (this.pendingFlush) return;
    this.pendingFlush = true;

    queueMicrotask(() => {
      const ch = this.pendingChannels;
      const fileIds = this.pendingFileIds;
      this.pendingFlush = false;
      this.pendingChannels = 0;
      this.pendingFileIds = new Set();

      for (const fid of fileIds) {
        const set = this.fileListeners.get(fid);
        if (set) set.forEach((l) => l());
      }
      if (ch & NotifyChannel.Stats) this.statsListeners.forEach((l) => l());
      if (ch & NotifyChannel.Selected) this.selectedListeners.forEach((l) => l());
      if (ch & NotifyChannel.List) this.listListeners.forEach((l) => l());
      if (ch & NotifyChannel.All) this.listeners.forEach((l) => l());
    });
  }

  private bumpVersion() {
    this.state.version++;
  }

  addFiles(fileList: AstroFile[]) {
    const valid = fileList.filter((f) => !isCalibRefAsdf(f.name));
    if (valid.length === 0) return;

    let addedBytes = 0;
    const newFiles: ProcessedFile[] = valid.map((f) => {
      addedBytes += f.size || 0;
      return {
        id: generateId(),
        name: f.name,
        path: f.path,
        size: f.size,
        status: FILE_STATUS.QUEUED,
        result: null,
        error: null,
        startedAt: null,
        finishedAt: null,
      };
    });

    for (const f of newFiles) {
      this.state.fileIds.push(f.id);
      this.state.fileMap.set(f.id, f);
    }

    this.state.stats = {
      ...this.state.stats,
      total: this.state.fileIds.length,
      totalBytes: this.state.stats.totalBytes + addedBytes,
    };
    this.state.statsVersion++;
    this.bumpVersion();

    this.scheduleFlush(NotifyChannel.All | NotifyChannel.Stats | NotifyChannel.List);
  }

  setProcessing(value: boolean) {
    this.state.isProcessing = value;
    this.scheduleFlush(NotifyChannel.All | NotifyChannel.Stats);
  }

  fileStarted(id: string) {
    const existing = this.state.fileMap.get(id);
    if (!existing) return;

    const updated: ProcessedFile = {
      ...existing,
      status: FILE_STATUS.PROCESSING,
      startedAt: Date.now(),
    };
    this.state.fileMap.set(id, updated);
    this.bumpVersion();
    this.scheduleFlush(NotifyChannel.All, id);
  }

  fileDone(id: string, result: ProcessResult | null) {
    const existing = this.state.fileMap.get(id);
    if (!existing) return;

    const updated: ProcessedFile = {
      ...existing,
      status: FILE_STATUS.DONE,
      result,
      finishedAt: Date.now(),
    };
    this.state.fileMap.set(id, updated);

    const done = this.state.stats.done + 1;
    const autoSelect =
      this.state.selected === null && done === 1 ? id : this.state.selected;
    const selectedChanged = autoSelect !== this.state.selected;

    this.state.selected = autoSelect;
    this.state.stats = { ...this.state.stats, done };
    this.state.statsVersion++;
    if (selectedChanged) this.state.selectedVersion++;
    this.bumpVersion();

    let channels = NotifyChannel.All | NotifyChannel.Stats;
    if (selectedChanged) channels |= NotifyChannel.Selected;
    this.scheduleFlush(channels, id);
  }

  fileError(id: string, error: string) {
    const existing = this.state.fileMap.get(id);
    if (!existing) return;

    const updated: ProcessedFile = {
      ...existing,
      status: FILE_STATUS.ERROR,
      error,
      finishedAt: Date.now(),
    };
    this.state.fileMap.set(id, updated);

    this.state.stats = { ...this.state.stats, failed: this.state.stats.failed + 1 };
    this.state.statsVersion++;
    this.bumpVersion();

    this.scheduleFlush(NotifyChannel.All | NotifyChannel.Stats, id);
  }

  fileResampled(id: string, resampleResult: any) {
    const existing = this.state.fileMap.get(id);
    if (!existing) return;

    const updated: ProcessedFile = {
      ...existing,
      result: {
        ...(existing.result ?? {}),
        resampled: resampleResult,
        resampledPath: resampleResult.fits_path,
      } as ProcessResult,
    };
    this.state.fileMap.set(id, updated);
    this.bumpVersion();
    this.scheduleFlush(NotifyChannel.All, id);
  }

  selectFile(id: string) {
    if (this.state.selected === id) return;
    this.state.selected = id;
    this.state.selectedVersion++;
    this.scheduleFlush(NotifyChannel.All | NotifyChannel.Selected);
  }

  reset() {
    this.state = {
      fileIds: [],
      fileMap: new Map(),
      selected: null,
      isProcessing: false,
      stats: { total: 0, done: 0, failed: 0, totalBytes: 0 },
      version: 0,
      selectedVersion: 0,
      statsVersion: 0,
    };
    this._doneFilesCache = [];
    this._doneFilesCacheVersion = -1;
    this._allFilesCache = [];
    this._allFilesCacheVersion = -1;
    this.fileListeners.clear();
    this.pendingFileIds.clear();
    this.pendingChannels = 0;
    this.scheduleFlush(NotifyChannel.All | NotifyChannel.Stats | NotifyChannel.Selected | NotifyChannel.List);
  }
}

export const fileStore = new FileStore();

export function useFileIds(): string[] {
  return useSyncExternalStore(fileStore.subscribeToList, fileStore.getFileIds);
}

export function useFileEntry(id: string): ProcessedFile | undefined {
  const subscribe = useCallback(
    (listener: Listener) => fileStore.subscribeToFile(id, listener),
    [id],
  );
  const getSnapshot = useCallback(() => fileStore.getFile(id), [id]);
  return useSyncExternalStore(subscribe, getSnapshot);
}

export function useSelectedFile(): ProcessedFile | null {
  return useSyncExternalStore(fileStore.subscribeToSelected, fileStore.getSelectedFile);
}

export function useSelectedId(): string | null {
  return useSyncExternalStore(fileStore.subscribeToSelected, fileStore.getSelected);
}

export function useFileStats() {
  const stats = useSyncExternalStore(fileStore.subscribeToStats, fileStore.getStats);
  const isProcessing = useSyncExternalStore(fileStore.subscribeToStats, fileStore.getIsProcessing);
  const isComplete = useSyncExternalStore(fileStore.subscribeToStats, fileStore.getIsComplete);
  const progress = useSyncExternalStore(fileStore.subscribeToStats, fileStore.getProgress);
  return { stats, isProcessing, isComplete, progress };
}

export function useDoneFiles(): ProcessedFile[] {
  const versionRef = useRef(0);
  const cachedRef = useRef<ProcessedFile[]>([]);

  const subscribe = fileStore.subscribeToStats;
  const getSnapshot = useCallback(() => {
    const currentVersion = fileStore.getStatsVersion();
    if (currentVersion !== versionRef.current) {
      versionRef.current = currentVersion;
      cachedRef.current = fileStore.getDoneFiles();
    }
    return cachedRef.current;
  }, []);

  return useSyncExternalStore(subscribe, getSnapshot);
}

export function useAllFiles(): ProcessedFile[] {
  const versionRef = useRef(0);
  const cachedRef = useRef<ProcessedFile[]>([]);

  const subscribe = fileStore.subscribe;
  const getSnapshot = useCallback(() => {
    const currentVersion = fileStore.getVersion();
    if (currentVersion !== versionRef.current) {
      versionRef.current = currentVersion;
      cachedRef.current = fileStore.getFiles();
    }
    return cachedRef.current;
  }, []);

  return useSyncExternalStore(subscribe, getSnapshot);
}
