import { useState, useCallback, useMemo, memo } from "react";
import { X, SlidersHorizontal, RotateCcw } from "lucide-react";
import {
  useFileContext,
  useHistContext,
  useCubeContext,
  useRenderContext,
  useDisplayContext,
  useDisplayedImage,
} from "../../context/PreviewContext";
import { useCompositePreview, useCompositeStf, useCompositeActions } from "../../context/CompositeContext";
import { useWizardCompositeDims } from "../../context/ComposeWizardContext";
import type { RawPixelData, RawRgbPixelData, StfParams } from "../../shared/types";
import { GRAY_LUT_RGBA, resolveTransferLimits, toDisplayTransfer } from "../../utils/displayTransfer";
import { CANVAS_HINT_CLASS, CUBE_SPECTRUM_HINT, histogramOnPath } from "../../utils/previewShell";
import { emitPixelClick, pixelFromRect, setMousePixel } from "../../hooks/useMousePixelStore";

import ZoomPanView from "../ui/ZoomPanView";
import GpuViewport, { type ViewportOriginal } from "../render/GpuViewport";
import GpuRenderer from "../render/GpuRenderer";
import GpuRgbRenderer from "../render/GpuRgbRenderer";
import DisplayControls from "./DisplayControls";
import { useRegionKey } from "../../hooks/useRegionKey";

interface PreviewTabProps {
  useGpu: boolean;
  rawPixels: RawPixelData | null;
  rgbRawPixels?: RawRgbPixelData | null;
  onCubePixelClick: (x: number, y: number) => void;
  onBackToFile: () => void;
  starOverlayRef: React.RefObject<HTMLCanvasElement | null>;
  dqCanvasRef?: React.RefObject<HTMLCanvasElement | null>;
}

const IDENTITY_STF: StfParams = { shadow: 0, midtone: 0.5, highlight: 1 };

function sameStf(a: StfParams, b: StfParams): boolean {
  return a.shadow === b.shadow && a.midtone === b.midtone && a.highlight === b.highlight;
}

function clearMousePixel() {
  setMousePixel(null);
}

const ProcessedBadge = memo(function ProcessedBadge({ label }: { label: string }) {
  return (
    <div
      className="absolute bottom-2 left-2 z-10 pointer-events-none text-[10px] px-2 py-0.5 rounded bg-black/60 text-emerald-300/90"
      title="Showing a processed result; use Revert to original in the preview header to return to the original"
    >
      {label}
    </div>
  );
});

