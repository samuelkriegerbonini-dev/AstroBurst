import { useReducer, useCallback, useRef } from "react";
import { FILE_STATUS } from "../utils/constants";
import { generateId } from "../utils/format";
import { useBackend } from "./useBackend";
import type { ProcessedFile, QueueStats, AstroFile } from "../utils/types";

const OUTPUT_DIR = "./output";

interface State {
  files: ProcessedFile[];
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
  | { type: "RESET" };

const initialState: State = {
  files: [],
  selected: null,
  isProcessing: false,
  stats: { total: 0, done: 0, failed: 0, totalBytes: 0 },
};

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
      return { ...state, files };
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
        stats: { ...state.stats, failed },
      };
    }

    case "PROCESSING_COMPLETE":
      return { ...state, isProcessing: false };

    case "SELECT_FILE":
      return { ...state, selected: action.payload };

    case "RESET":
      return { ...initialState };

    default:
      return state;
  }
}

export function useFileQueue() {
  const [state, dispatch] = useReducer(reducer, initialState);
  const { processFits, getHeader } = useBackend();
  const processingRef = useRef(false);
  const stateRef = useRef(state);
  stateRef.current = state;

  const addFiles = useCallback((fileList: AstroFile[]) => {
    dispatch({ type: "ADD_FILES", payload: fileList });
  }, []);

  const selectFile = useCallback((id: string) => {
    dispatch({ type: "SELECT_FILE", payload: id });
  }, []);

  const startProcessing = useCallback(
    async (onStart?: () => void, onComplete?: () => void) => {
      if (processingRef.current) return;
      processingRef.current = true;
      dispatch({ type: "START_PROCESSING" });
      if (onStart) onStart();

      await new Promise((r) => setTimeout(r, 50));

      const currentFiles = stateRef.current.files;
      const filesToProcess = currentFiles.filter((f) => f.status === FILE_STATUS.QUEUED);

      for (const file of filesToProcess) {
        dispatch({ type: "FILE_STARTED", payload: { id: file.id } });

        try {
          const result = await processFits(file.path, OUTPUT_DIR);
          let header = null;
          try {
            header = await getHeader(file.path);
          } catch (e) {
            console.warn("[AstroKit] Header fetch failed:", e);
          }
          dispatch({
            type: "FILE_DONE",
            payload: { id: file.id, result: { ...result, header } },
          });
        } catch (err: any) {
          console.error("[AstroKit] Process failed:", file.name, err);
          dispatch({
            type: "FILE_ERROR",
            payload: { id: file.id, error: err.message || String(err) },
          });
        }

        await new Promise((r) => setTimeout(r, 0));
      }

      dispatch({ type: "PROCESSING_COMPLETE" });
      processingRef.current = false;
      if (onComplete) onComplete();
    },
    [processFits, getHeader],
  );

  const reset = useCallback(() => {
    processingRef.current = false;
    dispatch({ type: "RESET" });
  }, []);

  const selectedFile = state.files.find((f) => f.id === state.selected) || null;

  const progress =
    state.stats.total > 0
      ? Math.round(((state.stats.done + state.stats.failed) / state.stats.total) * 100)
      : 0;

  const isComplete =
    state.stats.total > 0 &&
    state.stats.done + state.stats.failed === state.stats.total &&
    !state.isProcessing;

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
    dispatch,
  };
}
