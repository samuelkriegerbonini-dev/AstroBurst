import { useReducer, useCallback, useRef, useState, useMemo } from "react";
import { FILE_STATUS } from "../utils/constants";
import { generateId } from "../utils/format";
import { useBackend } from "./useBackend";
import type {ProcessedFile, QueueStats, AstroFile, ProcessResult} from "../utils/types";

const OUTPUT_DIR = "./output";
const RESAMPLE_RATIO_THRESHOLD = 1.5;

const CALIB_REF_RE =
  /^jwst_[a-z]+_(distortion|filteroffset|sirskernel|photom|flat|dark|bias|readnoise|gain|linearity|saturation|superbias|ipc|area|specwcs|regions|wavelengthrange|trappars|mask|drizpars|throughput|psfmask)_\d+\.asdf$/i;

function isCalibRefAsdf(name: string): boolean {
  return CALIB_REF_RE.test(name);
}

interface State {
  files: ProcessedFile[];
  fileMap: Map<string, ProcessedFile>;
  selected: string | null;
  isProcessing: boolean;
  stats: QueueStats;
}

type Action =
  | { type: "ADD_FILES"; payload: AstroFile[] }
  | { type: "START_PROCESSING" }
  | { type: "FILE_STARTED"; payload: { id: string } }
  | { type: "FILE_DONE"; payload: { id: string; result: any } }
  | { type: "FILE_ERROR"; payload: { id: string; error: string } }
  | { type: "PROCESSING_COMPLETE" }
  | { type: "SELECT_FILE"; payload: string }
  | { type: "FILE_RESAMPLED"; payload: { id: string; resampleResult: any } }
  | { type: "RESET" };

const initialState: State = {
  files: [],
  fileMap: new Map(),
  selected: null,
  isProcessing: false,
  stats: { total: 0, done: 0, failed: 0, totalBytes: 0 },
};

function rebuildMap(files: ProcessedFile[]): Map<string, ProcessedFile> {
  const map = new Map();
  for (const f of files) map.set(f.id, f);
  return map;
}

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "ADD_FILES": {
      const newFiles: ProcessedFile[] = action.payload.map((f) => ({
        id: generateId(),
        name: f.name,
        path: f.path,
        size: f.size,
        status: FILE_STATUS.QUEUED,
        result: null,
        error: null,
        startedAt: null,
        finishedAt: null,
      }));
      const files = [...state.files, ...newFiles];
      return {
        ...state,
        files,
        fileMap: rebuildMap(files),
        stats: {
          ...state.stats,
          total: files.length,
          totalBytes: files.reduce((a, f) => a + (f.size || 0), 0),
        },
      };
    }

    case "START_PROCESSING":
      return { ...state, isProcessing: true };

    case "FILE_STARTED": {
      const files = state.files.map((f) =>
        f.id === action.payload.id
          ? { ...f, status: FILE_STATUS.PROCESSING as const, startedAt: Date.now() }
          : f,
      );
      return { ...state, files, fileMap: rebuildMap(files) };
    }

    case "FILE_DONE": {
      const files = state.files.map((f) =>
        f.id === action.payload.id
          ? {
            ...f,
            status: FILE_STATUS.DONE as const,
            result: action.payload.result,
            finishedAt: Date.now(),
          }
          : f,
      );
      const done = files.filter((f) => f.status === FILE_STATUS.DONE).length;
      const autoSelect =
        state.selected === null && done === 1 ? action.payload.id : state.selected;
      return {
        ...state,
        files,
        fileMap: rebuildMap(files),
        selected: autoSelect,
        stats: { ...state.stats, done },
      };
    }

    case "FILE_ERROR": {
      const files = state.files.map((f) =>
        f.id === action.payload.id
          ? {
            ...f,
            status: FILE_STATUS.ERROR as const,
            error: action.payload.error,
            finishedAt: Date.now(),
          }
          : f,
      );
      const failed = files.filter((f) => f.status === FILE_STATUS.ERROR).length;
      return {
        ...state,
        files,
        fileMap: rebuildMap(files),
        stats: { ...state.stats, failed },
      };
    }

    case "PROCESSING_COMPLETE":
      return { ...state, isProcessing: false };

    case "SELECT_FILE":
      return { ...state, selected: action.payload };

    case "FILE_RESAMPLED": {
      const files = state.files.map((f) =>
        f.id === action.payload.id
          ? {
            ...f,
            result: {
              ...(f.result ?? {}),
              resampled: action.payload.resampleResult,
              resampledPath: action.payload.resampleResult.fits_path,
            } as ProcessResult,
          }
          : f,
      );
      return { ...state, files, fileMap: rebuildMap(files) };
    }

    case "RESET":
      return { ...initialState, fileMap: new Map() };

    default:
      return state;
  }
}

