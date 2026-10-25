import { useState, useCallback, useEffect, useRef, useMemo, memo, useSyncExternalStore, useId, lazy, Suspense, type CSSProperties } from "react";
import { Plus, RotateCcw, FolderOpen, Info as InfoIcon, X, Download, Columns2, Sparkles, Undo2, Redo2, Activity } from "lucide-react";
import { historyStore } from "./utils/historyStore";

import DropZone from "./components/file/DropZone";
import EmptyState from "./components/EmptyState";
import AppFooter from "./components/AppFooter";
import { appModeStore, modeFromShortcut, useAppMode } from "./hooks/useAppMode";
import { useArchivePaletteActions } from "./components/archive/state/useArchivePaletteActions";
import { workspaceFileRef } from "./components/archive/state/useAroundTarget";
import DisplayedImageBridge from "./components/archive/DisplayedImageBridge";
import { shortcutSheet, type ShortcutRow } from "./utils/shortcutSheet";
import MetadataFileList from "./components/file/MetadataFileList";
import type { MetadataFile } from "./components/file/MetadataFileList";
import PreviewPanel from "./components/PreviewPanel";
import DockShell from "./components/dock/DockShell";
import { DOCK_TOOLS } from "./components/dock/toolRegistry";
import { dockOnboardingStore } from "./components/dock/dockOnboardingStore";

import Confetti from "./components/Confetti";
import ErrorBoundary from "./components/ErrorBoundary";
import { AstroLogo } from "./components/AstroLogo";

import { useFileQueue } from "./hooks/useFileQueue";
import { followFilteredSelection, registerFileIngest, showWorkspaceUnlessKept, type IngestOptions } from "./hooks/useFileIngest";
import { quietCompletion } from "./utils/quietCompletion";
import { useFileStats, useFileIds, useSelectedId, fileStore, useSelectedFile, useDoneFiles } from "./hooks/useFileStore";
import { useZipExport } from "./hooks/useZipExport";
import { SUPPORTED_EXTENSIONS, astroFileFromPath, dialogExtensions, folderErrorReport, folderReport, partitionIncoming, rejectionReport, type IngestReport } from "./utils/validation";
import { useActiveFilters, useFilterMode, useProductFilterActions, useProductFilterState, detectProductTypes, filtersRevealing, matchesActiveFilters, metadataFilterable, processedFilterable, productFilterStore } from "./hooks/useProductFilter";
import { displayFilterValue } from "./utils/channelMapping";
import { FOCUSABLE_SELECTOR, focusTrapTarget } from "./utils/focusTrap";
import { comparableX1dIds, findSiblingCube } from "./utils/x1dCompare";
import { spectrumComparisonStore } from "./hooks/useSpectrumComparisonStore";

import type { AstroFile, ProcessedFile } from "./shared/types";
import { FILE_STATUS } from "./utils/constants";
import { fallbackSelectionId, selectableRows } from "./utils/fileSelection";
import { CompositeProvider } from "./context/CompositeContext";
import { ComposeWizardProvider } from "./context/ComposeWizardContext";
import { PreviewProvider, fileKeyOf } from "./context/PreviewContext";
import { openSpectrumCsv, spectrumCsvStore } from "./hooks/useSpectrum1dStore";
import { openSpectrumCsvPaletteAction } from "./utils/spectrumDocument";

import { terminateStfWorker } from "./utils/stfworker";
import CommandPalette, { type PaletteAction, type PaletteFile } from "./components/CommandPalette";
import { DOCK_TOOL_META, paletteToolEntries } from "./utils/dockLayout";
import { dockStore, useDockLayout } from "./hooks/useDockLayout";
import nebulaImg from "./assets/nebulosa.jpg";
import GlobalProgress from "./components/file/GlobalProgress";
import StatsBar from "./components/analysis/StatsBar";
import { useLingeringVisibility } from "./hooks/useLingeringVisibility";
import { isTauri } from "./infrastructure/tauri";
import { sampleDataStore, useSampleData } from "./hooks/useSampleData";
import { openSampleProducts, paletteSampleSetsSelector, samplePaletteActions, sampleSetRun } from "./utils/sampleOffers";
import type { SampleOpen } from "./services/sampleData";

type ViewState = "empty" | "processing" | "complete";

