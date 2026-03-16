import { useState, useCallback, useRef, useEffect, useMemo, lazy, Suspense, memo } from "react";
import {
  Image, Cpu, Zap, BarChart3, Layers, Sparkles, Loader2,
  Maximize2, Minimize2, X, Info as InfoIcon, FileText,
  PackageOpen, Layers2, Settings,
} from "lucide-react";

import { getCubeSpectrum } from "../services/cube.service";
import { probeGpu, isGpuAvailable } from "../infrastructure/gpu/GpuSingleton";
import { PreviewProvider, useFileContext, useHistContext, useCubeContext, useRawPixelsContext } from "../context/PreviewContext";
import { useMousePixel, useMousePixelActions } from "../hooks/useMousePixelStore";
import { useSelectedFile, useDoneFiles } from "../hooks/useFileStore";
import WcsReadout from "./header/WcsReadout";
import type { ProcessedFile } from "../shared/types";

const PreviewTab = lazy(() => import("./preview/PreviewTab"));
const AnalysisTab = lazy(() => import("./analysis/AnalysisTab"));
const ProcessingTab = lazy(() => import("./processing/ProcessingTab"));
const ComposeTab = lazy(() => import("./compose/ComposeTab"));
const HeadersTab = lazy(() => import("./header/HeadersTab"));
const ExportTab = lazy(() => import("./export/ExportTab"));
const StackingTab = lazy(() => import("./stacking/StackingTab"));
const ConfigTab = lazy(() => import("./preview/ConfigTab"));

type BottomTabId = "info" | "analysis" | "headers" | "export";
type SideTabId = "processing" | "compose" | "stacking" | "config";

interface TabDef<T> { id: T; label: string; icon: typeof Image; }

const BOTTOM_TABS: TabDef<BottomTabId>[] = [
  { id: "info", label: "Info", icon: InfoIcon },
  { id: "analysis", label: "Analysis", icon: BarChart3 },
  { id: "headers", label: "Headers", icon: FileText },
  { id: "export", label: "Export", icon: PackageOpen },
];

const SIDE_TABS: TabDef<SideTabId>[] = [
  { id: "processing", label: "Processing", icon: Sparkles },
  { id: "compose", label: "Compose", icon: Layers },
  { id: "stacking", label: "Stacking", icon: Layers2 },
  { id: "config", label: "Settings", icon: Settings },
];

const BOTTOM_MIN = 28;
const BOTTOM_DEFAULT = 220;
const BOTTOM_MAX = 500;
const SIDE_PANEL_DEFAULT = 380;
const SIDE_PANEL_MAX = 600;
const SIDE_PANEL_MIN_W = 280;
const EMPTY_SPECTRUM: number[] = [];

function TabSpinner() {
  return <div className="flex items-center justify-center py-12"><Loader2 size={20} className="animate-spin" style={{ color: "var(--ab-teal)" }} /></div>;
}

export default function PreviewPanel() {
  const file = useSelectedFile();
  const doneFiles = useDoneFiles();

  return (
    <PreviewProvider file={file} doneFiles={doneFiles}>
      <PreviewPanelInner />
    </PreviewProvider>
  );
}

