import { useState, useCallback, useRef, useEffect, useMemo, lazy, Suspense } from "react";
import {
  Image, Cpu, Zap, Sparkles, Loader2,
  Layers2, FlaskConical, Settings, Download, FileText, BarChart3,
} from "lucide-react";

import { getCubeSpectrum } from "../services/cube";
import { probeGpu, isGpuAvailable, onGpuLost, getGpuReason } from "../infrastructure/gpu/GpuSingleton";
import { useFileContext, useCubeContext, useRawPixelsContext, useRenderContext, useStarOverlayContext } from "../context/PreviewContext";
import { useCompositePreview, useCompositeActions } from "../context/CompositeContext";
import { useMousePixelActions, setMousePixel, emitPixelClick, usePixelClick } from "../hooks/useMousePixelStore";
import { useSpectrum, beginSpectrum, commitSpectrum, failSpectrum, resetSpectrum } from "../hooks/useSpectrumStore";
import AdvancedImageViewer from "./viewer/AdvancedImageViewer";
import { useProgress } from "../hooks/useProgress";

const PreviewTab = lazy(() => import("./preview/PreviewTab"));
const ProcessingTab = lazy(() => import("./processing/ProcessingTab"));
const ComposeWizard = lazy(() => import("./compose/ComposeWizard"));
const StackingTab = lazy(() => import("./stacking/StackingTab"));
const ConfigTab = lazy(() => import("./preview/ConfigTab"));
const SynthPanel = lazy(() => import("./synth/SynthPanel"));
const ExportTab = lazy(() => import("./export/ExportTab"));
const AnalysisTab = lazy(() => import("./analysis/AnalysisTab"));
const HeadersTab = lazy(() => import("./header/HeadersTab"));

export type ToolId = "compose" | "processing" | "stacking" | "synth" | "config" | "export" | "headers" | "analysis";
export type RightToolId = Exclude<ToolId, "compose">;

interface ToolDef {
  id: RightToolId;
  label: string;
  shortLabel: string;
  icon: typeof Image;
  accent: string;
}

const TOP_TOOLS: ToolDef[] = [
  { id: "headers", label: "Headers", shortLabel: "Headers", icon: FileText, accent: "var(--ab-teal)" },
  { id: "analysis", label: "Analysis", shortLabel: "Analysis", icon: BarChart3, accent: "var(--ab-blue)" },
  { id: "processing", label: "Processing", shortLabel: "Proc", icon: Sparkles, accent: "var(--ab-amber)" },
  { id: "stacking", label: "Stacking", shortLabel: "Stack", icon: Layers2, accent: "var(--ab-blue)" },
];

const BOTTOM_STRIP_TOOLS: ToolDef[] = [
  { id: "synth", label: "Synth", shortLabel: "Synth", icon: FlaskConical, accent: "var(--ab-rose)" },
  { id: "export", label: "Export", shortLabel: "Export", icon: Download, accent: "var(--ab-amber)" },
  { id: "config", label: "Settings", shortLabel: "Config", icon: Settings, accent: "#a1a1aa" },
];

const BOTTOM_MIN = 140;
const BOTTOM_MAX = 600;
const BOTTOM_DEFAULT = 280;

const gpuSupported = typeof navigator !== "undefined" && !!navigator.gpu;

function TabSpinner() {
  return <div className="flex items-center justify-center py-8"><Loader2 size={16} className="animate-spin" style={{ color: "var(--ab-teal)" }} /></div>;
}

function RightToolContent({ toolId, starOverlayRef }: { toolId: RightToolId; starOverlayRef: React.RefObject<HTMLCanvasElement | null> }) {
  const spec = useSpectrum();
  switch (toolId) {
    case "headers": return <HeadersTab />;
    case "analysis": return (
      <AnalysisTab
        spectrum={spec.spectrum}
        specWavelengths={spec.wavelengths}
        specCoord={spec.coord}
        specLoading={spec.loading}
        specElapsed={spec.elapsed}
        specError={spec.error}
        starOverlayRef={starOverlayRef}
      />
    );
    case "processing": return <ProcessingTab />;
    case "stacking": return <StackingTab />;
    case "config": return <ConfigTab />;
    case "synth": return <SynthPanel />;
    case "export": return <ExportTab />;
    default: return null;
  }
}

