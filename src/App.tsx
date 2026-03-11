import { useState, useCallback, useEffect, useRef, memo } from "react";
import { Plus, RotateCcw } from "lucide-react";

import DropZone from "./components/file/DropZone";
import EmptyState from "./components/EmptyState";
import FileList from "./components/file/FileList";
import PreviewPanel from "./components/PreviewPanel";

import Confetti from "./components/Confetti";
import ErrorBoundary from "./components/ErrorBoundary";
import { AstroLogo } from "./components/AstroLogo";

import { useFileQueue } from "./hooks/useFileQueue";
import { useTimer } from "./hooks/useTimer";
import { useZipExport } from "./hooks/useZipExport";
import { isValidFitsFile } from "./utils/validation";

import type { AstroFile } from "./utils/types";

import nebulaImg from "./assets/nebulosa.jpg";
import GlobalProgress from "./components/file/GlobalProgress";
import StatsBar from "./components/analysis/StatsBar";

const isTauri = (): boolean => !!(window as any).__TAURI_INTERNALS__;

type ViewState = "empty" | "processing" | "complete";

const MemoizedFileList = memo(FileList);
const MemoizedPreviewPanel = memo(PreviewPanel);

const SIDEBAR_MIN = 42;
const SIDEBAR_DEFAULT = 300;
const SIDEBAR_MAX = 480;

