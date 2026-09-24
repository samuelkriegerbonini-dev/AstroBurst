import { useState, useCallback, useRef, useEffect, useMemo, lazy, Suspense } from "react";
import {
  Image, Cpu, Zap, Sparkles, Loader2, RotateCcw,
  Layers2, FlaskConical, Settings, Download, FileText, BarChart3,
} from "lucide-react";

import { getCubeSpectrum } from "../services/cube";
import { restretchComposite, updateCompositeChannel } from "../services/compose";
import { getOutputDir, getPreviewUrl } from "../infrastructure/tauri";
import { probeGpu, isGpuAvailable, onGpuLost, getGpuReason } from "../infrastructure/gpu/GpuSingleton";
import {
  fileKeyOf,
  useFileContext,
  useCubeContext,
  useDisplayedImage,
  useRawPixelsContext,
  useRenderActions,
  useRenderContext,
  useStarOverlayContext,
} from "../context/PreviewContext";
import { useCompositePreview, useCompositeActions, useCompositeStf } from "../context/CompositeContext";
import { useMousePixelActions, setMousePixel, emitPixelClick, usePixelClick } from "../hooks/useMousePixelStore";
import { useSpectrum, beginSpectrum, commitSpectrum, failSpectrum, resetSpectrum } from "../hooks/useSpectrumStore";
import AdvancedImageViewer from "./viewer/AdvancedImageViewer";
import { loadLayout, saveLayout } from "../utils/layout";
import { loadGpuPreference, saveGpuPreference } from "../utils/gpuPreference";
import { advanceCompositeSync, compositeSyncStore, forgetCompositeSync, syncedChannelFor, wizardStepStaleAfterChannelSync } from "../utils/compositeSync";
import { useComposeWizardContext } from "../context/ComposeWizardContext";
import { parseImageRef, planeLabel } from "../utils/imageRef";
import { useRightTool, rightToolStore } from "../hooks/useRightTool";
import type { ToolId, RightToolId } from "../hooks/useRightTool";
import DqControls from "./preview/DqControls";
import DqOverlayCanvas from "./preview/DqOverlayCanvas";

const PreviewTab = lazy(() => import("./preview/PreviewTab"));
const ProcessingTab = lazy(() => import("./processing/ProcessingTab"));
const ComposeWizard = lazy(() => import("./compose/ComposeWizard"));
const StackingTab = lazy(() => import("./stacking/StackingTab"));
const ConfigTab = lazy(() => import("./preview/ConfigTab"));
const SynthPanel = lazy(() => import("./synth/SynthPanel"));
const ExportTab = lazy(() => import("./export/ExportTab"));
const AnalysisTab = lazy(() => import("./analysis/AnalysisTab"));
const HeadersTab = lazy(() => import("./header/HeadersTab"));

export type { ToolId, RightToolId };

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

const RIGHT_MIN = 280;
const RIGHT_MAX = 640;
const RIGHT_DEFAULT = 380;

const MIN_PREVIEW_W = 320;
const MIN_PREVIEW_H = 200;

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

export interface PreviewPanelProps {
  activeTool: ToolId | null;
}