const CONCURRENCY = 3;

function yieldToUI(): Promise<void> {
  return new Promise((resolve) => {
    if (typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => setTimeout(resolve, 0));
    } else {
      setTimeout(resolve, 16);
    }
  });
}

interface ResolutionGroup {
  width: number;
  height: number;
  files: ProcessedFile[];
}

function detectResolutionGroups(files: ProcessedFile[]): ResolutionGroup[] {
  const groups: ResolutionGroup[] = [];

  for (const file of files) {
    if (file.status !== FILE_STATUS.DONE || !file.result?.dimensions) continue;
    const [w, h] = file.result.dimensions;
    const existing = groups.find(
      (g) => Math.abs(g.width - w) < 10 && Math.abs(g.height - h) < 10,
    );
    if (existing) {
      existing.files.push(file);
    } else {
      groups.push({ width: w, height: h, files: [file] });
    }
  }

  return groups;
}
function shouldResample(groups: ResolutionGroup[]): {
  needed: boolean;
  targetGroup: ResolutionGroup | null;
  resampleGroups: ResolutionGroup[];
} {
  if (groups.length < 2) {
    return { needed: false, targetGroup: null, resampleGroups: [] };
  }

  const sorted = [...groups].sort(
    (a, b) => a.width * a.height - b.width * b.height,
  );
  const smallest = sorted[0];
  const largest = sorted[sorted.length - 1];

  const ratio = (largest.width * largest.height) / (smallest.width * smallest.height);

  if (ratio < RESAMPLE_RATIO_THRESHOLD) {
    return { needed: false, targetGroup: null, resampleGroups: [] };
  }

  const resampleGroups = sorted.slice(1);

  return { needed: true, targetGroup: smallest, resampleGroups };
}