function ProgressBarInner() {
  const progress = useProgress("compose-progress");
  if (!progress.active) return null;
  return (
    <div className="ab-compose-progress shrink-0">
      <div className="ab-compose-progress-bar" style={{ transform: `scaleX(${Math.min(100, Math.max(0, progress.percent)) / 100})` }} />
      <span className="ab-compose-progress-label">{progress.stage} {progress.percent > 0 ? `${progress.percent}%` : ""}</span>
    </div>
  );
}

export interface PreviewPanelProps {
  activeTool: ToolId | null;
}

export default function PreviewPanel({ activeTool }: PreviewPanelProps) {
  const { file } = useFileContext();
  const { isCube } = useCubeContext();
  const { rawPixels, rawPixelsLoading, loadRawPixels, clearRawPixels,
          rgbRawPixels, rgbRawPixelsLoading, loadRgbRawPixels, clearRgbRawPixels } = useRawPixelsContext();
  const { renderedPreviewUrl } = useRenderContext();
  const { compositePreviewUrl } = useCompositePreview();
  const { initRgb, setCompositePreviewUrl } = useCompositeActions();
  const { starOverlayRef } = useStarOverlayContext();
  const { handleMove, handleLeave, reset: resetMouse } = useMousePixelActions();

  const [useGpu, setUseGpu] = useState(false);
  const [gpuAvailable, setGpuAvailable] = useState<boolean | null>(null);
  const [gpuProbing, setGpuProbing] = useState(true);
  const [gpuReason, setGpuReason] = useState<string | null>(null);
  const [, forceRender] = useState(0);
  const [rightTool, setRightTool] = useState<RightToolId | null>(null);
  const toggleRightTool = useCallback((id: RightToolId) => {
    setRightTool((prev) => (prev === id ? null : id));
  }, []);

  const prevFileIdRef = useRef<string | null>(null);
  const prevCompositeUrlRef = useRef<string | null>(null);
  const rgbLoadKeyRef = useRef<string | null>(null);
  const specAbortRef = useRef(0);
  const fileDimsRef = useRef<[number, number] | undefined>(undefined);
  fileDimsRef.current = file?.result?.dimensions;

  const isRgbView = compositePreviewUrl !== null;
  const isFileRgbView = isRgbView && !!file?.result?.is_rgb && compositePreviewUrl === (file?.result?.previewUrl ?? null);
  const toggleLoading = isRgbView ? rgbRawPixelsLoading : rawPixelsLoading;

  const bottomHeightRef = useRef(BOTTOM_DEFAULT);
  const bottomElRef = useRef<HTMLDivElement>(null);
  const bResizing = useRef(false);
  const bStartY = useRef(0);
  const bStartH = useRef(0);

  useEffect(() => { probeGpu().then(() => { setGpuAvailable(isGpuAvailable() === true); setGpuReason(getGpuReason()); setGpuProbing(false); }); }, []);

  useEffect(() => {
    const unsub = onGpuLost(() => {
      setGpuAvailable(false);
      setGpuReason(getGpuReason());
      setUseGpu(false);
      clearRawPixels();
      clearRgbRawPixels();
    });
    return unsub;
  }, [clearRawPixels, clearRgbRawPixels]);

  useEffect(() => {
    if (!file) {
      if (prevFileIdRef.current !== null) {
        prevFileIdRef.current = null;
        rgbLoadKeyRef.current = null;
        clearRawPixels();
        clearRgbRawPixels();
      }
      return;
    }
    if (file.id === prevFileIdRef.current) return;
    prevFileIdRef.current = file.id;
    specAbortRef.current++;
    resetSpectrum();
    resetMouse();
    clearRawPixels();
    clearRgbRawPixels();
    rgbLoadKeyRef.current = null;
    if (gpuAvailable && useGpu) {
      const fid = file.id;
      const path = file.path;
      const isRgb = !!file.result?.is_rgb;
      queueMicrotask(() => {
        if (prevFileIdRef.current !== fid) return;
        if (isRgb) {
          rgbLoadKeyRef.current = `${fid}|${path}`;
          loadRgbRawPixels(path, true);
        } else {
          loadRawPixels(true);
        }
      });
    }
  }, [file, gpuAvailable, useGpu, clearRawPixels, clearRgbRawPixels, loadRawPixels, loadRgbRawPixels, resetMouse]);

  useEffect(() => {
    const prevUrl = prevCompositeUrlRef.current;
    prevCompositeUrlRef.current = compositePreviewUrl;
    if (compositePreviewUrl === prevUrl) return;
    if (!file || !gpuAvailable || !useGpu) return;
    if (compositePreviewUrl) {
      const source = isFileRgbView ? file.path : null;
      const key = `${file.id}|${source ?? compositePreviewUrl}`;
      if (rgbLoadKeyRef.current === key) return;
      rgbLoadKeyRef.current = key;
      loadRgbRawPixels(source, true);
    } else {
      rgbLoadKeyRef.current = null;
      clearRgbRawPixels();
      loadRawPixels();
    }
  }, [compositePreviewUrl, file, gpuAvailable, useGpu, isFileRgbView, loadRgbRawPixels, clearRgbRawPixels, loadRawPixels]);

  const enableGpu = useCallback(() => {
    setUseGpu(true);
    if (compositePreviewUrl) {
      const source = isFileRgbView ? (file?.path ?? null) : null;
      rgbLoadKeyRef.current = `${file?.id}|${source ?? compositePreviewUrl}`;
      loadRgbRawPixels(source);
    } else if (file?.result?.is_rgb) {
      const r = file.result;
      if (r.stf_r && r.stf_g && r.stf_b) initRgb(r.previewUrl ?? null, r.stf_r, r.stf_g, r.stf_b);
      else if (r.previewUrl) setCompositePreviewUrl(r.previewUrl);
      rgbLoadKeyRef.current = `${file.id}|${file.path}`;
      loadRgbRawPixels(file.path ?? null, true);
    } else {
      loadRawPixels();
    }
  }, [compositePreviewUrl, isFileRgbView, file, loadRawPixels, loadRgbRawPixels, initRgb, setCompositePreviewUrl]);

  const handleToggleGpu = useCallback(() => {
    if (useGpu) {
      setUseGpu(false);
      rgbLoadKeyRef.current = null;
      clearRawPixels();
      clearRgbRawPixels();
      return;
    }
    if (gpuAvailable === false) {
      setGpuProbing(true);
      probeGpu().then(() => {
        const ok = isGpuAvailable() === true;
        setGpuAvailable(ok);
        setGpuReason(getGpuReason());
        setGpuProbing(false);
        if (ok) enableGpu();
      });
      return;
    }
    enableGpu();
  }, [useGpu, gpuAvailable, enableGpu, clearRawPixels, clearRgbRawPixels]);

  const extractSpectrum = useCallback(async (x: number, y: number) => {
    const path = file?.path;
    if (!path) return;
    const seq = ++specAbortRef.current;
    beginSpectrum({ x, y });
    const t0 = performance.now();
    try {
      const result = await getCubeSpectrum(path, x, y);
      if (specAbortRef.current !== seq) return;
      commitSpectrum(result, Math.round(performance.now() - t0));
    } catch (err) {
      if (specAbortRef.current !== seq) return;
      failSpectrum(err instanceof Error ? err.message : String(err));
    }
  }, [file?.path]);

  const handleImageClick = useCallback((e: React.MouseEvent<HTMLElement>) => {
    if (!isCube || !file?.path) return;
    const target = e.target as HTMLElement;
    if (!(target instanceof HTMLImageElement) && !(target instanceof HTMLCanvasElement)) return;
    const rect = target.getBoundingClientRect();
    const dims = file.result?.dimensions;
    if (!dims || rect.width <= 0 || rect.height <= 0) return;
    const pixelX = Math.floor(((e.clientX - rect.left) / rect.width) * dims[0]);
    const pixelY = Math.floor(((e.clientY - rect.top) / rect.height) * dims[1]);
    if (pixelX < 0 || pixelX >= dims[0] || pixelY < 0 || pixelY >= dims[1]) return;
    extractSpectrum(pixelX, pixelY);
  }, [isCube, file?.path, file?.result?.dimensions, extractSpectrum]);

  const pixelClick = usePixelClick();
  const spectrumClickSeqRef = useRef(0);
  useEffect(() => {
    if (!pixelClick || !isCube) return;
    if (pixelClick.seq === spectrumClickSeqRef.current) return;
    spectrumClickSeqRef.current = pixelClick.seq;
    extractSpectrum(pixelClick.x, pixelClick.y);
  }, [pixelClick, isCube, extractSpectrum]);

  const handlePreviewMouseMove = useCallback((e: React.MouseEvent<HTMLElement>) => { handleMove(e, fileDimsRef.current); }, [handleMove]);
  const handleViewerMousePixel = useCallback((x: number, y: number) => { setMousePixel({ x, y }); }, []);

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
            {file && <span className="text-[10px] font-mono text-zinc-400 truncate max-w-[200px]" title={file.name}>{file.name}</span>}
            {file?.result?.dimensions && (
              <span className="text-[10px] font-mono text-zinc-500 flex items-center gap-1.5 shrink-0">
                <span className="text-zinc-400">{file.result.dimensions[0]}&times;{file.result.dimensions[1]}</span>
                {file.result.header?.BITPIX && <span className="text-zinc-500">BITPIX {file.result.header.BITPIX}</span>}
                <span className="text-zinc-500">{(file.result.elapsed_ms / 1000).toFixed(2)}s</span>
              </span>
            )}
          </div>
          <div className="flex items-center gap-2 shrink-0">
            {file && (
              <button onClick={handleToggleGpu} disabled={gpuProbing || (gpuAvailable === false && !useGpu && !gpuSupported)}
                      title={gpuAvailable === false && gpuSupported ? `${gpuReason ?? "GPU unavailable"} — click to retry` : gpuReason ?? (useGpu ? "Rendering on GPU (WebGPU)" : "Rendering on CPU — click to use GPU")}
                      className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded transition-all duration-200 disabled:opacity-30 disabled:cursor-not-allowed"
                      style={useGpu ? { background: "rgba(168,85,247,0.15)", color: "#c084fc", border: "1px solid rgba(168,85,247,0.3)" } : { color: "#71717a", border: "1px solid transparent" }}>
                {gpuProbing ? <Loader2 size={10} className="animate-spin" /> : toggleLoading ? <Loader2 size={10} className="animate-spin" /> : useGpu ? <Zap size={10} /> : <Cpu size={10} />}
                {gpuProbing ? "..." : toggleLoading ? "..." : gpuAvailable === false ? "CPU" : useGpu ? "GPU" : "CPU"}
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
              onPixelClick={emitPixelClick}
              onMouseLeave={handleLeave}
              overlayCanvasRef={starOverlayRef}
            />
          ) : (
            <div className="h-full" onMouseMove={handlePreviewMouseMove} onMouseLeave={handleLeave}>
              <Suspense fallback={<TabSpinner />}>
                <PreviewTab useGpu={useGpu} rawPixels={rawPixels} rgbRawPixels={rgbRawPixels} onImageClick={handleImageClick} starOverlayRef={starOverlayRef} />
              </Suspense>
            </div>
          )}
        </div>

        {file && activeTool === "compose" && (
          <>
            <div className="ab-resize-handle-h" onMouseDown={handleBottomResize} />
            <div
              ref={bottomElRef}
              className="ab-bottom-panel"
              style={{ height: bottomHeightRef.current }}
            >
              <Suspense fallback={<TabSpinner />}>
                <ComposeWizard />
              </Suspense>
            </div>
          </>
        )}
      </div>

      {file && rightTool && (
        <div
          className="shrink-0 flex flex-col overflow-hidden"
          style={{ width: "min(380px, 42vw)", borderLeft: "1px solid rgba(20,184,166,0.08)", background: "rgba(5,5,16,0.55)" }}
        >
          <div className="flex-1 overflow-y-auto min-h-0">
            <Suspense fallback={<TabSpinner />}>
              <RightToolContent toolId={rightTool} starOverlayRef={starOverlayRef} />
            </Suspense>
          </div>
        </div>
      )}

      {file && (
        <div className="ab-tool-strip">
          {TOP_TOOLS.map((def) => {
            const Icon = def.icon;
            const isActive = rightTool === def.id;
            return (
              <button
                key={def.id}
                onClick={() => toggleRightTool(def.id)}
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
            const isActive = rightTool === def.id;
            return (
              <button
                key={def.id}
                onClick={() => toggleRightTool(def.id)}
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