const MemoizedPreviewPanel = memo(PreviewPanel);
const ArchiveRoot = lazy(() => import("./components/archive/ArchiveRoot"));
const ArchiveFooterStatus = lazy(() => import("./components/archive/ArchiveFooterStatus"));
const HIDDEN_LAYER: CSSProperties = { visibility: "hidden" };
const showWorkspace = () => appModeStore.set("workspace");
const showArchive = () => appModeStore.set("archive");
const showCubeTool = () => {
  showWorkspace();
  dockStore.dispatch({ type: "open", tool: "cube" });
};
const PALETTE_SAMPLE_SETS = paletteSampleSetsSelector(sampleDataStore);
const IS_MAC = typeof navigator !== "undefined" && (navigator.platform?.toLowerCase().includes("mac") ?? false);
const SHORTCUT_GROUPS = shortcutSheet(IS_MAC).filter((g) => g.rows.length > 0);

function ShortcutRows({ rows }: { rows: readonly ShortcutRow[] }) {
  return (
    <div className="flex flex-col gap-1.5 text-[11px]">
      {rows.map((s) => (
        <div key={s.keys} className="flex items-center justify-between gap-6">
          <span className="text-zinc-400">{s.desc}</span>
          <kbd className="shrink-0 text-[10px] font-mono px-1.5 py-0.5 rounded border border-zinc-700 bg-zinc-900 text-zinc-300">{s.keys}</kbd>
        </div>
      ))}
    </div>
  );
}

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
      spectrum: f.spectrum,
      metadata: header
        ? {
          filter: displayFilterValue(f) ?? undefined,
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
  const quietCompletionRef = useRef(false);
  const dockLayout = useDockLayout();
  const mode = useAppMode();
  const [archiveMounted, setArchiveMounted] = useState(mode === "archive");
  if (mode === "archive" && !archiveMounted) setArchiveMounted(true);

  const { addFiles, startProcessing, scheduleProcessing, reset, isResampling, resampleProgress } = useFileQueue();
  const { stats, isProcessing, isComplete, progress } = useFileStats();
  const progressStripVisible = useLingeringVisibility(isProcessing || isResampling || !isComplete);
  const fileIds = useFileIds();
  const selectedId = useSelectedId();
  const selectedFile = useSelectedFile();
  const allDoneFiles = useDoneFiles();

  const { exportZip, progress: zipProgress, isExporting, downloaded, error: zipError } = useZipExport();

  const activeFilters = useActiveFilters();
  const filterMode = useFilterMode();
  const filterState = useProductFilterState();
  const { toggleFilter, toggleMode, clearAll, addCustomChip, removeCustomChip, reset: resetProductFilter } = useProductFilterActions();

  const [showBg, setShowBg] = useState(false);

  const filteredDoneFiles = useMemo(() => {
    if (activeFilters.length === 0) return allDoneFiles;
    return allDoneFiles.filter((f) => matchesActiveFilters(processedFilterable(f), activeFilters, filterMode));
  }, [allDoneFiles, activeFilters, filterMode]);

  useEffect(() => { const t = setTimeout(() => setLoading(false), 600); return () => clearTimeout(t); }, []);
  useEffect(() => { if (!loading) { const t = setTimeout(() => setShowBg(true), 100); return () => clearTimeout(t); } }, [loading]);

  const [ingestReport, setIngestReport] = useState<IngestReport | null>(null);

  const handleFilesAdded = useCallback((newFiles: AstroFile[], options?: IngestOptions) => {
    if (newFiles.length === 0) return;
    showWorkspaceUnlessKept(options);
    quietCompletionRef.current = quietCompletion(options?.quiet === true, prevCompleteRef.current, quietCompletionRef.current);
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
      if (stats.done > 0 && stats.failed === 0 && !quietCompletionRef.current) {
        setShowConfetti(true);
        setTimeout(() => setShowConfetti(false), 3000);
      }
      quietCompletionRef.current = false;
    }
    prevCompleteRef.current = isComplete;
  }, [isComplete, stats.done, stats.failed]);

  useEffect(() => {
    const teardownRenderWorker = () => terminateStfWorker();
    window.addEventListener("beforeunload", teardownRenderWorker);
    return () => {
      window.removeEventListener("beforeunload", teardownRenderWorker);
      teardownRenderWorker();
    };
  }, []);

  const handleBrowseFiles = useCallback(async () => {
    if (isTauri()) {
      try {
        const { open } = await import("@tauri-apps/plugin-dialog");
        const result = await open({ multiple: true, filters: [{ name: "FITS", extensions: dialogExtensions(SUPPORTED_EXTENSIONS, IS_MAC) }] });
        if (result) {
          const partition = partitionIncoming(Array.isArray(result) ? result : [result]);
          setIngestReport(rejectionReport(partition));
          handleFilesAdded(partition.accepted.map(astroFileFromPath));
        }
      } catch (err) { console.error("[AstroBurst] File dialog error:", err); }
    } else {
      const input = document.createElement("input");
      input.type = "file"; input.multiple = true; input.accept = SUPPORTED_EXTENSIONS.map((ext) => `.${ext}`).join(",");
      input.onchange = (e: Event) => {
        const files = (e.target as HTMLInputElement).files;
        if (!files) return;
        const partition = partitionIncoming(Array.from(files), (f) => f.name);
        setIngestReport(rejectionReport(partition));
        handleFilesAdded(partition.accepted.map((f) => ({ name: f.name, path: f.name, size: f.size })));
      };
      input.click();
    }
  }, [handleFilesAdded]);

  const openSampleFrames = useCallback((opened: SampleOpen) => setIngestReport(openSampleProducts(opened)), []);
  const reportSampleFailure = useCallback((message: string) => setIngestReport({ skipped: 0, calib: 0, message }), []);
  const sampleRunners = useMemo(
    () => ({ primary: sampleSetRun(sampleDataStore, showWorkspace, reportSampleFailure), cube: sampleSetRun(sampleDataStore, showCubeTool, reportSampleFailure) }),
    [reportSampleFailure],
  );
  const sampleData = useSampleData(openSampleFrames);
  const paletteSampleSets = useSyncExternalStore(sampleDataStore.subscribe, PALETTE_SAMPLE_SETS, PALETTE_SAMPLE_SETS);

  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const shortcutsRef = useRef<HTMLDivElement>(null);
  const shortcutsTitleId = useId();
  useEffect(() => {
    if (!shortcutsOpen) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    shortcutsRef.current?.focus();
    return () => {
      if (previous?.isConnected) previous.focus();
    };
  }, [shortcutsOpen]);
  const handleShortcutsKeyDown = useCallback((e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Tab") return;
    const focusables = Array.from(e.currentTarget.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
    const target = focusTrapTarget(focusables.length, focusables.indexOf(document.activeElement as HTMLElement), e.shiftKey);
    if (target === null) return;
    e.preventDefault();
    focusables[target].focus();
  }, []);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const lastShiftUp = useRef(0);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const nextMode = modeFromShortcut(e);
      if (nextMode) {
        e.preventDefault();
        appModeStore.set(nextMode);
        return;
      }
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
    let dir: string | null = null;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const { readDir } = await import("@tauri-apps/plugin-fs");
      const result = await open({ directory: true, multiple: false, title: "Select FITS Folder" });
      dir = typeof result === "string" ? result : null;
      if (!dir) return;
      const base = dir;
      const sep = base.includes("\\") ? "\\" : "/";
      const entries = await readDir(base);
      const paths = entries.filter((entry) => !entry.isDirectory && entry.name).map((entry) => `${base}${sep}${entry.name}`);
      const partition = partitionIncoming(paths);
      setIngestReport(folderReport(base, partition));
      handleFilesAdded(partition.accepted.map(astroFileFromPath));
    } catch (err) {
      console.error("[AstroBurst] Folder dialog error:", err);
      if (dir) setIngestReport(folderErrorReport(dir, err));
    }
  }, [handleFilesAdded, handleBrowseFiles]);

  const handleNewBatch = useCallback(() => {
    reset();
    resetProductFilter();
    spectrumCsvStore.close();
    setView("empty");
    setShowConfetti(false);
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

  const handleHideFiles = useCallback(() => dockStore.dispatch({ type: "hide", tool: "files" }), []);

  const handleSelectFile = useCallback((id: string) => {
    fileStore.selectFile(id);
  }, []);

  const handleOpenTable = useCallback((id: string) => {
    const x1d = fileStore.getFile(id);
    if (!x1d) return;
    const cube = findSiblingCube(fileStore.getFiles(), x1d.path);
    if (!cube) return;
    const listed = metaCacheRef.current.get(cube);
    const active = productFilterStore.getActiveFilters();
    const revealed = filtersRevealing(listed ? metadataFilterable(listed) : processedFilterable(cube), active, productFilterStore.getMode());
    if (revealed !== active) productFilterStore.setActiveFilters(revealed);
    fileStore.selectFile(cube.id);
    spectrumComparisonStore.requestTable(cube.path, x1d.path);
    dockStore.dispatch({ type: "open", tool: "cube" });
  }, []);

  const handleOpenSpectrum = useCallback((id: string) => {
    if (!fileStore.getFile(id)) return;
    fileStore.selectFile(id);
    dockStore.dispatch({ type: "open", tool: "cube" });
  }, []);

  const handleExportZip = useCallback(() => {
    const all = fileStore.getFiles();
    exportZip(
      activeFilters.length === 0
        ? all
        : all.filter((f) => matchesActiveFilters(processedFilterable(f), activeFilters, filterMode)),
    );
  }, [exportZip, activeFilters, filterMode]);

  const storeVersion = useSyncExternalStore(fileStore.subscribe, fileStore.getVersion);
  const metaCacheRef = useRef<WeakMap<ProcessedFile, MetadataFile>>(new WeakMap());
  const metadataFiles = useMemo(
    () => toMetadataFiles(fileIds, (id) => fileStore.getFile(id), metaCacheRef.current),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [fileIds, storeVersion],
  );

  const comparableX1d = useMemo(() => comparableX1dIds(metadataFiles), [metadataFiles]);

  const productTypes = useMemo(
    () => detectProductTypes(fileIds.map((id) => fileStore.getFile(id)?.name ?? "")),
    [fileIds],
  );

  const filteredMetadataFiles = useMemo(() => {
    if (activeFilters.length === 0) return metadataFiles;
    return metadataFiles.filter((f) => matchesActiveFilters(metadataFilterable(f), activeFilters, filterMode));
  }, [metadataFiles, activeFilters, filterMode]);

  const filteredSelectedId = useMemo(
    () => fallbackSelectionId(filteredMetadataFiles, selectedId, activeFilters.length > 0),
    [selectedId, activeFilters, filteredMetadataFiles],
  );

  useEffect(() => {
    if (filteredSelectedId !== selectedId) followFilteredSelection(filteredSelectedId);
  }, [filteredSelectedId, selectedId]);

  const visibleSelectedFile = useMemo(() => {
    if (!selectedFile) return null;
    if (activeFilters.length === 0) return selectedFile;
    return filteredMetadataFiles.some((f) => f.id === selectedFile.id) ? selectedFile : null;
  }, [selectedFile, activeFilters, filteredMetadataFiles]);

  const hasVisibleFile = visibleSelectedFile !== null;
  const visibleFileKey = fileKeyOf(visibleSelectedFile);
  const sampleEntries = useMemo(
    () => samplePaletteActions(paletteSampleSets, { workspaceEmpty: view === "empty", fileShown: hasVisibleFile }, sampleRunners),
    [paletteSampleSets, view, hasVisibleFile, sampleRunners],
  );

  const selectedPath = selectedFile?.path ?? null;
  const selectedName = selectedFile?.name ?? null;
  const selectedStatus = selectedFile?.status ?? null;
  const workspaceFile = useMemo(
    () => workspaceFileRef(selectedPath, selectedName, selectedStatus),
    [selectedPath, selectedName, selectedStatus],
  );
  const archivePaletteActions = useArchivePaletteActions({ mode, setMode: appModeStore.set, workspaceFile });

  const paletteActions = useMemo<PaletteAction[]>(() => {
    const inWorkspace = (run: () => void) => () => {
      showWorkspace();
      run();
    };
    const acts: PaletteAction[] = [
      { id: "open-files", label: "Open FITS Files...", hint: "Ctrl+O", icon: FolderOpen, run: handleBrowseFiles },
      { id: "open-folder", label: "Open Folder...", icon: FolderOpen, run: handleSelectFolder },
    ];
    acts.push(...sampleEntries);
    if (view !== "empty") {
      for (const { tool, label } of paletteToolEntries(dockLayout, hasVisibleFile)) {
        acts.push({
          id: `tool-${tool}`,
          label,
          hint: "Tool",
          keywords: DOCK_TOOL_META[tool].keywords,
          icon: DOCK_TOOLS[tool].icon,
          run: inWorkspace(() => dockStore.dispatch({ type: "toggle", tool })),
        });
      }
      acts.push({
        id: "reset-layout",
        label: "Reset Layout (default panels, order and sizes; closes the right panels)",
        icon: Columns2,
        run: inWorkspace(() => dockStore.resetAll()),
      });
      acts.push({
        ...openSpectrumCsvPaletteAction(
          inWorkspace(() => {
            void openSpectrumCsv(visibleFileKey).then((opened) => {
              if (opened) dockStore.dispatch({ type: "open", tool: "cube" });
            });
          }),
        ),
        icon: Activity,
      });
      acts.push({ id: "dock-tips", label: "Show tool window tips", keywords: ["Layout", "Drag", "Dock", "Onboarding"], icon: Sparkles, run: inWorkspace(() => dockOnboardingStore.show()) });
      acts.push({ id: "history-undo", label: "Undo processing step", hint: "Ctrl+Z", keywords: ["History", "Undo"], icon: Undo2, run: inWorkspace(() => historyStore.undo()) });
      acts.push({ id: "history-redo", label: "Redo processing step", hint: "Ctrl+Shift+Z", keywords: ["History", "Redo"], icon: Redo2, run: inWorkspace(() => historyStore.redo()) });
      if (stats.done > 0) acts.push({ id: "export-zip", label: "Download ZIP of Processed Files", icon: Download, run: inWorkspace(handleExportZip) });
      if (isComplete) acts.push({ id: "new-batch", label: "New Batch (discard processed files)", hint: "confirm in footer", icon: RotateCcw, run: inWorkspace(handleNewBatchClick) });
    }
    acts.push(...archivePaletteActions);
    acts.push({ id: "shortcuts", label: "Keyboard Shortcuts", hint: "?", icon: InfoIcon, run: () => setShortcutsOpen(true) });
    return acts;
  }, [view, dockLayout, hasVisibleFile, visibleFileKey, stats.done, isComplete, handleBrowseFiles, handleSelectFolder, sampleEntries, handleExportZip, handleNewBatchClick, archivePaletteActions]);

  const handlePaletteSelectFile = useCallback((id: string) => {
    showWorkspace();
    fileStore.selectFile(id);
  }, []);

  const processing = useMemo(
    () => (isProcessing ? { done: stats.done, total: stats.total } : null),
    [isProcessing, stats.done, stats.total],
  );

  const paletteFiles = useMemo<PaletteFile[]>(
    () => selectableRows(filteredMetadataFiles).map((f) => ({ id: f.id, name: f.name, filter: f.metadata?.filter })),
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
          onSelectFile={handlePaletteSelectFile}
        />
        {shortcutsOpen && (
          <div
            className="fixed inset-0 z-[120] flex items-center justify-center bg-black/50 backdrop-blur-sm"
            onClick={() => setShortcutsOpen(false)}
          >
            <div
              ref={shortcutsRef}
              role="dialog"
              data-testid="shortcuts-sheet"
              aria-modal="true"
              aria-labelledby={shortcutsTitleId}
              tabIndex={-1}
              onKeyDown={handleShortcutsKeyDown}
              className="rounded-lg p-4 min-w-[300px] max-w-[min(640px,calc(100vw-32px))] max-h-[85vh] overflow-y-auto animate-fade-in focus:outline-none"
              style={{ background: "rgba(8,8,18,0.97)", border: "1px solid var(--ab-border-strong)", boxShadow: "0 8px 30px rgba(0,0,0,0.55)" }}
              onClick={(e) => e.stopPropagation()}
            >
              <div className="flex items-center justify-between mb-3">
                <span id={shortcutsTitleId} className="text-xs font-semibold text-zinc-300 uppercase tracking-wider">Keyboard Shortcuts</span>
                <button onClick={() => setShortcutsOpen(false)} title="Close" className="text-zinc-500 hover:text-zinc-300 transition-colors">
                  <X size={12} />
                </button>
              </div>
              {SHORTCUT_GROUPS.map((g, i) => (
                <div key={g.id} data-testid={`shortcuts-${g.id}`} className={i > 0 ? "mt-4" : undefined}>
                  {g.title && <div className="text-[10px] font-semibold text-zinc-400 uppercase tracking-wider mb-2">{g.title}</div>}
                  <ShortcutRows rows={g.rows} />
                </div>
              ))}
            </div>
          </div>
        )}
        <div className="fixed inset-0 z-0 pointer-events-none">
          <div
            className="absolute inset-0"
            style={{
              backgroundImage: showBg ? `url(${nebulaImg})` : "none", backgroundSize: "cover", backgroundPosition: "center",
              opacity: view === "empty" && mode === "workspace" ? 0.4 : 0, transition: "opacity 0.6s ease",
            }}
          />
          <div
            className="absolute inset-0"
            style={{
              backgroundImage: showBg ? `url(${nebulaImg})` : "none", backgroundSize: "cover", backgroundPosition: "center",
              filter: "blur(8px) brightness(0.3)",
              opacity: view !== "empty" || mode === "archive" ? 0.4 : 0, transition: "opacity 0.6s ease",
            }}
          />
        </div>
        {loading ? (
          <div className="relative z-50 h-screen flex flex-col items-center justify-center animate-fade-in" style={{ background: "var(--ab-deep)" }}>
            <AstroLogo size={80} showText={false} className="animate-pulse" />
            <h1 className="mt-6 text-xl tracking-[0.5em] uppercase cosmic-text">AstroBurst</h1>
          </div>
        ) : (
          <div className="relative z-10 h-full animate-fade-in flex flex-col">
            <div className="relative flex-1 min-h-0">
              <DropZone onFilesAdded={handleFilesAdded} report={ingestReport}>
                <div data-testid="workspace-root" className="absolute inset-0" style={mode === "workspace" ? undefined : HIDDEN_LAYER} inert={mode !== "workspace"}>
                  {view === "empty" ? (
                    <div className="h-full flex items-center justify-center">
                      <EmptyState onBrowseFiles={handleBrowseFiles} onSelectFolder={handleSelectFolder} onBrowseArchive={showArchive} sample={sampleData ?? undefined} />
                    </div>
                  ) : (
                    <CompositeProvider>
                      <ComposeWizardProvider>
                        <PreviewProvider file={visibleSelectedFile} doneFiles={filteredDoneFiles}>
                          <DisplayedImageBridge />
                          <div className="flex flex-col h-full">
                            <div
                              className="shrink-0 grid"
                              data-testid="progress-strip"
                              data-open={progressStripVisible ? "true" : "false"}
                              aria-hidden={!progressStripVisible}
                              style={{
                                gridTemplateRows: progressStripVisible ? "1fr" : "0fr",
                                opacity: progressStripVisible ? 1 : 0,
                                transition: "grid-template-rows 300ms ease, opacity 300ms ease",
                              }}
                            >
                              <div className="overflow-hidden min-h-0">
                                <div
                                  className="px-4 py-2 space-y-1.5"
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
                              </div>
                            </div>

                            <DockShell
                              tools={{
                                files: (
                                    <MetadataFileList
                                      files={filteredMetadataFiles}
                                      totalFiles={metadataFiles.length}
                                      selectedId={selectedId}
                                      onSelect={handleSelectFile}
                                      onExportZip={handleExportZip}
                                      collapsed={false}
                                      onToggle={handleHideFiles}
                                      isExporting={isExporting}
                                      zipProgress={zipProgress}
                                      downloaded={downloaded}
                                      exportError={zipError}
                                      productTypes={productTypes}
                                      customChips={filterState.customChips}
                                      activeFilters={activeFilters}
                                      filterMode={filterMode}
                                      onToggleFilter={toggleFilter}
                                      onToggleMode={toggleMode}
                                      onClearFilters={clearAll}
                                      onAddCustomChip={addCustomChip}
                                      onRemoveCustomChip={removeCustomChip}
                                      onOpenSpectrum={handleOpenSpectrum}
                                      onOpenTable={handleOpenTable}
                                      comparableX1d={comparableX1d}
                                    />
                                ),
                              }}
                            >
                              <MemoizedPreviewPanel />
                            </DockShell>
                          </div>
                        </PreviewProvider>
                      </ComposeWizardProvider>
                    </CompositeProvider>
                  )}
                </div>
                {archiveMounted && (
                  <div data-testid="archive-root" className="absolute inset-0" style={mode === "archive" ? undefined : HIDDEN_LAYER} inert={mode !== "archive"}>
                    <Suspense fallback={null}>
                      <ArchiveRoot active={mode === "archive"} workspaceFile={workspaceFile} onSwitchToWorkspace={showWorkspace} />
                    </Suspense>
                  </div>
                )}
              </DropZone>
            </div>
            {(mode === "archive" || view !== "empty") && (
              <AppFooter mode={mode} onModeChange={appModeStore.set} onOpenPalette={() => setPaletteOpen(true)} onOpenShortcuts={() => setShortcutsOpen(true)}>
                {mode === "archive" ? (
                  <Suspense fallback={null}>
                    <ArchiveFooterStatus processing={processing} />
                  </Suspense>
                ) : isComplete ? (
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
              </AppFooter>
            )}
          </div>
        )}
      </div>
    </ErrorBoundary>
  );
}
