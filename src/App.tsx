import { useState, useCallback, useEffect, useRef, useMemo, memo, useSyncExternalStore } from "react";
import { Plus, RotateCcw } from "lucide-react";

import DropZone from "./components/file/DropZone";
import EmptyState from "./components/EmptyState";
import MetadataFileList from "./components/file/MetadataFileList";
import type { MetadataFile } from "./components/file/MetadataFileList";
import PreviewPanel from "./components/PreviewPanel";

import Confetti from "./components/Confetti";
import ErrorBoundary from "./components/ErrorBoundary";
import { AstroLogo } from "./components/AstroLogo";

import { useFileQueue } from "./hooks/useFileQueue";
import { useFileStats, useFileIds, useSelectedId, fileStore } from "./hooks/useFileStore";
import { useTimer } from "./hooks/useTimer";
import { useZipExport } from "./hooks/useZipExport";
import { isValidFitsFile } from "./utils/validation";
import { useActiveFilters, useFilterMode, useProductFilterActions, useProductFilterState, detectProductTypes, matchesActiveFilters } from "./hooks/useProductFilter";

import type { AstroFile, ProcessedFile } from "./shared/types";
import { APP_VERSION, FILE_STATUS } from "./utils/constants";

// @ts-ignore
import nebulaImg from "./assets/nebulosa.jpg";
import GlobalProgress from "./components/file/GlobalProgress";
import StatsBar from "./components/analysis/StatsBar";
import { isTauri } from "./infrastructure/tauri";

type ViewState = "empty" | "processing" | "complete";

const MemoizedPreviewPanel = memo(PreviewPanel);

const SIDEBAR_MIN = 42;
const SIDEBAR_DEFAULT = 300;
const SIDEBAR_MAX = 480;

function toMetadataFiles(fileIds: string[], getFile: (id: string) => ProcessedFile | undefined): MetadataFile[] {
  return fileIds.map((id) => {
    const f = getFile(id);
    if (!f) return { id, name: "Unknown", path: "", size: 0, status: "queued" as const };
    const header = f.result?.header;
    return {
      id: f.id,
      name: f.name,
      path: f.path,
      size: f.size ?? 0,
      status: (f.status ?? FILE_STATUS.QUEUED) as MetadataFile["status"],
      error: f.error,
      metadata: header
        ? {
          filter: header.FILTER ?? undefined,
          exptime: header.EXPTIME != null ? Number(header.EXPTIME) : undefined,
          instrument: header.INSTRUME ?? undefined,
          detector: header.DETECTOR ?? undefined,
          bitpix: header.BITPIX != null ? Number(header.BITPIX) : undefined,
          dateObs: header["DATE-OBS"] ?? undefined,
        }
        : undefined,
      previewUrl: f.result?.previewUrl,
      dimensions: f.result?.dimensions,
      elapsed_ms: f.result?.elapsed_ms,
    };
  });
}

