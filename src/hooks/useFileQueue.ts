import { useCallback, useRef, useState } from "react";
import { FILE_STATUS } from "../utils/constants";
import { processFitsFull, processFits, resampleFits } from "../services/fits";
import { getHeader } from "../services/header";
import {
  fileStore,
  useFileStats,
  useSelectedFile,
  useSelectedId,
} from "./useFileStore";
import type { ProcessedFile, AstroFile } from "../shared/types";

const RESAMPLE_RATIO_THRESHOLD = 1.5;
const CONCURRENCY = 3;
const YIELD_INTERVAL = 4;

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

function shouldResample(groups: ResolutionGroup[]) {
  if (groups.length < 2) return { needed: false, targetGroup: null, resampleGroups: [] as ResolutionGroup[] };
  const sorted = [...groups].sort((a, b) => a.width * a.height - b.width * b.height);
  const smallest = sorted[0];
  const largest = sorted[sorted.length - 1];
  const ratio = (largest.width * largest.height) / (smallest.width * smallest.height);
  if (ratio < RESAMPLE_RATIO_THRESHOLD) return { needed: false, targetGroup: null, resampleGroups: [] as ResolutionGroup[] };
  return { needed: true, targetGroup: smallest, resampleGroups: sorted.slice(1) };
}

export function useFileQueue() {
  const processingRef = useRef(false);
  const [isResamplingState, setIsResampling] = useState(false);
  const [resampleProgressState, setResampleProgress] = useState(0);

  const { stats, isProcessing, isComplete, progress } = useFileStats();
  const selectedFile = useSelectedFile();
  const selected = useSelectedId();

  const addFiles = useCallback((fileList: AstroFile[]) => {
    fileStore.addFiles(fileList);
  }, []);

  const selectFile = useCallback((id: string) => {
    fileStore.selectFile(id);
  }, []);

  const processOneFile = useCallback(
    async (file: ProcessedFile) => {
      fileStore.fileStarted(file.id);
      try {
        const result = await processFitsFull(file.path);
        fileStore.fileDone(file.id, result);
      } catch (err: any) {
        const msg = err?.message || String(err);
        const isRetriable = !msg.includes("Calibration reference file")
          && !msg.includes("No such file")
          && !msg.includes("not found")
          && !msg.includes("Permission denied");

        if (isRetriable) {
          try {
            const result = await processFits(file.path);
            let header = null;
            try { header = await getHeader(file.path); } catch {}
            fileStore.fileDone(file.id, { ...result, header });
            return;
          } catch {}
        }
        fileStore.fileError(file.id, msg);
      }
    },
    [],
  );

  const runAutoResample = useCallback(async () => {
    const currentFiles = fileStore.getFiles();
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
        const result = await resampleFits(file.path, targetGroup.width, targetGroup.height);
        fileStore.fileResampled(file.id, result);
      } catch {}
      completed++;
      setResampleProgress(Math.round((completed / filesToResample.length) * 100));
      await yieldToUI();
    }

    setIsResampling(false);
  }, []);

  const pendingKickRef = useRef(false);

  const startProcessing = useCallback(
    async (onStart?: () => void, onComplete?: () => void) => {
      if (processingRef.current) {
        pendingKickRef.current = true;
        return;
      }

      const initialQueue = fileStore.getFiles().filter((f) => f.status === FILE_STATUS.QUEUED);
      if (initialQueue.length === 0) return;

      processingRef.current = true;
      pendingKickRef.current = false;
      fileStore.setProcessing(true);
      if (onStart) onStart();

      await yieldToUI();

      let hasWork = true;
      while (hasWork) {
        pendingKickRef.current = false;
        const queue = fileStore.getFiles().filter((f) => f.status === FILE_STATUS.QUEUED);
        if (queue.length === 0) {
          if (pendingKickRef.current) continue;
          break;
        }

        let idx = 0;
        let processedSinceYield = 0;
        const getNext = (): ProcessedFile | null => (idx >= queue.length ? null : queue[idx++]);

        const runNext = async (): Promise<void> => {
          let file: ProcessedFile | null;
          while ((file = getNext()) !== null) {
            await processOneFile(file);
            processedSinceYield++;
            if (processedSinceYield >= YIELD_INTERVAL) {
              processedSinceYield = 0;
              await yieldToUI();
            }
          }
        };

        const workers = Array.from(
          { length: Math.min(CONCURRENCY, queue.length) },
          () => runNext(),
        );
        await Promise.all(workers);

        const remaining = fileStore.getFiles().filter((f) => f.status === FILE_STATUS.QUEUED);
        hasWork = remaining.length > 0 || pendingKickRef.current;
      }

      processingRef.current = false;
      fileStore.setProcessing(false);
      await runAutoResample();

      if (onComplete) onComplete();
    },
    [processOneFile, runAutoResample],
  );

  const scheduleProcessing = useCallback(() => {
    if (processingRef.current) {
      pendingKickRef.current = true;
      return;
    }
    startProcessing();
  }, [startProcessing]);

  const reset = useCallback(() => {
    processingRef.current = false;
    pendingKickRef.current = false;
    setIsResampling(false);
    setResampleProgress(0);
    fileStore.reset();
  }, []);

  return {
    files: fileStore.getFiles(),
    selected,
    selectedFile,
    isProcessing,
    stats,
    progress,
    isComplete,
    addFiles,
    selectFile,
    startProcessing,
    scheduleProcessing,
    reset,
    isResampling: isResamplingState,
    resampleProgress: resampleProgressState,
  };
}
