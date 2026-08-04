import { useState, useCallback, useEffect, useRef, useMemo, memo, useSyncExternalStore } from "react";
import {
  Plus, RotateCcw, FolderOpen, Layers, Info as InfoIcon, X, Search,
  FileText, BarChart3, Sparkles, Layers2, FlaskConical, Download, Settings, PanelLeftClose,
} from "lucide-react";

import DropZone from "./components/file/DropZone";
import EmptyState from "./components/EmptyState";
import MetadataFileList from "./components/file/MetadataFileList";
import type { MetadataFile } from "./components/file/MetadataFileList";
import PreviewPanel, { type ToolId } from "./components/PreviewPanel";
import { InfoPanel } from "./components/file/SidebarPanels";

import Confetti from "./components/Confetti";
import ErrorBoundary from "./components/ErrorBoundary";
import { AstroLogo } from "./components/AstroLogo";

import { useFileQueue } from "./hooks/useFileQueue";
import { registerFileIngest } from "./hooks/useFileIngest";
import { useFileStats, useFileIds, useSelectedId, fileStore, useSelectedFile, useDoneFiles } from "./hooks/useFileStore";
import { useZipExport } from "./hooks/useZipExport";
import { isValidFitsFile } from "./utils/validation";
import { useActiveFilters, useFilterMode, useProductFilterActions, useProductFilterState, detectProductTypes, matchesActiveFilters } from "./hooks/useProductFilter";

import type { AstroFile, ProcessedFile } from "./shared/types";
import { APP_VERSION, FILE_STATUS } from "./utils/constants";
import { CompositeProvider } from "./context/CompositeContext";
import { ComposeWizardProvider } from "./context/ComposeWizardContext";
import { PreviewProvider } from "./context/PreviewContext";

import { loadLayout, saveLayout } from "./utils/layout";
import CommandPalette, { type PaletteAction, type PaletteFile } from "./components/CommandPalette";
import { useRightTool, rightToolStore, RIGHT_TOOLS, type RightToolId } from "./hooks/useRightTool";
import nebulaImg from "./assets/nebulosa.jpg";
import GlobalProgress from "./components/file/GlobalProgress";
import StatsBar from "./components/analysis/StatsBar";
import { isTauri } from "./infrastructure/tauri";

type ViewState = "empty" | "processing" | "complete";

const MemoizedPreviewPanel = memo(PreviewPanel);

const SIDEBAR_DEFAULT = 300;
const SIDEBAR_MIN = 180;
const SIDEBAR_MAX = 480;

const LEFT_TABS: { id: "files"; label: string; icon: typeof FolderOpen }[] = [
  { id: "files", label: "Files", icon: FolderOpen },
];

const TOOL_ICONS: Record<RightToolId, typeof FileText> = {
  headers: FileText,
  analysis: BarChart3,
  processing: Sparkles,
  stacking: Layers2,
  synth: FlaskConical,
  export: Download,
  config: Settings,
};