export default function App() {
  const [loading, setLoading] = useState(true);
  const [view, setView] = useState<ViewState>("empty");
  const [showConfetti, setShowConfetti] = useState(false);
  const prevCompleteRef = useRef(false);

  const [sidebarOpen, setSidebarOpen] = useState(true);
  const sidebarWidthRef = useRef(SIDEBAR_DEFAULT);
  const sidebarResizing = useRef(false);
  const sidebarStartX = useRef(0);
  const sidebarStartW = useRef(0);
  const sidebarElRef = useRef<HTMLDivElement>(null);
  const [, forceSidebarRender] = useState(0);

  const { addFiles, startProcessing, scheduleProcessing, reset } = useFileQueue();
  const { stats, isProcessing, isComplete, progress } = useFileStats();
  const fileIds = useFileIds();
  const selectedId = useSelectedId();

  const timer = useTimer();
  const { exportZip, progress: zipProgress, isExporting, downloaded } = useZipExport();

  const activeFilters = useActiveFilters();
  const filterMode = useFilterMode();
  const filterState = useProductFilterState();
  const { toggleFilter, toggleMode, clearAll, addCustomChip, removeCustomChip, reset: resetProductFilter } = useProductFilterActions();

  const [showBg, setShowBg] = useState(false);

  useEffect(() => { const t = setTimeout(() => setLoading(false), 600); return () => clearTimeout(t); }, []);
  useEffect(() => { if (!loading) { const t = setTimeout(() => setShowBg(true), 100); return () => clearTimeout(t); } }, [loading]);

  const handleFilesAdded = useCallback((newFiles: AstroFile[]) => {
    if (newFiles.length === 0) return;
    addFiles(newFiles);
    setView((v) => (v === "empty" || v === "complete") ? "processing" : v);
    scheduleProcessing();
  }, [addFiles, scheduleProcessing]);

  useEffect(() => {
    if (view === "processing" && stats.total > 0 && !isProcessing && !isComplete) {
      startProcessing(() => timer.start(), () => timer.stop());
    }
  }, [view, stats.total, isProcessing, isComplete, startProcessing, timer]);

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
        const result = await open({ multiple: true, filters: [{ name: "FITS", extensions: ["fits", "fit", "fts", "asdf"] }] });
        if (result) {
          const paths = Array.isArray(result) ? result : [result];
          handleFilesAdded(paths.map((p: string) => ({ name: p.split(/[/\\]/).pop() || "Unknown", path: p, size: 0 })));
        }
      } catch (err) { console.error("[AstroBurst] File dialog error:", err); }
    } else {
      const input = document.createElement("input");
      input.type = "file"; input.multiple = true; input.accept = ".fits,.fit,.fts,.asdf";
      input.onchange = (e: any) => {
        const list = Array.from(e.target.files as FileList).filter((f) => isValidFitsFile(f.name)).map((f) => ({ name: f.name, path: f.name, size: f.size }));
        if (list.length > 0) handleFilesAdded(list);
      };
      input.click();
    }
  }, [handleFilesAdded]);

  const handleSelectFolder = useCallback(async () => {
    if (!isTauri()) { handleBrowseFiles(); return; }
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const { readDir } = await import("@tauri-apps/plugin-fs");
      const result = await open({ directory: true, multiple: false, title: "Select FITS Folder" });
      const dir = typeof result === "string" ? result : null;
      if (!dir) return;
      const entries = await readDir(dir);
      const fitsFiles: AstroFile[] = [];
      for (const entry of entries) {
        const name = entry.name || "";
        if (isValidFitsFile(name) && !entry.isDirectory) {
          const sep = dir.includes("\\") ? "\\" : "/";
          fitsFiles.push({ name, path: `${dir}${sep}${name}`, size: 0 });
        }
      }
      if (fitsFiles.length > 0) handleFilesAdded(fitsFiles);
    } catch (err) { console.error("[AstroBurst] Folder dialog error:", err); }
  }, [handleFilesAdded, handleBrowseFiles]);

  const handleNewBatch = useCallback(() => {
    reset();
    timer.reset();
    resetProductFilter();
    setView("empty");
    setShowConfetti(false);
  }, [reset, timer, resetProductFilter]);

  const handleSelectFile = useCallback((id: string) => {
    fileStore.selectFile(id);
  }, []);

  const handleExportZip = useCallback(() => {
    exportZip(fileStore.getFiles());
  }, [exportZip]);

  const storeVersion = useSyncExternalStore(fileStore.subscribe, fileStore.getVersion);
  const metadataFiles = useMemo(
    () => toMetadataFiles(fileIds, (id) => fileStore.getFile(id)),
    [fileIds, storeVersion],
  );

  const productTypes = useMemo(
    () => detectProductTypes(metadataFiles.map((f) => f.name)),
    [metadataFiles],
  );

  const filteredMetadataFiles = useMemo(() => {
    if (activeFilters.length === 0) return metadataFiles;
    return metadataFiles.filter((f) => matchesActiveFilters(f.name, activeFilters, filterMode));
  }, [metadataFiles, activeFilters, filterMode]);

  const filteredSelectedId = useMemo(() => {
    if (!selectedId) return null;
    if (activeFilters.length === 0) return selectedId;
    const exists = filteredMetadataFiles.some((f) => f.id === selectedId);
    if (exists) return selectedId;
    const firstDone = filteredMetadataFiles.find((f) => f.status === "done");
    return firstDone?.id ?? null;
  }, [selectedId, activeFilters, filteredMetadataFiles]);

  useEffect(() => {
    if (filteredSelectedId !== null && filteredSelectedId !== selectedId) {
      fileStore.selectFile(filteredSelectedId);
    }
  }, [filteredSelectedId, selectedId]);

  const handleSidebarResizeStart = useCallback((e: React.MouseEvent) => {
    if (!sidebarOpen) return;
    e.preventDefault();
    sidebarResizing.current = true;
    sidebarStartX.current = e.clientX;
    sidebarStartW.current = sidebarWidthRef.current;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    const el = sidebarElRef.current;
    const onMove = (ev: MouseEvent) => {
      if (!sidebarResizing.current) return;
      const next = Math.max(180, Math.min(SIDEBAR_MAX, sidebarStartW.current + (ev.clientX - sidebarStartX.current)));
      sidebarWidthRef.current = next;
      if (el) el.style.width = `${next}px`;
    };
    const onUp = () => {
      sidebarResizing.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      forceSidebarRender((c) => c + 1);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, [sidebarOpen]);

  const effectiveSidebarW = sidebarOpen ? sidebarWidthRef.current : SIDEBAR_MIN;

  return (
    <ErrorBoundary>
      <div className="relative h-screen w-full text-zinc-100 overflow-hidden" style={{ background: "var(--ab-deep)" }}>
        {showConfetti && <Confetti show />}
        <div
          className="fixed inset-0 z-0 opacity-40 pointer-events-none"
          style={{
            backgroundImage: showBg ? `url(${nebulaImg})` : "none", backgroundSize: "cover", backgroundPosition: "center",
            filter: view !== "empty" ? "blur(8px) brightness(0.3)" : "none", transition: "filter 0.6s ease",
          }}
        />
        {loading ? (
          <div className="relative z-50 h-screen flex flex-col items-center justify-center animate-fade-in" style={{ background: "var(--ab-deep)" }}>
            <AstroLogo size={80} showText={false} className="animate-pulse" />
            <h1 className="mt-6 text-xl tracking-[0.5em] uppercase cosmic-text">AstroBurst</h1>
          </div>
        ) : (
          <div className="relative z-10 h-full animate-fade-in">
            <DropZone onFilesAdded={handleFilesAdded}>
              {view === "empty" ? (
                <div className="h-full flex items-center justify-center">
                  <EmptyState onBrowseFiles={handleBrowseFiles} onSelectFolder={handleSelectFolder} />
                </div>
              ) : (
                <div className="flex flex-col h-full">
                  <div
                    className="px-4 py-2 shrink-0 space-y-1.5"
                    style={{
                      background: "linear-gradient(90deg, rgba(20,184,166,0.03) 0%, rgba(5,5,16,0.65) 50%, rgba(59,130,246,0.03) 100%)",
                      borderBottom: "1px solid rgba(20,184,166,0.1)",
                    }}
                  >
                    <StatsBar stats={stats} elapsed={timer.elapsed} formatted={timer.formatted} isComplete={isComplete} />
                    <GlobalProgress progress={progress} isComplete={isComplete} />
                  </div>

                  <div className="flex-1 flex overflow-hidden min-h-0">
                    <div
                      ref={sidebarElRef}
                      className="shrink-0 flex flex-col overflow-hidden"
                      style={{
                        width: effectiveSidebarW,
                        transition: sidebarResizing.current ? "none" : "width 0.15s ease-out",
                        borderRight: "1px solid rgba(20,184,166,0.08)",
                        background: "rgba(5,5,16,0.55)",
                      }}
                    >
                      <MetadataFileList
                        files={filteredMetadataFiles}
                        totalFiles={metadataFiles.length}
                        selectedId={selectedId}
                        onSelect={handleSelectFile}
                        onExportZip={handleExportZip}
                        collapsed={!sidebarOpen}
                        onToggle={() => setSidebarOpen((p) => !p)}
                        isExporting={isExporting}
                        zipProgress={zipProgress}
                        downloaded={downloaded}
                        productTypes={productTypes}
                        customChips={filterState.customChips}
                        activeFilters={activeFilters}
                        filterMode={filterMode}
                        onToggleFilter={toggleFilter}
                        onToggleMode={toggleMode}
                        onClearFilters={clearAll}
                        onAddCustomChip={addCustomChip}
                        onRemoveCustomChip={removeCustomChip}
                      />
                    </div>

                    {sidebarOpen && (
                      <div className="ab-resize-handle" onMouseDown={handleSidebarResizeStart} />
                    )}

                    <div className="flex-1 min-w-0 flex flex-col overflow-hidden">
                      <MemoizedPreviewPanel />
                    </div>
                  </div>

                  <div
                    className="px-4 py-1.5 flex items-center justify-between shrink-0"
                    style={{ borderTop: "1px solid rgba(20,184,166,0.06)", background: "rgba(5,5,16,0.6)" }}
                  >
                    <div className="flex items-center gap-3">
                      <div className="flex items-center gap-2 pointer-events-auto select-none">
                        <AstroLogo size={16} showText={false} className="opacity-30" />
                        <span className="text-[8px] font-bold tracking-widest uppercase cosmic-text" style={{ opacity: 0.5 }}>AstroBurst</span>
                        <span className="text-[7px] font-mono uppercase" style={{ color: "rgba(20,184,166,0.2)" }}>{APP_VERSION}</span>
                      </div>
                      <div className="w-px h-3" style={{ background: "rgba(20,184,166,0.08)" }} />
                      {isComplete ? (
                        <button
                          onClick={handleNewBatch}
                          className="flex items-center gap-1 transition-all duration-200 px-2 py-1 rounded text-[10px] font-medium"
                          style={{ background: "rgba(20,184,166,0.06)", border: "1px solid rgba(20,184,166,0.15)", color: "#a1a1aa" }}
                        >
                          <RotateCcw size={10} /> New Batch
                        </button>
                      ) : (
                        <button
                          onClick={handleBrowseFiles}
                          className="flex items-center gap-1 transition-all duration-200 px-2 py-1 rounded text-[10px] font-medium"
                          style={{ background: "rgba(20,184,166,0.06)", border: "1px solid rgba(20,184,166,0.1)", color: "#a1a1aa" }}
                        >
                          <Plus size={11} /> Add FITS
                        </button>
                      )}
                    </div>
                  </div>
                </div>
              )}

            </DropZone>
          </div>
        )}
      </div>
    </ErrorBoundary>
  );
}