function PreviewTabInner({ useGpu, rawPixels, rgbRawPixels, onCubePixelClick, onBackToFile, starOverlayRef, dqCanvasRef }: PreviewTabProps) {
  const { file } = useFileContext();
  const { stfParams, histData, histDataPath } = useHistContext();
  const { isCube } = useCubeContext();
  const { processed } = useRenderContext();
  const displayed = useDisplayedImage();
  const { compositePreviewUrl } = useCompositePreview();
  const { setCompositeStf, setCompositeStfLinked } = useCompositeActions();
  const {
    compositeStfR, compositeStfG, compositeStfB, compositeStfLinked,
    compositeAutoStfR, compositeAutoStfG, compositeAutoStfB,
  } = useCompositeStf();
  const [stfOpen, setStfOpen] = useState(false);
  const { display, limits, lut } = useDisplayContext();
  const regionKey = useRegionKey();

  const isFileRgbView =
    compositePreviewUrl !== null && !!file?.result?.is_rgb && compositePreviewUrl === (file?.result?.previewUrl ?? null);
  const showComposite = compositePreviewUrl !== null && !(isFileRgbView && processed !== null);

  const fileDims = file?.result?.dimensions ?? null;
  const wizardCompositeDims = useWizardCompositeDims();
  const compositeDims = isFileRgbView ? fileDims : wizardCompositeDims;
  const displayedDims = displayed.dimensions;
  const regionsOnDisplayed = !!displayedDims && regionKey !== null;
  const regionsOnRgbFile = isFileRgbView && !!fileDims && regionKey !== null;

  const histForDisplayed = histogramOnPath(histData, histDataPath, displayed.path);

  const rawMin = rawPixels?.min;
  const rawMax = rawPixels?.max;
  const transfer = useMemo(() => {
    const rawRange = rawMin !== undefined && rawMax !== undefined ? { min: rawMin, max: rawMax } : null;
    return toDisplayTransfer(display, stfParams, resolveTransferLimits(display.stretch, limits, rawRange, histForDisplayed));
  }, [display, stfParams, limits, rawMin, rawMax, histForDisplayed]);
  const lutBytes = lut ?? GRAY_LUT_RGBA;

  const updateStf = useCallback((ch: "r" | "g" | "b", param: keyof StfParams, val: number) => {
    if (compositeStfLinked) {
      const next = { ...compositeStfR, [param]: val };
      setCompositeStf(next, next, next);
    } else {
      setCompositeStf(
        ch === "r" ? { ...compositeStfR, [param]: val } : compositeStfR,
        ch === "g" ? { ...compositeStfG, [param]: val } : compositeStfG,
        ch === "b" ? { ...compositeStfB, [param]: val } : compositeStfB,
      );
    }
  }, [compositeStfLinked, compositeStfR, compositeStfG, compositeStfB, setCompositeStf]);

  const toggleStfLinked = useCallback(() => {
    const next = !compositeStfLinked;
    setCompositeStfLinked(next);
    if (next) setCompositeStf(compositeStfR, compositeStfR, compositeStfR);
  }, [compositeStfLinked, compositeStfR, setCompositeStf, setCompositeStfLinked]);

  const resetStfToAuto = useCallback(() => {
    if (compositeAutoStfR && compositeAutoStfG && compositeAutoStfB) {
      setCompositeStf(compositeAutoStfR, compositeAutoStfG, compositeAutoStfB);
      setCompositeStfLinked(sameStf(compositeAutoStfR, compositeAutoStfG) && sameStf(compositeAutoStfG, compositeAutoStfB));
    } else {
      const d = { shadow: 0, midtone: 0.5, highlight: 1 };
      setCompositeStf(d, d, d);
    }
  }, [compositeAutoStfR, compositeAutoStfG, compositeAutoStfB, setCompositeStf, setCompositeStfLinked]);

  const handleRgbFileMouseMove = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      const img = e.currentTarget.querySelector("img");
      if (!img) return;
      const coord = pixelFromRect(e.clientX, e.clientY, img.getBoundingClientRect(), fileDims);
      if (coord) setMousePixel(coord);
    },
    [fileDims],
  );

  const handleViewerMousePixel = useCallback((x: number, y: number) => {
    setMousePixel({ x, y });
  }, []);

  const originalPreviewUrl = file?.result?.previewUrl ?? null;
  const isProcessed = displayed.isProcessed;
  const heldOriginal = useMemo<ViewportOriginal | null>(() => {
    if (!isProcessed || !originalPreviewUrl) return null;
    const sameGrid = !!fileDims && !!displayedDims && fileDims[0] === displayedDims[0] && fileDims[1] === displayedDims[1];
    return {
      url: originalPreviewUrl,
      disabledReason: sameGrid
        ? null
        : `The original is ${fileDims ? `${fileDims[0]}×${fileDims[1]}` : "of unknown size"} and the result ${displayedDims ? `${displayedDims[0]}×${displayedDims[1]}` : "of unknown size"} px; hold-to-compare needs the same pixel grid`,
    };
  }, [isProcessed, originalPreviewUrl, fileDims, displayedDims]);

  if (showComposite) {
    const rgbOnGpu = useGpu && !!rgbRawPixels;
    const displayReferred = !!rgbRawPixels?.displayReferred;
    const stfRow = (label: string, color: string, stf: StfParams, ch: "r" | "g" | "b") => (
      <div key={label} className="flex items-center gap-2">
        <span className="text-[9px] font-mono w-7 shrink-0" style={{ color }}>{label}</span>
        {(["shadow", "midtone", "highlight"] as const).map((param) => (
          <div key={param} className="flex-1 flex items-center gap-1 min-w-0" title={param}>
            <span className="text-[8px] text-zinc-500 uppercase shrink-0">{param[0]}</span>
            <input
              type="range"
              min={param === "midtone" ? 0.001 : 0}
              max={param === "midtone" ? 0.999 : 1}
              step={0.001}
              value={stf[param]}
              onChange={(e) => updateStf(ch, param, parseFloat(e.target.value))}
              aria-label={`${label} ${param}`}
              className="w-full h-1 accent-violet-400 cursor-pointer"
            />
            <span className="text-[8px] font-mono text-zinc-500 w-9 shrink-0 text-right">{stf[param].toFixed(3)}</span>
          </div>
        ))}
      </div>
    );
    return (
      <div className="flex flex-col h-full">
        <div className="flex items-center gap-2 px-3 py-1.5 bg-violet-900/30 border-b border-violet-600/20">
          <span className="text-[10px] text-violet-300">{isFileRgbView ? "RGB" : "RGB Composite"}{rgbOnGpu ? " · GPU" : ""}</span>
          {rgbOnGpu && displayReferred && (
            <span
              className="text-[9px] px-1.5 py-0.5 rounded text-emerald-300/90 bg-emerald-600/15"
              title="Showing the processed composite (stretch/curves applied). STF is baked in."
            >
              processed
            </span>
          )}
          {rgbOnGpu && !displayReferred && (
            <button
              onClick={() => setStfOpen((v) => !v)}
              title={stfOpen ? "Hide live STF controls" : "Adjust per-channel STF live"}
              className={`flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded transition-colors ${stfOpen ? "text-violet-200 bg-violet-600/25" : "text-violet-400/80 hover:text-violet-200"}`}
            >
              <SlidersHorizontal size={10} />
              STF
            </button>
          )}
          {!isFileRgbView && (
            <button
              onClick={onBackToFile}
              className="ml-auto flex items-center gap-1 text-[10px] text-zinc-400 hover:text-zinc-200 transition-colors"
            >
              Back to file
              <X size={10} />
            </button>
          )}
        </div>
        {rgbOnGpu && stfOpen && !displayReferred && (
          <div className="flex flex-col gap-1 px-3 py-2 border-b border-violet-600/15" style={{ background: "rgba(46,16,101,0.25)" }}>
            <div className="flex items-center justify-between">
              <button
                onClick={toggleStfLinked}
                className={`text-[9px] px-1.5 py-0.5 rounded transition-colors ${compositeStfLinked ? "text-violet-200 bg-violet-600/25" : "text-zinc-400 bg-zinc-800/60 hover:text-zinc-200"}`}
                title={compositeStfLinked ? "Channels linked — click to adjust R/G/B independently" : "Independent channels — click to link"}
              >
                {compositeStfLinked ? "Linked RGB" : "Per-channel"}
              </button>
              <button
                onClick={resetStfToAuto}
                className="flex items-center gap-1 text-[9px] text-zinc-400 hover:text-zinc-200 transition-colors"
                title="Reset to auto STF"
              >
                <RotateCcw size={9} />
                Auto
              </button>
            </div>
            {compositeStfLinked
              ? stfRow("RGB", "#c4b5fd", compositeStfR, "r")
              : (
                <>
                  {stfRow("R", "#f87171", compositeStfR, "r")}
                  {stfRow("G", "#4ade80", compositeStfG, "g")}
                  {stfRow("B", "#60a5fa", compositeStfB, "b")}
                </>
              )}
          </div>
        )}
        {useGpu && rgbRawPixels ? (
          <div className="relative flex-1 min-h-0">
            <GpuViewport
              renderW={rgbRawPixels.width}
              renderH={rgbRawPixels.height}
              fitsW={compositeDims?.[0]}
              fitsH={compositeDims?.[1]}
              deepZoomOffered={isFileRgbView}
              regionsEnabled={regionsOnRgbFile}
              crosshairEnabled={isFileRgbView}
              onMousePixel={isFileRgbView ? handleViewerMousePixel : undefined}
              onPixelClick={isFileRgbView ? emitPixelClick : undefined}
              onMouseLeave={clearMousePixel}
              overlayCanvasRef={starOverlayRef}
            >
              <GpuRgbRenderer
                rgb={rgbRawPixels}
                stfR={displayReferred ? IDENTITY_STF : compositeStfR}
                stfG={displayReferred ? IDENTITY_STF : compositeStfG}
                stfB={displayReferred ? IDENTITY_STF : compositeStfB}
                linked={displayReferred ? true : compositeStfLinked}
              />
            </GpuViewport>
          </div>
        ) : (
          <div
            className="flex-1 min-h-0 flex flex-col"
            onMouseMove={isFileRgbView ? handleRgbFileMouseMove : undefined}
            onMouseLeave={clearMousePixel}
          >
            <ZoomPanView
              src={compositePreviewUrl}
              alt="RGB composite"
              className="flex-1 min-h-0"
              overlayCanvasRef={starOverlayRef}
            />
          </div>
        )}
      </div>
    );
  }

  if (!useGpu || !rawPixels || displayed.previewOnly) return null;

  const badge = displayed.isProcessed && displayed.label ? <ProcessedBadge label={displayed.label} /> : null;

  return (
    <div className="flex flex-col h-full">
      <DisplayControls vmin={transfer.vmin} vmax={transfer.vmax} />
      <div className="relative flex-1 min-h-0">
        <GpuViewport
          renderW={rawPixels.width}
          renderH={rawPixels.height}
          fitsW={displayedDims?.[0]}
          fitsH={displayedDims?.[1]}
          crosshairEnabled
          onMousePixel={handleViewerMousePixel}
          onPixelClick={emitPixelClick}
          onMouseLeave={clearMousePixel}
          overlayCanvasRef={starOverlayRef}
          dqCanvasRef={dqCanvasRef}
          onCanvasPixelClick={isCube ? onCubePixelClick : undefined}
          regionsEnabled={regionsOnDisplayed}
          original={heldOriginal}
        >
          <GpuRenderer
            rawData={rawPixels.data}
            width={rawPixels.width}
            height={rawPixels.height}
            transfer={transfer}
            lut={lutBytes}
          />
        </GpuViewport>
        {badge}
        {isCube && <div className={CANVAS_HINT_CLASS}>{CUBE_SPECTRUM_HINT}</div>}
      </div>
    </div>
  );
}

export default memo(PreviewTabInner);
