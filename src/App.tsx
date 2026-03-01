import { useState, useCallback, useEffect, useRef } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Plus, RotateCcw } from "lucide-react";

import DropZone from "./components/DropZone";
import EmptyState from "./components/EmptyState";
import FileList from "./components/FileList";
import PreviewPanel from "./components/PreviewPanel";
import StatsBar from "./components/StatsBar";
import GlobalProgress from "./components/GlobalProgress";
import DownloadButton from "./components/DownloadButton";
import Confetti from "./components/Confetti";
import ErrorBoundary from "./components/ErrorBoundary";
import { AstroLogo } from "./components/AstroLogo";

import { useFileQueue } from "./hooks/useFileQueue";
import { useTimer } from "./hooks/useTimer";
import { useZipExport } from "./hooks/useZipExport";
import { isValidFitsFile } from "./utils/validation";

import type { AstroFile } from "./utils/types";

import nebulaImg from "./assets/nebulosa.jpg";

const isTauri = (): boolean => !!(window as any).__TAURI_INTERNALS__;

type ViewState = "empty" | "processing" | "complete";

export default function App() {
  const [loading, setLoading] = useState(true);
  const [view, setView] = useState<ViewState>("empty");
  const [showConfetti, setShowConfetti] = useState(false);
  const prevCompleteRef = useRef(false);

  const {
    files,
    selected,
    selectedFile,
    isProcessing,
    stats,
    progress,
    isComplete,
    addFiles,
    selectFile,
    startProcessing,
    reset,
  } = useFileQueue();

  const timer = useTimer();
  const { exportZip, progress: zipProgress, isExporting, downloaded } = useZipExport();

  useEffect(() => {
    const t = setTimeout(() => setLoading(false), 1800);
    return () => clearTimeout(t);
  }, []);

  const handleFilesAdded = useCallback(
    (newFiles: AstroFile[]) => {
      if (newFiles.length === 0) return;
      addFiles(newFiles);
      if (view === "empty" || view === "complete") {
        setView("processing");
      }
    },
    [addFiles, view],
  );

  useEffect(() => {
    if (view === "processing" && files.length > 0 && !isProcessing && !isComplete) {
      const queued = files.filter((f: any) => f.status === "queued");
      if (queued.length > 0) {
        startProcessing(
          () => timer.start(),
          () => timer.stop(),
        );
      }
    }
  }, [view, files.length, isProcessing, isComplete]);

  useEffect(() => {
    if (isComplete && !prevCompleteRef.current) {
      setView("complete");
      setShowConfetti(true);
      setTimeout(() => setShowConfetti(false), 3000);
    }
    prevCompleteRef.current = isComplete;
  }, [isComplete]);

  const handleBrowseFiles = useCallback(async () => {
    if (isTauri()) {
      try {
        const { open } = await import("@tauri-apps/plugin-dialog");
        const result = await open({
          multiple: true,
          filters: [{ name: "FITS", extensions: ["fits", "fit", "fts"] }],
        });
        if (result) {
          const paths = Array.isArray(result) ? result : [result];
          const mapped: AstroFile[] = paths.map((p: string) => ({
            name: p.split(/[/\\]/).pop() || "Unknown",
            path: p,
            size: 0,
          }));
          handleFilesAdded(mapped);
        }
      } catch (err) {
        console.error("[AstroBurst] File dialog error:", err);
      }
    } else {
      const input = document.createElement("input");
      input.type = "file";
      input.multiple = true;
      input.accept = ".fits,.fit,.fts";
      input.onchange = (e: any) => {
        const list = Array.from(e.target.files as FileList)
          .filter((f) => isValidFitsFile(f.name))
          .map((f) => ({ name: f.name, path: f.name, size: f.size }));
        if (list.length > 0) handleFilesAdded(list);
      };
      input.click();
    }
  }, [handleFilesAdded]);

  const handleSelectFolder = useCallback(async () => {
    if (!isTauri()) {
      handleBrowseFiles();
      return;
    }
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const { readDir } = await import("@tauri-apps/plugin-fs");

      const result = await open({
        directory: true,
        multiple: false,
        title: "Select FITS Folder",
      });

      const dir = typeof result === "string" ? result : null;
      if (!dir) return;

      const entries = await readDir(dir);
      const fitsFiles: AstroFile[] = [];

      for (const entry of entries) {
        const name = entry.name || "";
        if (isValidFitsFile(name) && !entry.isDirectory) {
          const sep = dir.includes("\\") ? "\\" : "/";
          fitsFiles.push({
            name,
            path: `${dir}${sep}${name}`,
            size: 0,
          });
        }
      }

      if (fitsFiles.length > 0) handleFilesAdded(fitsFiles);
    } catch (err) {
      console.error("[AstroBurst] Folder dialog error:", err);
    }
  }, [handleFilesAdded, handleBrowseFiles]);

  const handleNewBatch = useCallback(() => {
    reset();
    timer.reset();
    setView("empty");
    setShowConfetti(false);
  }, [reset, timer]);

  return (
    <ErrorBoundary>
      <div className="relative h-screen w-full bg-zinc-950 text-zinc-100 overflow-hidden">
        <Confetti show={showConfetti} />

        <div
          className="fixed inset-0 z-0 opacity-40 pointer-events-none transition-all duration-1000"
          style={{
            backgroundImage: `url(${nebulaImg})`,
            backgroundSize: "cover",
            backgroundPosition: "center",
            filter: view !== "empty" ? "blur(8px) brightness(0.3)" : "none",
          }}
        />

        <AnimatePresence mode="wait">
          {loading ? (
            <motion.div
              key="splash"
              exit={{ opacity: 0 }}
              className="relative z-50 h-screen flex flex-col items-center justify-center bg-zinc-950"
            >
              <AstroLogo size={80} showText={false} className="animate-pulse" />
              <h1 className="mt-6 text-xl tracking-[0.5em] uppercase text-blue-400">
                AstroBurst
              </h1>
            </motion.div>
          ) : (
            <div key="app" className="relative z-10 h-full">
              <DropZone onFilesAdded={handleFilesAdded}>
                {view === "empty" ? (
                  <div className="h-full flex items-center justify-center">
                    <EmptyState
                      onBrowseFiles={handleBrowseFiles}
                      onSelectFolder={handleSelectFolder}
                    />
                  </div>
                ) : (
                  <div className="flex flex-col h-full bg-black/20 backdrop-blur-md">
                    <div className="px-8 py-6 border-b border-white/5 bg-zinc-950/40 shrink-0 space-y-2">
                      <StatsBar
                        stats={stats}
                        elapsed={timer.elapsed}
                        formatted={timer.formatted}
                        isComplete={isComplete}
                      />
                      <GlobalProgress progress={progress} isComplete={isComplete} />
                    </div>

                    <div className="flex-1 flex gap-6 p-6 overflow-hidden min-h-0">
                      <div className="w-[350px] shrink-0 bg-zinc-900/50 rounded-2xl border border-white/5 overflow-hidden">
                        <FileList files={files} selected={selected} onSelect={selectFile} />
                      </div>
                      <div className="flex-1 min-w-0 bg-black/40 rounded-3xl border border-white/10 overflow-hidden shadow-2xl relative">
                        <PreviewPanel file={selectedFile} allFiles={files} />
                      </div>
                    </div>

                    <div className="px-8 py-5 border-t border-white/5 bg-zinc-950/60 flex items-center justify-between shrink-0">
                      <div className="flex items-center gap-2">
                        {isComplete ? (
                          <button
                            onClick={handleNewBatch}
                            className="flex items-center gap-2 bg-zinc-800 hover:bg-zinc-700 transition-colors px-6 py-2.5 rounded-xl border border-white/10 text-sm font-medium"
                          >
                            <RotateCcw size={16} />
                            New Batch
                          </button>
                        ) : (
                          <button
                            onClick={handleBrowseFiles}
                            className="flex items-center gap-2 bg-zinc-800 hover:bg-zinc-700 transition-colors px-6 py-2.5 rounded-xl border border-white/10 text-sm font-medium"
                          >
                            <Plus size={18} />
                            Add FITS
                          </button>
                        )}
                      </div>

                      <DownloadButton
                        files={files}
                        onExport={exportZip}
                        isExporting={isExporting}
                        progress={zipProgress}
                        downloaded={downloaded}
                        doneCount={stats.done}
                        isComplete={isComplete}
                      />
                    </div>
                  </div>
                )}

                <div className="absolute bottom-6 left-8 pointer-events-none flex items-center gap-3 select-none z-20">
                  <AstroLogo size={32} showText={false} className="opacity-40" />
                  <div className="flex flex-col border-l border-white/10 pl-3">
                    <span className="text-[10px] font-bold tracking-widest text-zinc-500 uppercase">
                      AstroBurst
                    </span>
                    <span className="text-[9px] font-mono text-blue-500/40 uppercase leading-none">
                      v0.1.0
                    </span>
                  </div>
                </div>
              </DropZone>
            </div>
          )}
        </AnimatePresence>
      </div>
    </ErrorBoundary>
  );
}