export default function App() {
  const [loading, setLoading] = useState(true);
  const [view, setView] = useState<ViewState>("empty");
  const [showConfetti, setShowConfetti] = useState(false);
  const prevCompleteRef = useRef(false);

  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [sidebarWidth, setSidebarWidth] = useState(SIDEBAR_DEFAULT);
  const sidebarResizing = useRef(false);
  const sidebarStartX = useRef(0);
  const sidebarStartW = useRef(0);

  const {
    files, selected, selectedFile, isProcessing, stats, progress, isComplete,
    addFiles, selectFile, startProcessing, reset, isResampling, resampleProgress,
  } = useFileQueue();

  const timer = useTimer();
  const { exportZip, progress: zipProgress, isExporting, downloaded } = useZipExport();

  const [showBg, setShowBg] = useState(false);

  useEffect(() => { const t = setTimeout(() => setLoading(false), 600); return () => clearTimeout(t); }, []);
  useEffect(() => { if (!loading) { const t = setTimeout(() => setShowBg(true), 100); return () => clearTimeout(t); } }, [loading]);

  const handleFilesAdded = useCallback((newFiles: AstroFile[]) => {
    if (newFiles.length === 0) return;
    addFiles(newFiles);
    if (view === "empty" || view === "complete") setView("processing");
  }, [addFiles, view]);

  useEffect(() => {
    if (view === "processing" && files.length > 0 && !isProcessing && !isComplete) {
      const queued = files.filter((f: any) => f.status === "queued");
      if (queued.length > 0) startProcessing(() => timer.start(), () => timer.stop());
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

  const handleNewBatch = useCallback(() => { reset(); timer.reset(); setView("empty"); setShowConfetti(false); }, [reset, timer]);

  const handleSidebarResizeStart = useCallback((e: React.MouseEvent) => {
    if (!sidebarOpen) return;
    e.preventDefault();
    sidebarResizing.current = true;
    sidebarStartX.current = e.clientX;
    sidebarStartW.current = sidebarWidth;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    const onMove = (ev: MouseEvent) => {
      if (!sidebarResizing.current) return;
      setSidebarWidth(Math.max(180, Math.min(SIDEBAR_MAX, sidebarStartW.current + (ev.clientX - sidebarStartX.current))));
    };
    const onUp = () => {
      sidebarResizing.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, [sidebarOpen, sidebarWidth]);

  const effectiveSidebarW = sidebarOpen ? sidebarWidth : SIDEBAR_MIN;

  return (
    <ErrorBoundary>
      <div className="relative h-screen w-full text-zinc-100 overflow-hidden" style={{ background: "var(--ab-deep)" }}>
        <Confetti show={showConfetti} />
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
                  {/* Top bar: stats + progress */}
                  <div
                    className="px-4 py-2 shrink-0 space-y-1.5"
                    style={{
                      background: "linear-gradient(90deg, rgba(20,184,166,0.03) 0%, rgba(5,5,16,0.65) 50%, rgba(59,130,246,0.03) 100%)",
                      borderBottom: "1px solid rgba(20,184,166,0.1)",
                    }}
                  >
                    <StatsBar stats={stats} elapsed={timer.elapsed} formatted={timer.formatted} isComplete={isComplete} />
                    <GlobalProgress progress={progress} isComplete={isComplete} />
                    {isResampling && (
                      <div className="flex items-center gap-2">
                        <span className="text-[11px] animate-pulse" style={{ color: "var(--ab-gold)" }}>Resampling... {resampleProgress}%</span>
                      </div>
                    )}
                  </div>

                  {/* Main area */}
                  <div className="flex-1 flex overflow-hidden min-h-0">
                    {/* Left sidebar: files + export */}
                    <div
                      className="shrink-0 flex flex-col overflow-hidden"
                      style={{
                        width: effectiveSidebarW,
                        transition: sidebarResizing.current ? "none" : "width 0.15s ease-out",
                        borderRight: "1px solid rgba(20,184,166,0.08)",
                        background: "rgba(5,5,16,0.55)",
                      }}
                    >
                      <MemoizedFileList
                        files={files}
                        selected={selected}
                        onSelect={selectFile}
                        collapsed={!sidebarOpen}
                        onToggle={() => setSidebarOpen((p) => !p)}
                        onExportZip={exportZip}
                        isExporting={isExporting}
                        zipProgress={zipProgress}
                        downloaded={downloaded}
                        doneCount={stats.done}
                        isComplete={isComplete}
                      />
                    </div>

                    {sidebarOpen && (
                      <div
                        className="w-[3px] shrink-0 cursor-col-resize"
                        style={{ background: "transparent" }}
                        onMouseDown={handleSidebarResizeStart}
                        onMouseEnter={(e) => (e.currentTarget.style.background = "rgba(20,184,166,0.3)")}
                        onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                      />
                    )}

                    {/* Center + right sidebar */}
                    <div className="flex-1 min-w-0 flex flex-col overflow-hidden">
                      <MemoizedPreviewPanel file={selectedFile} allFiles={files} />
                    </div>
                  </div>

                  {/* Footer: minimal */}
                  <div
                    className="px-4 py-1.5 flex items-center justify-between shrink-0"
                    style={{
                      borderTop: "1px solid rgba(20,184,166,0.06)",
                      background: "rgba(5,5,16,0.6)",
                    }}
                  >
                    <div className="flex items-center gap-3">
                      <div className="flex items-center gap-2 pointer-events-auto select-none">
                        <AstroLogo size={16} showText={false} className="opacity-30" />
                        <span className="text-[8px] font-bold tracking-widest uppercase cosmic-text" style={{ opacity: 0.5 }}>AstroBurst</span>
                        <span className="text-[7px] font-mono uppercase" style={{ color: "rgba(20,184,166,0.2)" }}>v0.3.0</span>
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
              <div className="absolute bottom-6 left-8 pointer-events-none flex items-center gap-3 select-none z-20">
                <AstroLogo size={32} showText={false} className="opacity-40" />
                <div className="flex flex-col border-l border-white/10 pl-3">
                    <span className="text-[10px] font-bold tracking-widest text-zinc-500 uppercase">
                      AstroBurst
                    </span>
                  <span className="text-[9px] font-mono text-blue-500/40 uppercase leading-none">
                      v0.3.1
                    </span>
                </div>
              </div>
            </DropZone>
          </div>
        )}
      </div>
    </ErrorBoundary>
  );
}