export function useFileQueue() {
  const [state, dispatch] = useReducer(reducer, initialState);
  const { processFitsFull, processFits, getHeader, resampleFits } = useBackend();
  const processingRef = useRef(false);
  const stateRef = useRef(state);
  stateRef.current = state;

  const [isResampling, setIsResampling] = useState(false);
  const [resampleProgress, setResampleProgress] = useState(0);

  const addFiles = useCallback((fileList: AstroFile[]) => {
    const valid: AstroFile[] = [];
    const skipped: string[] = [];

    for (const f of fileList) {
      if (isCalibRefAsdf(f.name)) {
        skipped.push(f.name);
      } else {
        valid.push(f);
      }
    }

    if (skipped.length > 0) {
      console.warn(
        `[AstroBurst] Skipped ${skipped.length} calibration reference file(s):`,
        skipped,
      );
    }

    if (valid.length > 0) {
      dispatch({ type: "ADD_FILES", payload: valid });
    }
  }, []);

  const selectFile = useCallback((id: string) => {
    dispatch({ type: "SELECT_FILE", payload: id });
  }, []);

  const processOneFile = useCallback(
    async (file: ProcessedFile) => {
      dispatch({ type: "FILE_STARTED", payload: { id: file.id } });
      try {
        const result = await processFitsFull(file.path, OUTPUT_DIR);
        dispatch({
          type: "FILE_DONE",
          payload: { id: file.id, result },
        });
      } catch (fullErr: any) {
        try {
          const result = await processFits(file.path, OUTPUT_DIR);
          let header = null;
          try {
            header = await getHeader(file.path);
          } catch (e) {
            console.warn("[AstroBurst] Header fetch failed:", e);
          }
          dispatch({
            type: "FILE_DONE",
            payload: { id: file.id, result: { ...result, header } },
          });
        } catch (err: any) {
          console.error("[AstroBurst] Process failed:", file.name, err);
          dispatch({
            type: "FILE_ERROR",
            payload: { id: file.id, error: err.message || String(err) },
          });
        }
      }
    },
    [processFitsFull, processFits, getHeader],
  );

  const runAutoResample = useCallback(async () => {
    const currentFiles = stateRef.current.files;
    const groups = detectResolutionGroups(currentFiles);
    const { needed, targetGroup, resampleGroups } = shouldResample(groups);

    if (!needed || !targetGroup) return;

    setIsResampling(true);
    setResampleProgress(0);
    await yieldToUI();

    const filesToResample = resampleGroups.flatMap((g) => g.files);
    let completed = 0;

    for (const file of filesToResample) {
      try {
        const result = await resampleFits(
          file.path,
          targetGroup.width,
          targetGroup.height,
          OUTPUT_DIR,
        );
        dispatch({
          type: "FILE_RESAMPLED",
          payload: { id: file.id, resampleResult: result },
        });
      } catch (err: any) {
        console.error("[AstroBurst] Resample failed:", file.name, err);
      }
      completed++;
      setResampleProgress(Math.round((completed / filesToResample.length) * 100));
      await yieldToUI();
    }

    setIsResampling(false);
  }, [resampleFits]);

  const startProcessing = useCallback(
    async (onStart?: () => void, onComplete?: () => void) => {
      if (processingRef.current) return;
      processingRef.current = true;
      dispatch({ type: "START_PROCESSING" });
      if (onStart) onStart();

      await yieldToUI();

      const currentFiles = stateRef.current.files;
      const queue = currentFiles.filter((f) => f.status === FILE_STATUS.QUEUED);

      let idx = 0;
      const getNext = (): ProcessedFile | null => {
        if (idx >= queue.length) return null;
        return queue[idx++];
      };

      const runNext = async (): Promise<void> => {
        let file: ProcessedFile | null;
        while ((file = getNext()) !== null) {
          await processOneFile(file);
          await yieldToUI();
        }
      };

      const workers = Array.from(
        { length: Math.min(CONCURRENCY, queue.length) },
        () => runNext(),
      );
      await Promise.all(workers);

      dispatch({ type: "PROCESSING_COMPLETE" });
      await runAutoResample();

      processingRef.current = false;
      if (onComplete) onComplete();
    },
    [processOneFile, runAutoResample],
  );

  const reset = useCallback(() => {
    processingRef.current = false;
    setIsResampling(false);
    setResampleProgress(0);
    dispatch({ type: "RESET" });
  }, []);

  const selectedFile = useMemo(
    () => (state.selected ? state.fileMap.get(state.selected) ?? null : null),
    [state.fileMap, state.selected],
  );

  const progress = useMemo(
    () =>
      state.stats.total > 0
        ? Math.round(((state.stats.done + state.stats.failed) / state.stats.total) * 100)
        : 0,
    [state.stats],
  );

  const isComplete = useMemo(
    () =>
      state.stats.total > 0 &&
      state.stats.done + state.stats.failed === state.stats.total &&
      !state.isProcessing &&
      !isResampling,
    [state.stats, state.isProcessing, isResampling],
  );

  return {
    files: state.files,
    selected: state.selected,
    selectedFile,
    isProcessing: state.isProcessing,
    stats: state.stats,
    progress,
    isComplete,
    addFiles,
    selectFile,
    startProcessing,
    reset,
    isResampling,
    resampleProgress,
  };
}