function toMetadataFiles(
  fileIds: string[],
  getFile: (id: string) => ProcessedFile | undefined,
  cache: WeakMap<ProcessedFile, MetadataFile>,
): MetadataFile[] {
  return fileIds.map((id) => {
    const f = getFile(id);
    if (!f) return { id, name: "Unknown", path: "", size: 0, status: "queued" as const };
    const hit = cache.get(f);
    if (hit) return hit;
    const header = f.result?.header;
    const built: MetadataFile = {
      id: f.id,
      name: f.name,
      path: f.path,
      size: f.size ?? 0,
      status: (f.status ?? FILE_STATUS.QUEUED) as MetadataFile["status"],
      error: f.error ?? undefined,
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
    cache.set(f, built);
    return built;
  });
}

export default function App() {
  const [loading, setLoading] = useState(true);
  const [view, setView] = useState<ViewState>("empty");
  const [showConfetti, setShowConfetti] = useState(false);
  const prevCompleteRef = useRef(false);

  const [sidebarOpen, setSidebarOpen] = useState(true);
  const sidebarWidthRef = useRef(loadLayout("sidebarW", SIDEBAR_DEFAULT, SIDEBAR_MIN, SIDEBAR_MAX));
  const sidebarResizing = useRef(false);
  const sidebarStartX = useRef(0);
  const sidebarStartW = useRef(0);
  const sidebarElRef = useRef<HTMLDivElement>(null);
  const sidebarInnerRef = useRef<HTMLDivElement>(null);
  const [, forceSidebarRender] = useState(0);

  const [activeTool, setActiveTool] = useState<ToolId | null>("compose");
  const handleToggleTool = useCallback((toolId: ToolId) => {
    setActiveTool((prev) => (prev === toolId ? null : toolId));
  }, []);
  const [infoOpen, setInfoOpen] = useState(false);

  const { addFiles, startProcessing, scheduleProcessing, reset, isResampling, resampleProgress } = useFileQueue();
  const { stats, isProcessing, isComplete, progress } = useFileStats();
  const fileIds = useFileIds();
  const selectedId = useSelectedId();
  const selectedFile = useSelectedFile();
  const allDoneFiles = useDoneFiles();

  const { exportZip, progress: zipProgress, isExporting, downloaded } = useZipExport();

  const activeFilters = useActiveFilters();
  const filterMode = useFilterMode();
  const filterState = useProductFilterState();
  const { toggleFilter, toggleMode, clearAll, addCustomChip, removeCustomChip, reset: resetProductFilter } = useProductFilterActions();

  const [showBg, setShowBg] = useState(false);

  const filteredDoneFiles = useMemo(() => {
    if (activeFilters.length === 0) return allDoneFiles;
    return allDoneFiles.filter((f) => matchesActiveFilters(f.name, activeFilters, filterMode));
  }, [allDoneFiles, activeFilters, filterMode]);

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
      startProcessing();
    }
  }, [view, stats.total, isProcessing, isComplete, startProcessing]);

  useEffect(() => registerFileIngest(handleFilesAdded), [handleFilesAdded]);

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
        const result = await open({ multiple: true, filters: [{ name: "FITS", extensions: ["fits", "fit", "fts", "asdf", "zip"] }] });
        if (result) {
          const paths = Array.isArray(result) ? result : [result];
          handleFilesAdded(paths.map((p: string) => ({ name: p.split(/[/\\]/).pop() || "Unknown", path: p, size: 0 })));
        }
      } catch (err) { console.error("[AstroBurst] File dialog error:", err); }
    } else {
      const input = document.createElement("input");
      input.type = "file"; input.multiple = true; input.accept = ".fits,.fit,.fts,.asdf,.zip";
      input.onchange = (e: Event) => {
        const files = (e.target as HTMLInputElement).files;
        if (!files) return;
        const list = Array.from(files).filter((f) => isValidFitsFile(f.name)).map((f) => ({ name: f.name, path: f.name, size: f.size }));
        if (list.length > 0) handleFilesAdded(list);
      };
      input.click();
    }
  }, [handleFilesAdded]);

  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const lastShiftUp = useRef(0);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && (e.key === "o" || e.key === "O")) {
        e.preventDefault();
        handleBrowseFiles();
        return;
      }
      if ((e.ctrlKey || e.metaKey) && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        setPaletteOpen((p) => !p);
        return;
      }
      if (e.key !== "Shift") lastShiftUp.current = 0;
      const target = e.target as HTMLElement | null;
      const inInput = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || (target?.isContentEditable ?? false);
      if (inInput) return;
      if (e.key === "?") {
        e.preventDefault();
        setShortcutsOpen((p) => !p);
        return;
      }
      if (e.key === "Escape") {
        setShortcutsOpen(false);
        setPaletteOpen(false);
      }
    };
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.key !== "Shift") return;
      const now = Date.now();
      if (now - lastShiftUp.current < 350) {
        lastShiftUp.current = 0;
        setPaletteOpen((p) => !p);
      } else {
        lastShiftUp.current = now;
      }
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [handleBrowseFiles]);

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
    resetProductFilter();
    setView("empty");
    setShowConfetti(false);
    setActiveTool("compose");
    setInfoOpen(false);
  }, [reset, resetProductFilter]);

  const [confirmNewBatch, setConfirmNewBatch] = useState(false);
  const confirmNewBatchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const handleNewBatchClick = useCallback(() => {
    if (stats.done === 0 || confirmNewBatch) {
      if (confirmNewBatchTimerRef.current) clearTimeout(confirmNewBatchTimerRef.current);
      setConfirmNewBatch(false);
      handleNewBatch();
      return;
    }
    setConfirmNewBatch(true);
    confirmNewBatchTimerRef.current = setTimeout(() => setConfirmNewBatch(false), 4000);
  }, [stats.done, confirmNewBatch, handleNewBatch]);

  const handleSidebarToggle = useCallback(() => setSidebarOpen((p) => !p), []);

  const handleSelectFile = useCallback((id: string) => {
    fileStore.selectFile(id);
  }, []);

  const handleExportZip = useCallback(() => {
    exportZip(fileStore.getFiles());
  }, [exportZip]);

  const storeVersion = useSyncExternalStore(fileStore.subscribe, fileStore.getVersion);
  const metaCacheRef = useRef<WeakMap<ProcessedFile, MetadataFile>>(new WeakMap());
  const metadataFiles = useMemo(
    () => toMetadataFiles(fileIds, (id) => fileStore.getFile(id), metaCacheRef.current),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [fileIds, storeVersion],
  );

  const productTypes = useMemo(
    () => detectProductTypes(fileIds.map((id) => fileStore.getFile(id)?.name ?? "")),
    [fileIds],
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
    const handle = e.currentTarget as HTMLElement;
    handle.dataset.dragging = "true";
    const el = sidebarElRef.current;
    const inner = sidebarInnerRef.current;
    if (el) el.style.transition = "none";
    const onMove = (ev: MouseEvent) => {
      if (!sidebarResizing.current) return;
      const next = Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, sidebarStartW.current + (ev.clientX - sidebarStartX.current)));
      sidebarWidthRef.current = next;
      if (el) el.style.width = `${next}px`;
      if (inner) inner.style.width = `${next}px`;
    };
    const onUp = () => {
      sidebarResizing.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      delete handle.dataset.dragging;
      if (el) el.style.transition = "";
      saveLayout("sidebarW", sidebarWidthRef.current);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      forceSidebarRender((c) => c + 1);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, [sidebarOpen]);

  const handleSidebarResizeReset = useCallback(() => {
    sidebarWidthRef.current = SIDEBAR_DEFAULT;
    saveLayout("sidebarW", SIDEBAR_DEFAULT);
    const el = sidebarElRef.current;
    if (el) el.style.width = `${SIDEBAR_DEFAULT}px`;
    const inner = sidebarInnerRef.current;
    if (inner) inner.style.width = `${SIDEBAR_DEFAULT}px`;
    forceSidebarRender((c) => c + 1);
  }, []);

  const rightTool = useRightTool();
  const paletteActions = useMemo<PaletteAction[]>(() => {
    const acts: PaletteAction[] = [
      { id: "open-files", label: "Open FITS Files...", hint: "Ctrl+O", icon: FolderOpen, run: handleBrowseFiles },
      { id: "open-folder", label: "Open Folder...", icon: FolderOpen, run: handleSelectFolder },
    ];
    if (view !== "empty") {
      acts.push(
        { id: "toggle-sidebar", label: sidebarOpen ? "Hide Files Panel" : "Show Files Panel", icon: PanelLeftClose, run: () => setSidebarOpen((p) => !p) },
        { id: "toggle-compose", label: activeTool === "compose" ? "Hide Compose Panel" : "Show Compose Panel", icon: Layers, run: () => handleToggleTool("compose") },
      );
      for (const t of RIGHT_TOOLS) {
        acts.push({
          id: `tool-${t.id}`,
          label: rightTool === t.id ? `Hide ${t.label} Panel` : `Open ${t.label} Panel`,
          hint: "Tool",
          icon: TOOL_ICONS[t.id],
          run: () => rightToolStore.toggle(t.id),
        });
      }
      if (stats.done > 0) acts.push({ id: "export-zip", label: "Download ZIP of Processed Files", icon: Download, run: handleExportZip });
      if (isComplete) acts.push({ id: "new-batch", label: "New Batch (discard processed files)", icon: RotateCcw, run: handleNewBatch });
    }
    acts.push({ id: "shortcuts", label: "Keyboard Shortcuts", hint: "?", icon: InfoIcon, run: () => setShortcutsOpen(true) });
    return acts;
  }, [view, sidebarOpen, activeTool, rightTool, stats.done, isComplete, handleBrowseFiles, handleSelectFolder, handleToggleTool, handleExportZip, handleNewBatch]);

  const paletteFiles = useMemo<PaletteFile[]>(
    () => filteredMetadataFiles
      .filter((f) => f.status === "done")
      .map((f) => ({ id: f.id, name: f.name, filter: f.metadata?.filter })),
    [filteredMetadataFiles],
  );

  return (
    <ErrorBoundary>
      <div className="relative h-screen w-full text-zinc-100 overflow-hidden" style={{ background: "var(--ab-deep)" }}>
        {showConfetti && <Confetti show />}
        <CommandPalette
          open={paletteOpen}
          onClose={() => setPaletteOpen(false)}
          actions={paletteActions}
          files={paletteFiles}
          selectedId={selectedId}
          onSelectFile={handleSelectFile}
        />
        {shortcutsOpen && (
          <div
            className="fixed inset-0 z-[120] flex items-center justify-center bg-black/50 backdrop-blur-sm"
            onClick={() => setShortcutsOpen(false)}
          >
            <div
              className="rounded-lg p-4 min-w-[300px] animate-fade-in"
              style={{ background: "rgba(8,8,18,0.97)", border: "1px solid var(--ab-border-strong)", boxShadow: "0 8px 30px rgba(0,0,0,0.55)" }}
              onClick={(e) => e.stopPropagation()}
            >
              <div className="flex items-center justify-between mb-3">
                <span className="text-xs font-semibold text-zinc-300 uppercase tracking-wider">Keyboard Shortcuts</span>
                <button onClick={() => setShortcutsOpen(false)} title="Close" className="text-zinc-500 hover:text-zinc-300 transition-colors">
                  <X size={12} />
                </button>
              </div>
              <div className="flex flex-col gap-1.5 text-[11px]">
                {[
                  { keys: navigator.platform?.toLowerCase().includes("mac") ? "⌘O" : "Ctrl+O", desc: "Open FITS files" },
                  { keys: navigator.platform?.toLowerCase().includes("mac") ? "⌘K" : "Ctrl+K", desc: "Search everywhere" },
                  { keys: "Shift Shift", desc: "Search everywhere" },
                  { keys: "↑ / ↓", desc: "Navigate file list (when focused)" },
                  { keys: "Enter", desc: "Select focused file" },
                  { keys: "?", desc: "Toggle this help" },
                  { keys: "Esc", desc: "Close overlays" },
                ].map((s) => (
                  <div key={s.keys} className="flex items-center justify-between gap-6">
                    <span className="text-zinc-400">{s.desc}</span>
                    <kbd className="text-[10px] font-mono px-1.5 py-0.5 rounded border border-zinc-700 bg-zinc-900 text-zinc-300">{s.keys}</kbd>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}
        <div className="fixed inset-0 z-0 pointer-events-none">
          <div
            className="absolute inset-0"
            style={{
              backgroundImage: showBg ? `url(${nebulaImg})` : "none", backgroundSize: "cover", backgroundPosition: "center",
              opacity: view === "empty" ? 0.4 : 0, transition: "opacity 0.6s ease",
            }}
          />
          <div
            className="absolute inset-0"
            style={{
              backgroundImage: showBg ? `url(${nebulaImg})` : "none", backgroundSize: "cover", backgroundPosition: "center",
              filter: "blur(8px) brightness(0.3)",
              opacity: view !== "empty" ? 0.4 : 0, transition: "opacity 0.6s ease",
            }}
          />
        </div>
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
                <CompositeProvider>
                  <ComposeWizardProvider>
                    <PreviewProvider file={selectedFile} doneFiles={filteredDoneFiles}>
                      <div className="flex flex-col h-full">
                        <div
                          className="px-4 py-2 shrink-0 space-y-1.5"
                          style={{
                            background: "rgba(5,5,16,0.65)",
                            borderBottom: "1px solid var(--ab-border)",
                          }}
                        >
                          <StatsBar stats={stats} isProcessing={isProcessing} isComplete={isComplete} />
                          <GlobalProgress progress={progress} isComplete={isComplete} />
                          {isResampling && (
                            <div className="flex items-center gap-2 text-[10px] font-mono" style={{ color: "rgba(56,189,248,0.75)" }}>
                              <span className="w-2 h-2 rounded-full animate-spin" style={{ border: "1.5px solid transparent", borderTopColor: "rgba(56,189,248,0.75)" }} />
                              Matching resolutions… {resampleProgress}%
                            </div>
                          )}
                        </div>

                        <div className="flex-1 flex overflow-hidden min-h-0">
                          <div className="ab-left-strip shrink-0">
                            {LEFT_TABS.map((tab) => {
                              const Icon = tab.icon;
                              return (
                                <button
                                  key={tab.id}
                                  onClick={() => setSidebarOpen((p) => !p)}
                                  className={`ab-left-strip-btn ${sidebarOpen ? "ab-left-strip-btn-active" : ""}`}
                                  title={tab.label}
                                >
                                  <Icon size={14} />
                                  <span>{tab.label}</span>
                                </button>
                              );
                            })}
                            <div className="my-1 mx-2 h-px" style={{ background: "var(--ab-border)" }} />
                            <button
                              onClick={() => handleToggleTool("compose")}
                              className={`ab-left-strip-btn ${activeTool === "compose" ? "ab-left-strip-btn-active" : ""}`}
                              title="Compose"
                            >
                              <Layers size={14} />
                              <span>Comp</span>
                            </button>
                            <button
                              onClick={() => setInfoOpen((p) => !p)}
                              className={`ab-left-strip-btn ${infoOpen ? "ab-left-strip-btn-active" : ""}`}
                              title="Info"
                            >
                              <InfoIcon size={14} />
                              <span>Info</span>
                            </button>
                          </div>

                          <div
                            ref={sidebarElRef}
                            className="shrink-0 relative overflow-hidden ab-panel-anim-w"
                            style={{ width: sidebarOpen ? sidebarWidthRef.current : 0 }}
                            aria-hidden={!sidebarOpen}
                          >
                            <div
                              ref={sidebarInnerRef}
                              inert={!sidebarOpen}
                              className="absolute inset-y-0 right-0 flex flex-col overflow-hidden"
                              style={{
                                width: sidebarWidthRef.current,
                                borderRight: "1px solid var(--ab-border)",
                                background: "rgba(5,5,16,0.55)",
                              }}
                            >
                              <MetadataFileList
                                files={filteredMetadataFiles}
                                totalFiles={metadataFiles.length}
                                selectedId={selectedId}
                                onSelect={handleSelectFile}
                                onExportZip={handleExportZip}
                                collapsed={false}
                                onToggle={handleSidebarToggle}
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
                          </div>

                          {sidebarOpen && (
                            <div
                              className="ab-resize-handle"
                              onMouseDown={handleSidebarResizeStart}
                              onDoubleClick={handleSidebarResizeReset}
                              title="Drag to resize — double-click to reset"
                            />
                          )}

                          <div className="flex-1 min-w-0 flex flex-col overflow-hidden">
                            <MemoizedPreviewPanel activeTool={activeTool} />
                          </div>
                        </div>

                        <div
                          className="px-4 py-1.5 flex items-center justify-between shrink-0"
                          style={{ borderTop: "1px solid var(--ab-border)", background: "rgba(5,5,16,0.6)" }}
                        >
                          <div className="flex items-center gap-3">
                            <div className="flex items-center gap-2.5 pointer-events-auto select-none">
                              <AstroLogo size={22} showText={false} className="opacity-40" />
                              <span className="text-[10px] font-bold tracking-[0.25em] uppercase cosmic-text" style={{ opacity: 0.6 }}>AstroBurst</span>
                              <span className="text-[9px] font-mono uppercase" style={{ color: "rgba(20,184,166,0.55)" }}>{APP_VERSION}</span>
                            </div>
                            <div className="w-px h-3" style={{ background: "var(--ab-border)" }} />
                            {isComplete ? (
                              <button
                                onClick={handleNewBatchClick}
                                className="flex items-center gap-1 transition-all duration-200 px-2 py-1 rounded text-[10px] font-medium"
                                style={confirmNewBatch
                                  ? { background: "rgba(245,158,11,0.12)", border: "1px solid rgba(245,158,11,0.35)", color: "#fbbf24" }
                                  : { background: "rgba(255,255,255,0.03)", border: "1px solid var(--ab-border)", color: "#a1a1aa" }}
                              >
                                <RotateCcw size={10} />
                                {confirmNewBatch ? `Discard ${stats.done} processed file${stats.done === 1 ? "" : "s"}?` : "New Batch"}
                              </button>
                            ) : (
                              <button
                                onClick={handleBrowseFiles}
                                className="flex items-center gap-1 transition-all duration-200 px-2 py-1 rounded text-[10px] font-medium"
                                style={{ background: "rgba(255,255,255,0.03)", border: "1px solid var(--ab-border)", color: "#a1a1aa" }}
                              >
                                <Plus size={11} /> Add FITS
                              </button>
                            )}
                          </div>
                          <div className="flex items-center gap-1">
                            <button
                              onClick={() => setPaletteOpen(true)}
                              title="Search everywhere (Ctrl+K or double Shift)"
                              className="flex items-center gap-1.5 px-2 py-1 rounded text-[10px] transition-colors text-zinc-500 hover:text-zinc-300"
                              style={{ background: "transparent", border: "none" }}
                            >
                              <Search size={10} />
                              Search
                              <kbd className="ab-cmdp-kbd">Ctrl+K</kbd>
                            </button>
                            <button
                              onClick={() => setShortcutsOpen(true)}
                              title="Keyboard shortcuts (?)"
                              className="px-2 py-1 rounded text-[10px] transition-colors text-zinc-500 hover:text-zinc-300"
                              style={{ background: "transparent", border: "none" }}
                            >
                              <kbd className="ab-cmdp-kbd">?</kbd>
                            </button>
                          </div>
                        </div>

                        {infoOpen && (
                          <div
                            className="fixed z-50 rounded-lg overflow-hidden flex flex-col animate-fade-in"
                            style={{
                              left: 60,
                              bottom: 88,
                              width: 300,
                              maxHeight: "50vh",
                              border: "1px solid var(--ab-border-strong)",
                              background: "rgba(8,8,18,0.97)",
                              boxShadow: "0 8px 30px rgba(0,0,0,0.55)",
                              backdropFilter: "blur(8px)",
                            }}
                          >
                            <div className="flex items-center justify-between px-3 py-1.5 shrink-0" style={{ borderBottom: "1px solid var(--ab-border)" }}>
                              <span className="flex items-center gap-1.5 text-[11px] font-medium text-zinc-300">
                                <InfoIcon size={12} style={{ color: "var(--ab-teal)" }} /> Info
                              </span>
                              <button onClick={() => setInfoOpen(false)} title="Close" className="text-zinc-500 hover:text-zinc-300 transition-colors">
                                <X size={12} />
                              </button>
                            </div>
                            <div className="overflow-y-auto min-h-0">
                              <InfoPanel />
                            </div>
                          </div>
                        )}
                      </div>
                    </PreviewProvider>
                  </ComposeWizardProvider>
                </CompositeProvider>
              )}
            </DropZone>
          </div>
        )}
      </div>
    </ErrorBoundary>
  );
}
