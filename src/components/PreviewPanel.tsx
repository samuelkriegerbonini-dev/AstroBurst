import { useState, useCallback, useRef, useEffect, useMemo, lazy, Suspense } from "react";
import {
  Image, Cpu, Zap, Layers, Sparkles, Loader2,
  Layers2, FlaskConical, Info, Settings,
} from "lucide-react";

import { getCubeSpectrum } from "../services/cube";
import { probeGpu, isGpuAvailable } from "../infrastructure/gpu/GpuSingleton";
import { useFileContext, useCubeContext, useRawPixelsContext, useRenderContext, useStarOverlayContext } from "../context/PreviewContext";
import { useCompositeContext } from "../context/CompositeContext";
import { useMousePixelActions, setMousePixel } from "../hooks/useMousePixelStore";
import AdvancedImageViewer from "./viewer/AdvancedImageViewer";
import { useProgress } from "../hooks/useProgress";

const PreviewTab = lazy(() => import("./preview/PreviewTab"));
const ProcessingTab = lazy(() => import("./processing/ProcessingTab"));
const ComposeWizard = lazy(() => import("./compose/ComposeWizard"));
const StackingTab = lazy(() => import("./stacking/StackingTab"));
const ConfigTab = lazy(() => import("./preview/ConfigTab"));
const SynthPanel = lazy(() => import("./synth/SynthPanel"));
const InfoPanel = lazy(() => import("./file/SidebarPanels").then((m) => ({ default: m.InfoPanel })));

type ToolId = "processing" | "compose" | "stacking" | "info" | "synth" | "config";

interface ToolDef {
  id: ToolId;
  label: string;
  shortLabel: string;
  icon: typeof Image;
  accent: string;
}

const TOP_TOOLS: ToolDef[] = [
  { id: "compose", label: "Compose", shortLabel: "Comp", icon: Layers, accent: "var(--ab-teal)" },
  { id: "processing", label: "Processing", shortLabel: "Proc", icon: Sparkles, accent: "var(--ab-amber)" },
  { id: "stacking", label: "Stacking", shortLabel: "Stack", icon: Layers2, accent: "var(--ab-blue)" },
];

const BOTTOM_STRIP_TOOLS: ToolDef[] = [
  { id: "info", label: "Info", shortLabel: "Info", icon: Info, accent: "var(--ab-teal)" },
  { id: "synth", label: "Synth", shortLabel: "Synth", icon: FlaskConical, accent: "var(--ab-rose)" },
  { id: "config", label: "Settings", shortLabel: "Config", icon: Settings, accent: "#a1a1aa" },
];

const BOTTOM_MIN = 140;
const BOTTOM_MAX = 600;
const BOTTOM_DEFAULT = 280;

function TabSpinner() {
  return <div className="flex items-center justify-center py-8"><Loader2 size={16} className="animate-spin" style={{ color: "var(--ab-teal)" }} /></div>;
}

function ToolContent({ toolId }: { toolId: ToolId }) {
  switch (toolId) {
    case "processing": return <ProcessingTab />;
    case "compose": return <ComposeWizard />;
    case "stacking": return <StackingTab />;
    case "info": return <InfoPanel />;
    case "config": return <ConfigTab />;
    case "synth": return <SynthPanel />;
    default: return null;
  }
}

function ProgressBarInner() {
  const progress = useProgress("compose-progress");
  if (!progress.active) return null;
  return (
    <div className="ab-compose-progress shrink-0">
      <div className="ab-compose-progress-bar" style={{ width: `${progress.percent}%` }} />
      <span className="ab-compose-progress-label">{progress.stage} {progress.percent > 0 ? `${progress.percent}%` : ""}</span>
    </div>
  );
}