function PreviewPanelInner() {
  const { file } = useFileContext();
  const { isCube } = useCubeContext();
  const { rawPixels, rawPixelsLoading, loadRawPixels, clearRawPixels } = useRawPixelsContext();
  const { handleMove, handleLeave, reset: resetMouse } = useMousePixelActions();

  const [activeBottomTab, setActiveBottomTab] = useState<BottomTabId>("info");
  const [activeSideTab, setActiveSideTab] = useState<SideTabId | null>(null);
  const [useGpu, setUseGpu] = useState(false);
  const [gpuAvailable, setGpuAvailable] = useState<boolean | null>(null);
  const [bottomOpen, setBottomOpen] = useState(true);
  const [, forceRender] = useState(0);

  const [spectrum, setSpectrum] = useState<number[]>(EMPTY_SPECTRUM);
  const [specWavelengths, setSpecWavelengths] = useState<number[] | null>(null);
  const [specCoord, setSpecCoord] = useState<{ x: number; y: number } | null>(null);
  const [specLoading, setSpecLoading] = useState(false);
  const [specElapsed, setSpecElapsed] = useState(0);

  const bottomHeightRef = useRef(BOTTOM_DEFAULT);
  const bottomResizing = useRef(false);
  const bottomElRef = useRef<HTMLDivElement>(null);
  const sidePanelWidthRef = useRef(SIDE_PANEL_DEFAULT);
  const sidePanelResizing = useRef(false);
  const sidePanelElRef = useRef<HTMLDivElement>(null);
  const starOverlayRef = useRef<HTMLCanvasElement>(null);
  const prevFileIdRef = useRef<string | null>(null);
  const specAbortRef = useRef(0);
  const fileDimsRef = useRef<[number, number] | undefined>(undefined);
  fileDimsRef.current = file?.result?.dimensions;

  useEffect(() => {
    probeGpu().then(() => {
      const available = isGpuAvailable() === true;
      setGpuAvailable(available);
      if (available) setUseGpu(true);
    });
  }, []);

  useEffect(() => {
    if (!file || file.id === prevFileIdRef.current) return;
    prevFileIdRef.current = file.id;
    setSpectrum(EMPTY_SPECTRUM);
    setSpecWavelengths(null);
    setSpecCoord(null);
    specAbortRef.current++;
    resetMouse();
    clearRawPixels();
    if (gpuAvailable && useGpu) loadRawPixels();
  }, [file?.id, gpuAvailable, useGpu, clearRawPixels, loadRawPixels, resetMouse]);

  const handleToggleGpu = useCallback(() => {
    if (useGpu) { setUseGpu(false); clearRawPixels(); }
    else { setUseGpu(true); loadRawPixels(); }
  }, [useGpu, loadRawPixels, clearRawPixels]);

  const handleImageClick = useCallback(async (e: React.MouseEvent<HTMLImageElement>) => {
    if (!isCube || !file?.path) return;
    const img = e.target as HTMLImageElement;
    const rect = img.getBoundingClientRect();
    const dims = file.result?.dimensions;
    if (!dims) return;
    const pixelX = Math.floor(((e.clientX - rect.left) / rect.width) * dims[0]);
    const pixelY = Math.floor(((e.clientY - rect.top) / rect.height) * dims[1]);
    setSpecCoord({ x: pixelX, y: pixelY });
    setSpecLoading(true);
    const seq = ++specAbortRef.current;
    try {
      const result = await getCubeSpectrum(file.path, pixelX, pixelY);
      if (specAbortRef.current !== seq) return;
      setSpectrum(result.spectrum || EMPTY_SPECTRUM);
      setSpecWavelengths(result.wavelengths || null);
      setSpecElapsed(result.elapsed_ms || 0);
    } catch { if (specAbortRef.current === seq) setSpectrum(EMPTY_SPECTRUM); }
    finally { if (specAbortRef.current === seq) setSpecLoading(false); }
  }, [isCube, file?.path, file?.result?.dimensions, getCubeSpectrum]);

  const handlePreviewMouseMove = useCallback((e: React.MouseEvent<HTMLElement>) => {
    handleMove(e, fileDimsRef.current);
  }, [handleMove]);

  const handleBottomResizeStart = useCallback((e: React.MouseEvent) => {
    if (!bottomOpen) return;
    e.preventDefault();
    bottomResizing.current = true;
    const startY = e.clientY;
    const startH = bottomHeightRef.current;
    document.body.style.cursor = "row-resize";
    document.body.style.userSelect = "none";
    const el = bottomElRef.current;
    const onMove = (ev: MouseEvent) => { if (!bottomResizing.current) return; const next = Math.max(100, Math.min(BOTTOM_MAX, startH + (startY - ev.clientY))); bottomHeightRef.current = next; if (el) el.style.height = `${next}px`; };
    const onUp = () => { bottomResizing.current = false; document.body.style.cursor = ""; document.body.style.userSelect = ""; window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); forceRender((c) => c + 1); };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, [bottomOpen]);

  const handleSidePanelResizeStart = useCallback((e: React.MouseEvent) => {
    if (!activeSideTab) return;
    e.preventDefault();
    sidePanelResizing.current = true;
    const startX = e.clientX;
    const startW = sidePanelWidthRef.current;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    const el = sidePanelElRef.current;
    const onMove = (ev: MouseEvent) => { if (!sidePanelResizing.current) return; const next = Math.max(SIDE_PANEL_MIN_W, Math.min(SIDE_PANEL_MAX, startW + (startX - ev.clientX))); sidePanelWidthRef.current = next; if (el) el.style.width = `${next}px`; };
    const onUp = () => { sidePanelResizing.current = false; document.body.style.cursor = ""; document.body.style.userSelect = ""; window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); forceRender((c) => c + 1); };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, [activeSideTab]);

  const handleSideTabClick = useCallback((tabId: SideTabId) => { setActiveSideTab((prev) => (prev === tabId ? null : tabId)); }, []);

  const sideContent = useMemo(() => {
    switch (activeSideTab) {
      case "processing": return <ProcessingTab />;
      case "compose": return <ComposeTab />;
      case "stacking": return <StackingTab />;
      case "config": return <ConfigTab />;
      default: return null;
    }
  }, [activeSideTab]);

  const effectiveBottomH = bottomOpen ? bottomHeightRef.current : BOTTOM_MIN;
  const sidePanelOpen = activeSideTab !== null;
  const activeTabMeta = SIDE_TABS.find((t) => t.id === activeSideTab);

  return (
    <div className="flex h-full overflow-hidden">
      <div className="flex-1 min-w-0 flex flex-col overflow-hidden">
        <div className="flex items-center justify-between px-3 py-1.5 shrink-0" style={{ background: "linear-gradient(90deg, rgba(20,184,166,0.04) 0%, rgba(5,5,16,0.6) 50%, rgba(59,130,246,0.04) 100%)", borderBottom: "1px solid rgba(20,184,166,0.12)" }}>
          <div className="flex items-center gap-2">
            <Image size={12} style={{ color: "var(--ab-teal)" }} />
            <span className="text-[11px] font-medium text-zinc-300">Preview</span>
          </div>
          <div className="flex items-center gap-2">
            {file && (
              <button onClick={handleToggleGpu} disabled={gpuAvailable === false && !useGpu}
                className="flex items-center gap-1 text-[10px] px-2 py-1 rounded transition-all duration-200 disabled:opacity-30 disabled:cursor-not-allowed"
                style={useGpu ? { background: "rgba(168,85,247,0.15)", color: "#c084fc", border: "1px solid rgba(168,85,247,0.3)", boxShadow: "0 0 8px rgba(168,85,247,0.15)" } : { color: "#71717a", border: "1px solid transparent" }}
                title={gpuAvailable === false ? "WebGPU not available" : useGpu ? "Switch to CPU rendering" : "Switch to GPU rendering"}>
                {rawPixelsLoading ? <Loader2 size={10} className="animate-spin" /> : useGpu ? <Zap size={10} /> : <Cpu size={10} />}
                {rawPixelsLoading ? "Loading..." : gpuAvailable === false ? "CPU only" : useGpu ? "GPU" : "CPU"}
              </button>
            )}
            {file && <span className="text-[10px] font-mono text-zinc-600 truncate max-w-[200px]">{file.name}</span>}
          </div>
        </div>

        <div className="flex-1 overflow-hidden" onMouseMove={handlePreviewMouseMove} onMouseLeave={handleLeave}>
          {!file ? (
            <div className="flex flex-col items-center justify-center h-full gap-3 text-zinc-600"><Image size={48} strokeWidth={1} /><p className="text-sm">Select a processed file</p></div>
          ) : (
            <Suspense fallback={<TabSpinner />}>
              <PreviewTab useGpu={useGpu} rawPixels={rawPixels} onImageClick={handleImageClick} starOverlayRef={starOverlayRef} />
            </Suspense>
          )}
        </div>

        {file && (
          <div ref={bottomElRef} className="shrink-0 flex flex-col overflow-hidden" style={{ height: effectiveBottomH, borderTop: "1px solid rgba(20,184,166,0.1)", background: "linear-gradient(180deg, rgba(20,184,166,0.03) 0%, rgba(5,5,16,0.5) 100%)" }}>
            {bottomOpen && <div className="ab-resize-handle-h shrink-0" onMouseDown={handleBottomResizeStart} />}
            <div className="flex items-center justify-between px-1 shrink-0" style={{ borderBottom: "1px solid rgba(20,184,166,0.08)" }}>
              <div className="flex items-center gap-0">
                {BOTTOM_TABS.map((tab) => { const Icon = tab.icon; return (
                  <button key={tab.id} onClick={() => { setActiveBottomTab(tab.id); if (!bottomOpen) setBottomOpen(true); }}
                    className="ab-tab" data-active={activeBottomTab === tab.id}>
                    <Icon size={11} />{tab.label}
                  </button>
                ); })}
              </div>
              <div className="flex items-center gap-2 pr-2">
                {file.result?.dimensions && (
                  <span className="text-[10px] font-mono text-zinc-600">
                    {file.result.dimensions[0]}x{file.result.dimensions[1]}
                    {file.result.header?.BITPIX && <span className="ml-2 text-zinc-700">BITPIX {file.result.header.BITPIX}</span>}
                    <span className="ml-2 text-zinc-700">{(file.result.elapsed_ms / 1000).toFixed(2)}s</span>
                  </span>
                )}
                <button onClick={() => setBottomOpen(!bottomOpen)} className="p-0.5 rounded hover:bg-zinc-800 text-zinc-600 hover:text-zinc-400 transition-colors">
                  {bottomOpen ? <Minimize2 size={11} /> : <Maximize2 size={11} />}
                </button>
              </div>
            </div>
            {bottomOpen && (
              <div className="flex-1 overflow-y-auto px-3 py-2">
                <Suspense fallback={<TabSpinner />}>
                  {activeBottomTab === "info" && <MemoBottomInfo />}
                  {activeBottomTab === "analysis" && <AnalysisTab spectrum={spectrum} specWavelengths={specWavelengths} specCoord={specCoord} specLoading={specLoading} specElapsed={specElapsed} starOverlayRef={starOverlayRef} />}
                  {activeBottomTab === "headers" && <HeadersTab />}
                  {activeBottomTab === "export" && <ExportTab />}
                </Suspense>
              </div>
            )}
          </div>
        )}
      </div>

      {file && sidePanelOpen && (
        <>
          <div className="ab-resize-handle" onMouseDown={handleSidePanelResizeStart} />
          <div ref={sidePanelElRef} className="shrink-0 flex flex-col overflow-hidden" style={{ width: sidePanelWidthRef.current, borderLeft: "1px solid rgba(20,184,166,0.1)", background: "linear-gradient(135deg, rgba(5,5,16,0.7) 0%, rgba(20,184,166,0.02) 100%)" }}>
            <div className="flex items-center justify-between px-3 py-1.5 shrink-0" style={{ borderBottom: "1px solid rgba(20,184,166,0.1)", background: "rgba(20,184,166,0.03)" }}>
              <div className="flex items-center gap-2">
                {activeTabMeta && <><activeTabMeta.icon size={12} style={{ color: "var(--ab-teal)" }} /><span className="text-[11px] font-medium text-zinc-300">{activeTabMeta.label}</span></>}
              </div>
              <button onClick={() => setActiveSideTab(null)} className="p-1 rounded transition-colors text-zinc-600 hover:text-zinc-300" style={{ background: "transparent" }} onMouseEnter={(e) => (e.currentTarget.style.background = "rgba(20,184,166,0.1)")} onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}>
                <X size={12} />
              </button>
            </div>
            <div className="flex-1 overflow-y-auto p-3"><Suspense fallback={<TabSpinner />}>{sideContent}</Suspense></div>
          </div>
        </>
      )}

      {file && (
        <div className="shrink-0 w-[38px] flex flex-col items-center pt-2 gap-0.5" style={{ borderLeft: "1px solid rgba(20,184,166,0.08)", background: "linear-gradient(180deg, rgba(20,184,166,0.04) 0%, rgba(5,5,16,0.7) 40%, rgba(59,130,246,0.03) 100%)" }}>
          {SIDE_TABS.map((tab) => { const Icon = tab.icon; const isActive = activeSideTab === tab.id; return (
            <button key={tab.id} onClick={() => handleSideTabClick(tab.id)}
              className="relative w-[32px] h-[32px] flex items-center justify-center rounded-md transition-all duration-200"
              style={isActive ? { background: "rgba(20,184,166,0.12)", color: "var(--ab-teal)", boxShadow: "0 0 10px rgba(20,184,166,0.1)" } : { color: "#52525b" }}
              title={tab.label}>
              {isActive && <span className="absolute left-0 top-[6px] bottom-[6px] w-[2px] rounded-r" style={{ background: "var(--ab-teal)" }} />}
              <Icon size={15} />
            </button>
          ); })}
        </div>
      )}
    </div>
  );
}

