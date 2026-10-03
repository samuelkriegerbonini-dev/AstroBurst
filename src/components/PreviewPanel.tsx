import { useState, useCallback, useRef, useEffect, useLayoutEffect, useMemo } from "react";
import { Image, Cpu, Zap, Loader2, SkipBack, Palette } from "lucide-react";

import { getCubeSpectrum } from "../services/cube";
import { getRampPixelSeries, rampSpectrumFromSeries } from "../services/ramp";
import { useRampIntegration } from "../hooks/useRampStore";
import { rampPixelTarget } from "../utils/rampLabels";
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
import { useCompositePreview, useCompositeActions } from "../context/CompositeContext";
import { useMousePixelActions, setMousePixel, emitPixelClick, usePixelClick } from "../hooks/useMousePixelStore";
import { beginSpectrum, commitSpectrum, failSpectrum, resetSpectrum } from "../hooks/useSpectrumStore";
import { useCompositeChain, useCompositeChainSync } from "../hooks/useCompositeChain";
import { useCompositeMode } from "../hooks/useCompositeMode";
import { beginCompositeCheck, useRunLocked } from "../hooks/useProcessingRun";
import { compositeChainReset } from "../services/compositeChain";
import { compositeChainStore } from "../utils/compositeChainStore";
import { COMPOSITE_RUN_KEY, compositeResultTarget, hasCompositeReset, lastCompositeStep } from "../utils/compositeChain";
import AdvancedImageViewer from "./viewer/AdvancedImageViewer";
import { loadGpuPreference, saveGpuPreference } from "../utils/gpuPreference";
import { monoPixelsAction } from "../utils/gpuMonoPixels";
import { useComposeWizardContext } from "../context/ComposeWizardContext";
import { parseImageRef, planeLabel } from "../utils/imageRef";
import { gpuDisplayStore } from "../hooks/useGpuDisplay";
import DqControls from "./preview/DqControls";
import DqOverlayCanvas from "./preview/DqOverlayCanvas";
import ViewerStatusStrip from "./preview/ViewerStatusStrip";
import DisplayControls from "./preview/DisplayControls";
import PreviewTab from "./preview/PreviewTab";
import {
  CUBE_SPECTRUM_HINT,
  backToFileAction,
  cpuViewerDisplayTitle,
  gpuAfterProbe,
  gpuDisplayOnScreen,
  gpuToggleView,
  previewViewer,
  type GpuToggleTone,
} from "../utils/previewShell";

const gpuSupported = typeof navigator !== "undefined" && !!navigator.gpu;

const GPU_TOGGLE_STYLES: Record<GpuToggleTone, React.CSSProperties> = {
  failed: { background: "rgba(245,158,11,0.15)", color: "#fbbf24", border: "1px solid rgba(245,158,11,0.4)" },
  gpu: { background: "rgba(168,85,247,0.15)", color: "#c084fc", border: "1px solid rgba(168,85,247,0.3)" },
  muted: { background: "transparent", color: "rgba(192,132,252,0.55)", border: "1px dashed rgba(168,85,247,0.35)" },
  off: { color: "#71717a", border: "1px solid transparent" },
};