export default function PreviewPanel() {
  const { file } = useFileContext();
  const { isCube } = useCubeContext();
  const { rawPixels, rawPixelsLoading, loadRawPixels, clearRawPixels } = useRawPixelsContext();
  const { renderedPreviewUrl } = useRenderContext();
  const { compositePreviewUrl } = useCompositeContext();
  const { starOverlayRef } = useStarOverlayContext();
  const { handleMove, handleLeave, reset: resetMouse } = useMousePixelActions();

  const [activeTool, setActiveTool] = useState<ToolId | null>("compose");
  const [useGpu, setUseGpu] = useState(false);
  const [gpuAvailable, setGpuAvailable] = useState<boolean | null>(null);
  const [gpuProbing, setGpuProbing] = useState(true);
  const [, forceRender] = useState(0);

  const prevFileIdRef = useRef<string | null>(null);
  const specAbortRef = useRef(0);
  const fileDimsRef = useRef<[number, number] | undefined>(undefined);
  fileDimsRef.current = file?.result?.dimensions;

  const bottomHeightRef = useRef(BOTTOM_DEFAULT);
  const bottomElRef = useRef<HTMLDivElement>(null);
  const bResizing = useRef(false);
  const bStartY = useRef(0);
  const bStartH = useRef(0);

  useEffect(() => { probeGpu().then(() => { setGpuAvailable(isGpuAvailable() === true); setGpuProbing(false); }); }, []);

  useEffect(() => {
    if (!file || file.id === prevFileIdRef.current) return;
    prevFileIdRef.current = file.id;
    specAbortRef.current++;
    resetMouse();
    clearRawPixels();
    if (gpuAvailable && useGpu) loadRawPixels();
  }, [file?.id, gpuAvailable, useGpu, clearRawPixels, loadRawPixels, resetMouse]);

  const handleToggleGpu = useCallback(() => {
    if (useGpu) { setUseGpu(false); clearRawPixels(); } else { setUseGpu(true); loadRawPixels(); }
  }, [useGpu, loadRawPixels, clearRawPixels]);

  const handleImageClick = useCallback(async (e: React.MouseEvent<HTMLImageElement>) => {
    if (!isCube || !file?.path) return;
    const img = e.target as HTMLImageElement;
    const rect = img.getBoundingClientRect();
    const dims = file.result?.dimensions;
    if (!dims) return;
    const pixelX = Math.floor(((e.clientX - rect.left) / rect.width) * dims[0]);
    const pixelY = Math.floor(((e.clientY - rect.top) / rect.height) * dims[1]);
    ++specAbortRef.current;
    try { await getCubeSpectrum(file.path, pixelX, pixelY); } catch {}
  }, [isCube, file?.path, file?.result?.dimensions]);

  const handlePreviewMouseMove = useCallback((e: React.MouseEvent<HTMLElement>) => { handleMove(e, fileDimsRef.current); }, [handleMove]);
  const handleViewerMousePixel = useCallback((x: number, y: number) => { setMousePixel({ x, y }); }, []);

  const handleToggleTool = useCallback((toolId: ToolId) => {
    setActiveTool((prev) => prev === toolId ? null : toolId);
  }, []);

  const handleBottomResize = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    bResizing.current = true;
    bStartY.current = e.clientY;
    bStartH.current = bottomHeightRef.current;
    document.body.style.cursor = "row-resize";
    document.body.style.userSelect = "none";
    const el = bottomElRef.current;
    const onMove = (ev: MouseEvent) => {
      if (!bResizing.current) return;
      const next = Math.max(BOTTOM_MIN, Math.min(BOTTOM_MAX, bStartH.current - (ev.clientY - bStartY.current)));
      bottomHeightRef.current = next;
      if (el) el.style.height = `${next}px`;
    };
    const onUp = () => {
      bResizing.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      forceRender((c) => c + 1);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, []);

  const originalImage = useMemo(() => {
    if (!file?.result?.previewUrl) return null;
    const base = file.result.previewUrl;
    const sep = base.includes("?") ? "&" : "?";
    return { url: `${base}${sep}_v=${file.id}`, label: "Original", width: file.result.dimensions?.[0], height: file.result.dimensions?.[1] };
  }, [file?.result?.previewUrl, file?.result?.dimensions, file?.id]);

  const processedImage = useMemo(() => {
    if (!renderedPreviewUrl || renderedPreviewUrl === file?.result?.previewUrl) return null;
    return { url: renderedPreviewUrl, label: "Processed", width: file?.result?.dimensions?.[0], height: file?.result?.dimensions?.[1] };
  }, [renderedPreviewUrl, file?.result?.previewUrl, file?.result?.dimensions]);

  const useAdvancedViewer = !compositePreviewUrl && !useGpu;

  return (
    <div className="flex h-full overflow-hidden">
      <div className="flex-1 min-w-0 flex flex-col overflow-hidden">

        <div className="flex items-center justify-between px-3 py-1 shrink-0" style={{ background: "linear-gradient(90deg, rgba(20,184,166,0.04) 0%, rgba(5,5,16,0.6) 50%, rgba(59,130,246,0.04) 100%)", borderBottom: "1px solid rgba(20,184,166,0.12)" }}>
          <div className="flex items-center gap-2 shrink-0">
            <Image size={12} style={{ color: "var(--ab-teal)" }} />
            <span className="text-[11px] font-medium text-zinc-300">Preview</span>
          </div>
          <div className="flex items-center gap-2 justify-center flex-1 min-w-0">
            {file && <span className="text-[10px] font-mono text-zinc-600 truncate max-w-[200px]">{file.name}</span>}
            {file?.result?.dimensions && (
              <span className="text-[10px] font-mono text-zinc-500 flex items-center gap-1.5 shrink-0">
                <span className="text-zinc-400">{file.result.dimensions[0]}&times;{file.result.dimensions[1]}</span>
                {file.result.header?.BITPIX && <span className="text-zinc-600">BITPIX {file.result.header.BITPIX}</span>}
                <span className="text-zinc-600">{(file.result.elapsed_ms / 1000).toFixed(2)}s</span>
              </span>
            )}
          </div>
          <div className="flex items-center gap-2 shrink-0">
            {file && (
              <button onClick={handleToggleGpu} disabled={gpuProbing || (gpuAvailable === false && !useGpu)}
                className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded transition-all duration-200 disabled:opacity-30 disabled:cursor-not-allowed"
                style={useGpu ? { background: "rgba(168,85,247,0.15)", color: "#c084fc", border: "1px solid rgba(168,85,247,0.3)" } : { color: "#71717a", border: "1px solid transparent" }}>
                {gpuProbing ? <Loader2 size={10} className="animate-spin" /> : rawPixelsLoading ? <Loader2 size={10} className="animate-spin" /> : useGpu ? <Zap size={10} /> : <Cpu size={10} />}
                {gpuProbing ? "..." : rawPixelsLoading ? "..." : gpuAvailable === false ? "CPU" : useGpu ? "GPU" : "CPU"}
              </button>
            )}
          </div>
        </div>

        <ProgressBarInner />

        <div className="flex-1 overflow-hidden min-h-0">
          {!file ? (
            <AdvancedImageViewer original={null} processed={null} />
          ) : useAdvancedViewer ? (
            <AdvancedImageViewer
              original={originalImage}
              processed={processedImage}
              onMousePixel={handleViewerMousePixel}
              onMouseLeave={handleLeave}
              overlayCanvasRef={starOverlayRef}
            />
          ) : (
            <div className="h-full" onMouseMove={handlePreviewMouseMove} onMouseLeave={handleLeave}>
              <Suspense fallback={<TabSpinner />}>
                <PreviewTab useGpu={useGpu} rawPixels={rawPixels} onImageClick={handleImageClick} starOverlayRef={starOverlayRef} />
              </Suspense>
            </div>
          )}
        </div>

        {file && activeTool && (
          <>
            <div className="ab-resize-handle-h" onMouseDown={handleBottomResize} />
            <div
              ref={bottomElRef}
              className="ab-bottom-panel"
              style={{ height: bottomHeightRef.current }}
            >
              <Suspense fallback={<TabSpinner />}>
                <ToolContent toolId={activeTool} />
              </Suspense>
            </div>
          </>
        )}
      </div>

      {file && (
        <div className="ab-tool-strip">
          {TOP_TOOLS.map((def) => {
            const Icon = def.icon;
            const isActive = activeTool === def.id;
            return (
              <button
                key={def.id}
                onClick={() => handleToggleTool(def.id)}
                className={`ab-tool-strip-btn ${isActive ? "ab-tool-strip-btn-active" : ""}`}
                style={isActive ? { "--strip-accent": def.accent } as React.CSSProperties : undefined}
                title={def.label}
              >
                <Icon size={14} />
                <span>{def.shortLabel}</span>
              </button>
            );
          })}
          <div className="flex-1" />
          {BOTTOM_STRIP_TOOLS.map((def) => {
            const Icon = def.icon;
            const isActive = activeTool === def.id;
            return (
              <button
                key={def.id}
                onClick={() => handleToggleTool(def.id)}
                className={`ab-tool-strip-btn ${isActive ? "ab-tool-strip-btn-active" : ""}`}
                style={isActive ? { "--strip-accent": def.accent } as React.CSSProperties : undefined}
                title={def.label}
              >
                <Icon size={14} />
                <span>{def.shortLabel}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