export default function PreviewPanel({ activeTool }: PreviewPanelProps) {
  const { file } = useFileContext();
  const { isCube } = useCubeContext();
  const { rawPixels, rawPixelsLoading, rawPixelsError, loadRawPixels, clearRawPixels,
          rgbRawPixels, rgbRawPixelsLoading, loadRgbRawPixels, clearRgbRawPixels } = useRawPixelsContext();
  const { processed, processedVersion, stfPreviewUrl, chain } = useRenderContext();
  const { resetProcessed } = useRenderActions();
  const displayed = useDisplayedImage();
  const processedSourcePath = processed?.fitsPath ?? null;
  const processedSourceVersion = processedSourcePath ? processedVersion : 0;
  const { compositePreviewUrl, compositeVersion } = useCompositePreview();
  const { initRgb, setCompositePreviewUrl, clearComposite, resetComposite } = useCompositeActions();
  const { compositeStfR, compositeStfG, compositeStfB, compositeStfLinked } = useCompositeStf();
  const { state: wizardState, dispatch: wizardDispatch } = useComposeWizardContext();
  const wizardReadyRef = useRef(wizardState.compositeReady);
  wizardReadyRef.current = wizardState.compositeReady;
  const { starOverlayRef } = useStarOverlayContext();
  const { handleLeave, reset: resetMouse } = useMousePixelActions();

  const [gpuPref] = useState(() => loadGpuPreference());
  const [useGpu, setUseGpu] = useState(gpuPref ?? false);
  const [gpuAvailable, setGpuAvailable] = useState<boolean | null>(null);
  const [gpuProbing, setGpuProbing] = useState(true);
  const [gpuReason, setGpuReason] = useState<string | null>(null);
  const [, forceRender] = useState(0);
  const rightTool = useRightTool();
  const toggleRightTool = useCallback((id: RightToolId) => {
    rightToolStore.toggle(id);
  }, []);

  const [rightMounted, setRightMounted] = useState(false);
  const lastToolRef = useRef<RightToolId | null>(null);
  if (rightTool) lastToolRef.current = rightTool;
  const displayTool = rightTool ?? lastToolRef.current;
  useEffect(() => { if (rightTool) setRightMounted(true); }, [rightTool]);
  const handleRightTransitionEnd = useCallback((e: React.TransitionEvent) => {
    if (e.target !== e.currentTarget || e.propertyName !== "width") return;
    if (!rightTool) setRightMounted(false);
  }, [rightTool]);

  const bottomOpen = activeTool === "compose";
  const [bottomMounted, setBottomMounted] = useState(false);
  useEffect(() => { if (bottomOpen) setBottomMounted(true); }, [bottomOpen]);
  const handleBottomTransitionEnd = useCallback((e: React.TransitionEvent) => {
    if (e.target !== e.currentTarget || e.propertyName !== "height") return;
    if (!bottomOpen) setBottomMounted(false);
  }, [bottomOpen]);

  const prevFileKeyRef = useRef<string | null>(null);
  const rgbLoadKeyRef = useRef<string | null>(null);
  const gpuLoadKeyRef = useRef<string | null>(null);
  const dqCanvasRef = useRef<HTMLCanvasElement>(null);
  const specAbortRef = useRef(0);

  const fileKey = fileKeyOf(file);
  const isRgbFile = !!file?.result?.is_rgb;
  const isFileRgbView = compositePreviewUrl !== null && isRgbFile && compositePreviewUrl === (file?.result?.previewUrl ?? null);
  const monoOverRgb = isFileRgbView && processed !== null;
  const isRgbView = compositePreviewUrl !== null && !monoOverRgb;
  const toggleLoading = isRgbView ? rgbRawPixelsLoading : rawPixelsLoading;
  const gpuLoadError = useGpu && !isRgbView && !rawPixelsLoading ? rawPixelsError : null;

  const bottomHeightRef = useRef(loadLayout("bottomH", BOTTOM_DEFAULT, BOTTOM_MIN, BOTTOM_MAX));
  const bottomElRef = useRef<HTMLDivElement>(null);
  const bottomOuterRef = useRef<HTMLDivElement>(null);
  const bResizing = useRef(false);
  const bStartY = useRef(0);
  const bStartH = useRef(0);

  const rightWidthRef = useRef(loadLayout("rightW", RIGHT_DEFAULT, RIGHT_MIN, RIGHT_MAX));
  const rightElRef = useRef<HTMLDivElement>(null);
  const rightOuterRef = useRef<HTMLDivElement>(null);
  const rResizing = useRef(false);
  const rStartX = useRef(0);
  const rStartW = useRef(0);

  const centerColRef = useRef<HTMLDivElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const rightToolOpenRef = useRef(false);
  rightToolOpenRef.current = rightTool !== null;
  const bottomOpenRef = useRef(false);
  bottomOpenRef.current = bottomOpen;

  const applyRightWidth = useCallback((width: number) => {
    const el = rightElRef.current;
    if (el) el.style.width = `min(${width}px, 60vw)`;
    const outer = rightOuterRef.current;
    if (outer) outer.style.width = `min(${width}px, 60vw)`;
  }, []);

  const applyBottomHeight = useCallback((height: number) => {
    const el = bottomElRef.current;
    if (el) el.style.height = `${height}px`;
    const outer = bottomOuterRef.current;
    if (outer) outer.style.height = `${height}px`;
  }, []);

  useEffect(() => {
    const center = centerColRef.current;
    if (!center) return;
    let raf: number | null = null;
    const clamp = () => {
      raf = null;
      if (rResizing.current || !rightToolOpenRef.current) return;
      const deficit = MIN_PREVIEW_W - center.clientWidth;
      if (deficit <= 1) return;
      const next = Math.max(RIGHT_MIN, rightWidthRef.current - deficit);
      if (next === rightWidthRef.current) return;
      rightWidthRef.current = next;
      applyRightWidth(next);
      saveLayout("rightW", next);
      forceRender((c) => c + 1);
    };
    const ro = new ResizeObserver(() => {
      if (raf === null) raf = requestAnimationFrame(clamp);
    });
    ro.observe(center);
    return () => {
      ro.disconnect();
      if (raf !== null) cancelAnimationFrame(raf);
    };
  }, [applyRightWidth]);

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    let raf: number | null = null;
    const clamp = () => {
      raf = null;
      if (bResizing.current || !bottomOpenRef.current) return;
      const deficit = MIN_PREVIEW_H - viewport.clientHeight;
      if (deficit <= 1) return;
      const next = Math.max(BOTTOM_MIN, bottomHeightRef.current - deficit);
      if (next === bottomHeightRef.current) return;
      bottomHeightRef.current = next;
      applyBottomHeight(next);
      saveLayout("bottomH", next);
      forceRender((c) => c + 1);
    };
    const ro = new ResizeObserver(() => {
      if (raf === null) raf = requestAnimationFrame(clamp);
    });
    ro.observe(viewport);
    return () => {
      ro.disconnect();
      if (raf !== null) cancelAnimationFrame(raf);
    };
  }, [applyBottomHeight]);

  useEffect(() => {
    probeGpu().then(() => {
      const ok = isGpuAvailable() === true;
      setGpuAvailable(ok);
      setGpuReason(getGpuReason());
      setGpuProbing(false);
      if (ok && gpuPref === null) setUseGpu(true);
    });
  }, [gpuPref]);

  useEffect(() => {
    const unsub = onGpuLost(() => {
      setGpuAvailable(false);
      setGpuReason(getGpuReason());
      setUseGpu(false);
      rgbLoadKeyRef.current = null;
      gpuLoadKeyRef.current = null;
      clearRawPixels();
      clearRgbRawPixels();
    });
    return unsub;
  }, [clearRawPixels, clearRgbRawPixels]);

  useEffect(() => {
    if (fileKey === prevFileKeyRef.current) return;
    prevFileKeyRef.current = fileKey;
    specAbortRef.current++;
    resetSpectrum();
    resetMouse();
    rgbLoadKeyRef.current = null;
    gpuLoadKeyRef.current = null;
    clearRawPixels();
    clearRgbRawPixels();
  }, [fileKey, clearRawPixels, clearRgbRawPixels, resetMouse]);

  const showRgbFileView = useCallback(() => {
    const r = file?.result;
    if (!r?.is_rgb) return;
    if (r.stf_r && r.stf_g && r.stf_b) initRgb(r.previewUrl ?? null, r.stf_r, r.stf_g, r.stf_b);
    else if (r.previewUrl) setCompositePreviewUrl(r.previewUrl);
  }, [file?.result, initRgb, setCompositePreviewUrl]);

  useEffect(() => {
    if (!isRgbFile) return;
    if (processed !== null) {
      if (isFileRgbView) resetComposite();
      return;
    }
    if (compositePreviewUrl === null) showRgbFileView();
  }, [isRgbFile, processed, isFileRgbView, compositePreviewUrl, resetComposite, showRgbFileView]);

  const wantGpu = !!gpuAvailable && useGpu;
  const filePath = file?.path ?? null;
  const monoLoadKey = fileKey ? `${fileKey}|${processedSourcePath ?? filePath}|${processedSourceVersion}` : null;
  const rgbSource = isFileRgbView ? filePath : null;
  const rgbLoadKey = fileKey && isRgbView ? `${fileKey}|${rgbSource ?? ""}|${compositeVersion}` : null;
  const rgbFileViewPending = isRgbFile && processed === null && compositePreviewUrl === null && !!file?.result?.previewUrl;
  const compositeOwnerRef = useRef<{ key: string | null; version: number } | null>(null);

  useEffect(() => {
    const owner = compositeOwnerRef.current;
    if (!owner || owner.version !== compositeVersion) compositeOwnerRef.current = { key: fileKey, version: compositeVersion };
    if (!wantGpu || !fileKey) return;
    if (compositeOwnerRef.current?.key !== fileKey || rgbFileViewPending) return;
    if (isRgbView) {
      if (!rgbLoadKey || rgbLoadKeyRef.current === rgbLoadKey) return;
      rgbLoadKeyRef.current = rgbLoadKey;
      queueMicrotask(() => {
        if (rgbLoadKeyRef.current === rgbLoadKey) loadRgbRawPixels(rgbSource, true);
      });
      return;
    }
    if (rgbLoadKeyRef.current !== null) {
      rgbLoadKeyRef.current = null;
      clearRgbRawPixels();
    }
    if (!monoLoadKey || gpuLoadKeyRef.current === monoLoadKey) return;
    gpuLoadKeyRef.current = monoLoadKey;
    queueMicrotask(() => {
      if (gpuLoadKeyRef.current === monoLoadKey) loadRawPixels(true);
    });
  }, [wantGpu, fileKey, compositeVersion, rgbFileViewPending, isRgbView, rgbLoadKey, rgbSource, monoLoadKey, loadRawPixels, loadRgbRawPixels, clearRgbRawPixels]);

  const enableGpu = useCallback(() => {
    setUseGpu(true);
  }, []);

  const handleBackToFile = useCallback(() => {
    if (isRgbFile && processed === null) {
      showRgbFileView();
      return;
    }
    void clearComposite();
  }, [isRgbFile, processed, showRgbFileView, clearComposite]);

  const liveCompositeUrlRef = useRef(compositePreviewUrl);
  liveCompositeUrlRef.current = compositePreviewUrl;
  const canReset = processed !== null || chain.psfKernel !== null;
  const handleResetProcessed = useCallback(() => {
    const path = file?.path ?? null;
    const channel = path && fileKey ? syncedChannelFor(compositeSyncStore.get(), compositePreviewUrl, fileKey) : null;
    resetProcessed();
    if (!path || !channel) return;
    compositeSyncStore.set(forgetCompositeSync(compositeSyncStore.get(), channel));
    const stf = { r: compositeStfR, g: compositeStfG, b: compositeStfB, linked: compositeStfLinked };
    (async () => {
      try {
        await updateCompositeChannel(channel, path);
        const staleStep = wizardStepStaleAfterChannelSync(wizardReadyRef.current);
        if (staleStep) wizardDispatch({ type: "INVALIDATE_FROM", stepId: staleStep });
        const dir = await getOutputDir();
        const result = await restretchComposite(dir, stf.r, stf.g, stf.b, undefined, undefined, stf.linked);
        if (!result?.png_path) return;
        const url = compositeSyncStore.tagUrl(await getPreviewUrl(result.png_path));
        compositeSyncStore.set(advanceCompositeSync(compositeSyncStore.get(), liveCompositeUrlRef.current, url));
        setCompositePreviewUrl(url);
      } catch (e) {
        console.error("[AstroBurst] Composite channel restore failed:", e);
      }
    })();
  }, [file?.path, fileKey, compositePreviewUrl, resetProcessed, compositeStfR, compositeStfG, compositeStfB, compositeStfLinked, setCompositePreviewUrl, wizardDispatch]);

  const handleToggleGpu = useCallback(() => {
    if (useGpu) {
      setUseGpu(false);
      saveGpuPreference(false);
      rgbLoadKeyRef.current = null;
      gpuLoadKeyRef.current = null;
      clearRawPixels();
      clearRgbRawPixels();
      return;
    }
    saveGpuPreference(true);
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

  const handleViewerMousePixel = useCallback((x: number, y: number) => { setMousePixel({ x, y }); }, []);

  const handleBottomResize = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    bResizing.current = true;
    bStartY.current = e.clientY;
    bStartH.current = bottomHeightRef.current;
    document.body.style.cursor = "row-resize";
    document.body.style.userSelect = "none";
    const handle = e.currentTarget as HTMLElement;
    handle.dataset.dragging = "true";
    const el = bottomElRef.current;
    const outer = bottomOuterRef.current;
    if (outer) outer.style.transition = "none";
    const viewportH = viewportRef.current?.clientHeight;
    const maxHeight = viewportH === undefined
      ? BOTTOM_MAX
      : Math.min(BOTTOM_MAX, Math.max(BOTTOM_MIN, bStartH.current + viewportH - MIN_PREVIEW_H));
    const onMove = (ev: MouseEvent) => {
      if (!bResizing.current) return;
      const next = Math.max(BOTTOM_MIN, Math.min(maxHeight, bStartH.current - (ev.clientY - bStartY.current)));
      bottomHeightRef.current = next;
      if (el) el.style.height = `${next}px`;
      if (outer) outer.style.height = `${next}px`;
    };
    const onUp = () => {
      bResizing.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      delete handle.dataset.dragging;
      if (outer) outer.style.transition = "";
      saveLayout("bottomH", bottomHeightRef.current);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      forceRender((c) => c + 1);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, []);

  const handleBottomReset = useCallback(() => {
    bottomHeightRef.current = BOTTOM_DEFAULT;
    saveLayout("bottomH", BOTTOM_DEFAULT);
    const el = bottomElRef.current;
    if (el) el.style.height = `${BOTTOM_DEFAULT}px`;
    const outer = bottomOuterRef.current;
    if (outer) outer.style.height = `${BOTTOM_DEFAULT}px`;
    forceRender((c) => c + 1);
  }, []);

  const handleRightResize = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    rResizing.current = true;
    rStartX.current = e.clientX;
    rStartW.current = rightWidthRef.current;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    const handle = e.currentTarget as HTMLElement;
    handle.dataset.dragging = "true";
    const el = rightElRef.current;
    const outer = rightOuterRef.current;
    if (outer) outer.style.transition = "none";
    const centerW = centerColRef.current?.clientWidth;
    const maxWidth = centerW === undefined
      ? RIGHT_MAX
      : Math.min(RIGHT_MAX, Math.max(RIGHT_MIN, rStartW.current + centerW - MIN_PREVIEW_W));
    const onMove = (ev: MouseEvent) => {
      if (!rResizing.current) return;
      const next = Math.max(RIGHT_MIN, Math.min(maxWidth, rStartW.current - (ev.clientX - rStartX.current)));
      rightWidthRef.current = next;
      if (el) el.style.width = `min(${next}px, 60vw)`;
      if (outer) outer.style.width = `min(${next}px, 60vw)`;
    };
    const onUp = () => {
      rResizing.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      delete handle.dataset.dragging;
      if (outer) outer.style.transition = "";
      saveLayout("rightW", rightWidthRef.current);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      forceRender((c) => c + 1);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, []);

  const handleRightReset = useCallback(() => {
    rightWidthRef.current = RIGHT_DEFAULT;
    saveLayout("rightW", RIGHT_DEFAULT);
    const el = rightElRef.current;
    if (el) el.style.width = `min(${RIGHT_DEFAULT}px, 60vw)`;
    const outer = rightOuterRef.current;
    if (outer) outer.style.width = `min(${RIGHT_DEFAULT}px, 60vw)`;
    forceRender((c) => c + 1);
  }, []);

  const originalImage = useMemo(() => {
    if (!file?.result?.previewUrl) return null;
    const base = file.result.previewUrl;
    const sep = base.includes("?") ? "&" : "?";
    return { url: `${base}${sep}_v=${file.id}`, label: "Original", width: file.result.dimensions?.[0], height: file.result.dimensions?.[1] };
  }, [file?.result?.previewUrl, file?.result?.dimensions, file?.id]);

  const displayedW = displayed.dimensions?.[0];
  const displayedH = displayed.dimensions?.[1];
  const displayedLabel = displayed.label;
  const displayedPreviewOnly = displayed.previewOnly;
  const processedPreviewUrl = processed?.previewUrl ?? null;
  const processedImage = useMemo(() => {
    if (stfPreviewUrl) {
      return { url: stfPreviewUrl, label: `${displayedLabel ?? "Original"} · STF`, width: displayedW, height: displayedH };
    }
    if (!processedPreviewUrl || !displayedLabel) return null;
    const label = displayedPreviewOnly ? `${displayedLabel} · PNG` : displayedLabel;
    return { url: processedPreviewUrl, label, width: displayedW, height: displayedH };
  }, [stfPreviewUrl, processedPreviewUrl, displayedLabel, displayedPreviewOnly, displayedW, displayedH]);

  const useAdvancedViewer = !isRgbView && !useGpu;

  const planeBadge = file ? planeLabel(parseImageRef(file.path), file.result?.plane?.extname) : null;

  return (
    <div className="flex h-full overflow-hidden">
      <div ref={centerColRef} className="flex-1 min-w-0 flex flex-col overflow-hidden">

        <div className="flex items-center justify-between px-3 py-1 shrink-0" style={{ background: "rgba(5,5,16,0.6)", borderBottom: "1px solid var(--ab-border)" }}>
          <div className="flex items-center gap-2 shrink-0">
            <Image size={12} style={{ color: "var(--ab-teal)" }} />
            <span className="text-[11px] font-medium text-zinc-300">Preview</span>
          </div>
          <div className="flex items-center gap-2 justify-center flex-1 min-w-0">
            {file && <span className="text-[10px] font-mono text-zinc-400 truncate max-w-[200px]" title={file.name}>{file.name}</span>}
            {planeBadge && (
              <span
                className="text-[9px] font-mono px-1.5 py-px rounded shrink-0"
                style={{ background: "rgba(20,184,166,0.12)", color: "var(--ab-teal)", border: "1px solid rgba(20,184,166,0.3)" }}
                title={file?.path}
              >
                {planeBadge}
              </span>
            )}
            {file?.result?.dimensions && (
              <span className="text-[10px] font-mono text-zinc-500 flex items-center gap-1.5 shrink-0">
                <span className="text-zinc-400">{file.result.dimensions[0]}&times;{file.result.dimensions[1]}</span>
                {file.result.header?.BITPIX && <span className="text-zinc-500">BITPIX {file.result.header.BITPIX}</span>}
                <span className="text-zinc-500">{(file.result.elapsed_ms / 1000).toFixed(2)}s</span>
              </span>
            )}
          </div>
          <div className="flex items-center gap-2 shrink-0">
            {file && <DqControls />}
            {file && <DqOverlayCanvas canvasRef={dqCanvasRef} />}
            {file && canReset && (
              <button
                onClick={handleResetProcessed}
                className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded text-zinc-400 hover:text-zinc-200 transition-colors"
                style={{ border: "1px solid var(--ab-border)" }}
                title={processed ? `Showing ${processed.label}. Reset to the original image` : "Clear the processing chain of this file"}
              >
                <RotateCcw size={10} />
                Reset
              </button>
            )}
            {file && (
              <button onClick={handleToggleGpu} disabled={gpuProbing || (gpuAvailable === false && !useGpu && !gpuSupported)}
                      title={gpuLoadError ? `GPU image load failed: ${gpuLoadError} — showing the PNG preview; click to switch to CPU` : gpuAvailable === false && gpuSupported ? `${gpuReason ?? "GPU unavailable"} — click to retry` : gpuReason ?? (useGpu ? "Rendering on GPU (WebGPU)" : "Rendering on CPU — click to use GPU")}
                      className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded transition-all duration-200 disabled:opacity-30 disabled:cursor-not-allowed"
                      style={gpuLoadError ? { background: "rgba(245,158,11,0.15)", color: "#fbbf24", border: "1px solid rgba(245,158,11,0.4)" } : useGpu ? { background: "rgba(168,85,247,0.15)", color: "#c084fc", border: "1px solid rgba(168,85,247,0.3)" } : { color: "#71717a", border: "1px solid transparent" }}>
                {gpuProbing ? <Loader2 size={10} className="animate-spin" /> : toggleLoading ? <Loader2 size={10} className="animate-spin" /> : useGpu ? <Zap size={10} /> : <Cpu size={10} />}
                {gpuProbing ? "..." : toggleLoading ? "..." : gpuAvailable === false ? "CPU" : gpuLoadError ? "GPU failed" : useGpu ? "GPU" : "CPU"}
              </button>
            )}
          </div>
        </div>

        <div ref={viewportRef} className="flex-1 overflow-hidden min-h-0">
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
              dqCanvasRef={dqCanvasRef}
            />
          ) : (
            <div className="h-full" onMouseLeave={handleLeave}>
              <Suspense fallback={<TabSpinner />}>
                <PreviewTab
                  useGpu={useGpu}
                  rawPixels={rawPixels}
                  rgbRawPixels={rgbRawPixels}
                  onImageClick={handleImageClick}
                  onBackToFile={handleBackToFile}
                  starOverlayRef={starOverlayRef}
                  dqCanvasRef={dqCanvasRef}
                />
              </Suspense>
            </div>
          )}
        </div>

        {file && (
          <>
            {bottomOpen && (
              <div
                className="ab-resize-handle-h"
                onMouseDown={handleBottomResize}
                onDoubleClick={handleBottomReset}
                title="Drag to resize — double-click to reset"
              />
            )}
            <div
              ref={bottomOuterRef}
              className="shrink-0 relative overflow-hidden ab-panel-anim-h"
              style={{ height: bottomOpen ? bottomHeightRef.current : 0 }}
              onTransitionEnd={handleBottomTransitionEnd}
              aria-hidden={!bottomOpen}
            >
              <div
                ref={bottomElRef}
                inert={!bottomOpen}
                className="ab-bottom-panel absolute inset-x-0 bottom-0"
                style={{ height: bottomHeightRef.current }}
              >
                {(bottomOpen || bottomMounted) && (
                  <Suspense fallback={<TabSpinner />}>
                    <ComposeWizard />
                  </Suspense>
                )}
              </div>
            </div>
          </>
        )}
      </div>

      {file && (
        <>
          {rightTool && (
            <div
              className="ab-resize-handle"
              onMouseDown={handleRightResize}
              onDoubleClick={handleRightReset}
              title="Drag to resize — double-click to reset"
            />
          )}
          <div
            ref={rightOuterRef}
            className="shrink-0 relative overflow-hidden ab-panel-anim-w"
            style={{ width: rightTool ? `min(${rightWidthRef.current}px, 60vw)` : 0 }}
            onTransitionEnd={handleRightTransitionEnd}
            aria-hidden={!rightTool}
          >
            <div
              ref={rightElRef}
              inert={!rightTool}
              className="absolute inset-y-0 left-0 flex flex-col overflow-hidden"
              style={{ width: `min(${rightWidthRef.current}px, 60vw)`, borderLeft: "1px solid var(--ab-border)", background: "rgba(5,5,16,0.55)" }}
            >
              <div className="flex-1 overflow-y-auto min-h-0">
                {(rightTool || rightMounted) && displayTool && (
                  <div key={displayTool} className="ab-tool-fade">
                    <Suspense fallback={<TabSpinner />}>
                      <RightToolContent toolId={displayTool} starOverlayRef={starOverlayRef} />
                    </Suspense>
                  </div>
                )}
              </div>
            </div>
          </div>
        </>
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