export default function PreviewPanel() {
  const { file } = useFileContext();
  const { isCube, ramp } = useCubeContext();
  const rampIntegration = useRampIntegration();
  const { rawPixels, rawPixelsLoading, rawPixelsError, loadRawPixels, clearRawPixels,
          rgbRawPixels, rgbRawPixelsLoading, loadRgbRawPixels, clearRgbRawPixels } = useRawPixelsContext();
  const { processed, processedVersion, stfPreviewUrl, chain } = useRenderContext();
  const { resetProcessed, currentFileKey } = useRenderActions();
  const displayed = useDisplayedImage();
  const processedSourcePath = processed?.fitsPath ?? null;
  const processedSourceVersion = processedSourcePath ? processedVersion : 0;
  const { compositePreviewUrl, compositeVersion, canShowParked } = useCompositePreview();
  const { initRgb, setCompositePreviewUrl, clearComposite, resetComposite, setCompositeStf, setCompositeAutoStf, setCompositeStfLinked, park, replaceParked, showParkedComposite } = useCompositeActions();
  const compositeMode = useCompositeMode();
  const compositeChain = useCompositeChain();
  useCompositeChainSync();
  const compositeRunning = useRunLocked(COMPOSITE_RUN_KEY);
  const { state: wizardState } = useComposeWizardContext();
  const wizardReadyRef = useRef(wizardState.compositeReady);
  wizardReadyRef.current = wizardState.compositeReady;
  const { starOverlayRef } = useStarOverlayContext();
  const { handleLeave, reset: resetMouse } = useMousePixelActions();

  const [gpuPref] = useState(() => loadGpuPreference());
  const [useGpu, setUseGpu] = useState(gpuPref ?? false);
  const [gpuAvailable, setGpuAvailable] = useState<boolean | null>(null);
  const [gpuProbing, setGpuProbing] = useState(true);
  const [gpuReason, setGpuReason] = useState<string | null>(null);

  const prevFileKeyRef = useRef<string | null>(null);
  const rgbLoadKeyRef = useRef<string | null>(null);
  const gpuLoadKeyRef = useRef<string | null>(null);
  const dqCanvasRef = useRef<HTMLCanvasElement>(null);
  const specAbortRef = useRef(0);
  const rampPixelRef = useRef<{ path: string; x: number; y: number } | null>(null);
  const lastRampIntegrationRef = useRef(rampIntegration);

  const fileKey = fileKeyOf(file);
  const isRgbFile = !!file?.result?.is_rgb;
  const isFileRgbView = compositePreviewUrl !== null && isRgbFile && compositePreviewUrl === (file?.result?.previewUrl ?? null);
  const monoOverRgb = isFileRgbView && processed !== null;
  const isRgbView = compositePreviewUrl !== null && !monoOverRgb;
  const toggleLoading = isRgbView ? rgbRawPixelsLoading : rawPixelsLoading;
  const gpuLoadError = useGpu && !isRgbView && !rawPixelsLoading ? rawPixelsError : null;

  const gpuDisplayInput = { hasFile: !!file, rgbView: isRgbView, useGpu, hasRawPixels: rawPixels !== null, previewOnly: displayed.previewOnly };
  const gpuDisplay = gpuDisplayOnScreen(gpuDisplayInput);
  const viewer = previewViewer(gpuDisplayInput);
  useLayoutEffect(() => gpuDisplayStore.set(gpuDisplay), [gpuDisplay]);
  useLayoutEffect(() => () => gpuDisplayStore.set(false), []);

  useEffect(() => {
    probeGpu().then(() => {
      const ok = isGpuAvailable() === true;
      setGpuAvailable(ok);
      setGpuReason(getGpuReason());
      setGpuProbing(false);
      setUseGpu((current) => gpuAfterProbe(ok, gpuPref, current));
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
  const pngOnlyRecord = displayed.previewOnly;
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
    const action = monoPixelsAction(pngOnlyRecord, monoLoadKey, gpuLoadKeyRef.current, isCube);
    if (action === "clear") {
      gpuLoadKeyRef.current = null;
      clearRawPixels();
      return;
    }
    if (action === "keep") return;
    if (action === "reload") clearRawPixels();
    gpuLoadKeyRef.current = monoLoadKey;
    queueMicrotask(() => {
      if (gpuLoadKeyRef.current === monoLoadKey) loadRawPixels(true);
    });
  }, [wantGpu, fileKey, compositeVersion, rgbFileViewPending, isRgbView, rgbLoadKey, rgbSource, monoLoadKey, pngOnlyRecord, isCube, loadRawPixels, clearRawPixels, loadRgbRawPixels, clearRgbRawPixels]);

  const enableGpu = useCallback(() => {
    setUseGpu(true);
  }, []);

  const handleBackToFile = useCallback(() => {
    const action = backToFileAction({ isRgbFile, hasProcessed: processed !== null, wizardCompositeReady: wizardReadyRef.current });
    if (action === "rgb-file") {
      park();
      showRgbFileView();
      return;
    }
    if (action === "display-only") {
      park();
      return;
    }
    void clearComposite();
  }, [isRgbFile, processed, showRgbFileView, park, clearComposite]);

  const showParked = !!file && canShowParked && wizardState.compositeReady;
  const handleShowComposite = useCallback(() => {
    void showParkedComposite();
  }, [showParkedComposite]);

  const canReset = processed !== null || chain.psfKernel !== null;

  const compositeLastStep = lastCompositeStep(compositeChain);
  const compositeLastLabel = compositeLastStep ? compositeChain.steps[compositeLastStep]?.label ?? null : null;
  const canCompositeReset = compositeMode && hasCompositeReset(compositeChain);
  const handleCompositeReset = useCallback(() => {
    const base = compositeChainStore.get().base;
    compositeChainStore.reset();
    if (!base) return;
    const stillSameComposite = beginCompositeCheck(currentFileKey);
    (async () => {
      try {
        const res = await compositeChainReset();
        const target = compositeResultTarget(res.restored, stillSameComposite());
        if (target === "parked") replaceParked(base.previewUrl, base.stf);
        if (target !== "screen") return;
        setCompositeAutoStf(base.stf.r, base.stf.g, base.stf.b);
        setCompositeStf(base.stf.r, base.stf.g, base.stf.b);
        setCompositeStfLinked(base.stf.linked);
        setCompositePreviewUrl(base.previewUrl);
      } catch (e) {
        console.error("[AstroBurst] Composite chain reset failed:", e);
      }
    })();
  }, [currentFileKey, setCompositeAutoStf, setCompositeStf, setCompositeStfLinked, setCompositePreviewUrl, replaceParked]);

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
    if (gpuAvailable === false) {
      setGpuProbing(true);
      probeGpu().then(() => {
        const ok = isGpuAvailable() === true;
        setGpuAvailable(ok);
        setGpuReason(getGpuReason());
        setGpuProbing(false);
        if (!ok) return;
        saveGpuPreference(true);
        enableGpu();
      });
      return;
    }
    saveGpuPreference(true);
    enableGpu();
  }, [useGpu, gpuAvailable, enableGpu, clearRawPixels, clearRgbRawPixels]);

  const loadSpectrum = useCallback(async (path: string, x: number, y: number, integration: number | null) => {
    const seq = ++specAbortRef.current;
    beginSpectrum({ x, y });
    const t0 = performance.now();
    try {
      const result = integration !== null
        ? rampSpectrumFromSeries(await getRampPixelSeries(path, x, y, integration))
        : await getCubeSpectrum(path, x, y);
      if (specAbortRef.current !== seq) return;
      commitSpectrum(result, Math.round(performance.now() - t0));
    } catch (err) {
      if (specAbortRef.current !== seq) return;
      failSpectrum(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const displayedDims = displayed.dimensions;
  const extractSpectrum = useCallback((x: number, y: number) => {
    const path = file?.path;
    if (!path) return;
    if (!ramp) {
      rampPixelRef.current = null;
      void loadSpectrum(path, x, y, null);
      return;
    }
    const target = rampPixelTarget(x, y, ramp, displayedDims);
    if (!target.ok) {
      specAbortRef.current++;
      failSpectrum(target.reason);
      return;
    }
    rampPixelRef.current = { path, x: target.x, y: target.y };
    void loadSpectrum(path, target.x, target.y, rampIntegration);
  }, [file?.path, ramp, displayedDims, rampIntegration, loadSpectrum]);

  useEffect(() => {
    if (lastRampIntegrationRef.current === rampIntegration) return;
    lastRampIntegrationRef.current = rampIntegration;
    const pixel = rampPixelRef.current;
    if (!ramp || !pixel || pixel.path !== file?.path) return;
    void loadSpectrum(pixel.path, pixel.x, pixel.y, rampIntegration);
  }, [rampIntegration, ramp, file?.path, loadSpectrum]);

  const handleCubePixelClick = useCallback((x: number, y: number) => {
    const dims = file?.result?.dimensions;
    if (!isCube || !dims || x < 0 || x >= dims[0] || y < 0 || y >= dims[1]) return;
    extractSpectrum(x, y);
  }, [isCube, file?.result?.dimensions, extractSpectrum]);

  const pixelClick = usePixelClick();
  const spectrumClickSeqRef = useRef(0);
  useEffect(() => {
    if (!pixelClick || !isCube) return;
    if (pixelClick.seq === spectrumClickSeqRef.current) return;
    spectrumClickSeqRef.current = pixelClick.seq;
    extractSpectrum(pixelClick.x, pixelClick.y);
  }, [pixelClick, isCube, extractSpectrum]);

  const handleViewerMousePixel = useCallback((x: number, y: number) => { setMousePixel({ x, y }); }, []);

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

  const cpuDisplayTitle = cpuViewerDisplayTitle({ useGpu, previewOnly: displayedPreviewOnly, loadFailed: gpuLoadError !== null });
  const gpuToggle = gpuToggleView({
    probing: gpuProbing,
    loading: toggleLoading,
    available: gpuAvailable,
    supported: gpuSupported,
    useGpu,
    loadError: gpuLoadError,
    reason: gpuReason,
    pngOnlyMono: !isRgbView && displayedPreviewOnly,
    cpuViewerShown: viewer === "cpu",
  });
  const gpuToggleBusy = gpuToggle.state === "probing" || gpuToggle.state === "loading";

  const planeBadge = file ? planeLabel(parseImageRef(file.path), file.result?.plane?.extname) : null;

  return (
    <div className="flex flex-col h-full overflow-hidden">
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
          {canCompositeReset && (
            <button
              onClick={handleCompositeReset}
              disabled={compositeRunning}
              className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded text-zinc-400 hover:text-zinc-200 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
              style={{ border: "1px solid var(--ab-border)" }}
              title={compositeRunning ? "A step is running on the composite; revert once it finishes" : compositeLastLabel ? `Showing ${compositeLastLabel} on the composite. Revert the composite to before processing.` : "Clear the PSF kernel estimated on the composite"}
            >
              <SkipBack size={10} />
              Revert to original
            </button>
          )}
          {showParked && (
            <button
              onClick={handleShowComposite}
              className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded text-violet-300/90 hover:text-violet-200 transition-colors"
              style={{ border: "1px solid rgba(168,85,247,0.3)" }}
              title="Show the colour composite built in Compose"
            >
              <Palette size={10} />
              Show composite
            </button>
          )}
          {file && !compositeMode && canReset && (
            <button
              onClick={resetProcessed}
              className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded text-zinc-400 hover:text-zinc-200 transition-colors"
              style={{ border: "1px solid var(--ab-border)" }}
              title={processed ? `Showing ${processed.label}. Revert to the original image; step outputs stay on disk` : "Clear the processing chain of this file"}
            >
              <SkipBack size={10} />
              Revert to original
            </button>
          )}
          {file && (
            <button onClick={handleToggleGpu} disabled={gpuProbing || (gpuAvailable === false && !useGpu && !gpuSupported)}
                    title={gpuToggle.title}
                    className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded transition-all duration-200 disabled:opacity-30 disabled:cursor-not-allowed"
                    style={GPU_TOGGLE_STYLES[gpuToggle.tone]}>
              {gpuToggleBusy ? <Loader2 size={10} className="animate-spin" /> : useGpu ? <Zap size={10} /> : <Cpu size={10} />}
              {gpuToggle.label}
            </button>
          )}
        </div>
      </div>

      <div data-dock-viewport="" className="flex-1 overflow-hidden min-h-0">
        {viewer === "empty" ? (
          <AdvancedImageViewer original={null} processed={null} />
        ) : viewer === "cpu" ? (
          <div className="flex flex-col h-full">
            <DisplayControls vmin={Number.NaN} vmax={Number.NaN} renderOnlyDisabled renderOnlyTitle={cpuDisplayTitle} />
            <div className="flex-1 min-h-0">
              <AdvancedImageViewer
                original={originalImage}
                processed={processedImage}
                onMousePixel={handleViewerMousePixel}
                onPixelClick={emitPixelClick}
                onCanvasPixelClick={isCube ? handleCubePixelClick : undefined}
                onMouseLeave={handleLeave}
                overlayCanvasRef={starOverlayRef}
                dqCanvasRef={dqCanvasRef}
                canvasHint={isCube ? CUBE_SPECTRUM_HINT : undefined}
              />
            </div>
          </div>
        ) : (
          <div className="h-full" onMouseLeave={handleLeave}>
            <PreviewTab
              useGpu={useGpu}
              rawPixels={rawPixels}
              rgbRawPixels={rgbRawPixels}
              onCubePixelClick={handleCubePixelClick}
              onBackToFile={handleBackToFile}
              starOverlayRef={starOverlayRef}
              dqCanvasRef={dqCanvasRef}
            />
          </div>
        )}
      </div>

      {file && <ViewerStatusStrip />}
    </div>
  );
}