const MemoBottomInfo = memo(function BottomInfo() {
  const { file } = useFileContext();
  const { histData, stfParams } = useHistContext();
  const mousePixel = useMousePixel();
  if (!file) return null;
  return (
    <div className="flex flex-col gap-2 text-[10px] font-mono text-zinc-500">
      {file?.path && file?.result?.dimensions && (
        <WcsReadout filePath={file.path} imageWidth={file.result.dimensions[0]} imageHeight={file.result.dimensions[1]} mouseX={mousePixel?.x ?? null} mouseY={mousePixel?.y ?? null} />
      )}
      {histData && (
        <div className="flex items-center gap-4 flex-wrap">
          <span>mean={histData.mean?.toFixed(2)}</span><span>median={histData.median?.toFixed(2)}</span><span>sigma={histData.sigma?.toFixed(2)}</span>
          <span style={{ color: "rgba(20,184,166,0.3)" }}>|</span>
          <span style={{ color: "rgba(239,68,68,0.6)" }}>S={stfParams.shadow.toFixed(4)}</span>
          <span style={{ color: "rgba(245,158,11,0.6)" }}>M={stfParams.midtone.toFixed(4)}</span>
          <span style={{ color: "rgba(16,185,129,0.6)" }}>H={stfParams.highlight.toFixed(4)}</span>
        </div>
      )}
      {file.result?.header && (
        <div className="flex items-center gap-4 flex-wrap text-zinc-600">
          {file.result.header.TELESCOP && <span>TELESCOP: {file.result.header.TELESCOP}</span>}
          {file.result.header.INSTRUME && <span>INSTRUME: {file.result.header.INSTRUME}</span>}
          {file.result.header.FILTER && <span>FILTER: {file.result.header.FILTER}</span>}
          {file.result.header.EXPTIME && <span>EXPTIME: {file.result.header.EXPTIME}s</span>}
          {file.result.header["DATE-OBS"] && <span>DATE: {file.result.header["DATE-OBS"]}</span>}
        </div>
      )}
    </div>
  );
});
