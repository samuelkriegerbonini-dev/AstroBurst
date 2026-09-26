import { useState, useCallback, useRef, useEffect, useMemo, memo, lazy, Suspense } from "react";
import { Image, Loader2, X, SlidersHorizontal, RotateCcw } from "lucide-react";
import {
  fileKeyOf,
  useFileContext,
  useHistContext,
  useCubeContext,
  useRenderContext,
  useDisplayContext,
  useDisplayedImage,
} from "../../context/PreviewContext";
import { useCompositePreview, useCompositeStf, useCompositeActions } from "../../context/CompositeContext";
import { useWizardCompositeDims } from "../../context/ComposeWizardContext";
import type { HistogramData, RawPixelData, RawRgbPixelData, StfParams } from "../../shared/types";
import { GRAY_LUT_RGBA, resolveTransferLimits, toDisplayTransfer } from "../../utils/displayTransfer";
import { emitPixelClick, pixelFromRect, setMousePixel } from "../../hooks/useMousePixelStore";

import ZoomPanView from "../ui/ZoomPanView";
import GpuViewport, { type ViewportOriginal } from "../render/GpuViewport";
import DisplayControls from "./DisplayControls";
import { useRegionKey } from "../../hooks/useRegionKey";

const GpuRenderer = lazy(() => import("../render/GpuRenderer"));
const GpuRgbRenderer = lazy(() => import("../render/GpuRgbRenderer"));

interface PreviewTabProps {
  useGpu: boolean;
  rawPixels: RawPixelData | null;
  rgbRawPixels?: RawRgbPixelData | null;
  onImageClick: (e: React.MouseEvent<HTMLElement>) => void;
  onCubePixelClick: (x: number, y: number) => void;
  onBackToFile: () => void;
  starOverlayRef: React.RefObject<HTMLCanvasElement | null>;
  dqCanvasRef?: React.RefObject<HTMLCanvasElement | null>;
}

const MAX_RETRIES = 2;
const RETRY_DELAYS = [300, 800] as const;
const IDENTITY_STF: StfParams = { shadow: 0, midtone: 0.5, highlight: 1 };
const PNG_ONLY_TITLE = "PNG-only result; use Revert to original in the preview header to get the display controls back";
const NEEDS_GPU_TITLE = "needs GPU rendering";

function sameStf(a: StfParams, b: StfParams): boolean {
  return a.shadow === b.shadow && a.midtone === b.midtone && a.highlight === b.highlight;
}

function clearMousePixel() {
  setMousePixel(null);
}

const Overlay = memo(function Overlay({
                                        starOverlayRef,
                                        dqCanvasRef,
                                        isCube,
                                      }: {
  starOverlayRef: React.RefObject<HTMLCanvasElement | null>;
  dqCanvasRef?: React.RefObject<HTMLCanvasElement | null>;
  isCube: boolean;
}) {
  return (
    <>
      <canvas
        ref={starOverlayRef}
        className="absolute inset-0 w-full h-full pointer-events-none"
        style={{ display: "none" }}
      />
      {dqCanvasRef && (
        <canvas
          ref={dqCanvasRef}
          className="absolute inset-0 w-full h-full pointer-events-none"
          style={{ display: "none" }}
        />
      )}
      {isCube && (
        <div className="absolute bottom-2 right-2 bg-black/60 backdrop-blur-sm text-[10px] text-purple-300 px-2 py-1 rounded">
          Click to extract spectrum
        </div>
      )}
    </>
  );
});

const ProcessedBadge = memo(function ProcessedBadge({ label, pngOnly }: { label: string; pngOnly: boolean }) {
  return (
    <div
      className="absolute bottom-2 left-2 z-10 pointer-events-none text-[10px] px-2 py-0.5 rounded bg-black/60 text-emerald-300/90"
      title={pngOnly ? PNG_ONLY_TITLE : "Showing a processed result; use Revert to original in the preview header to return to the original"}
    >
      {pngOnly ? `${label} · PNG` : label}
    </div>
  );
});

interface ImageBox {
  left: number;
  top: number;
  width: number;
  height: number;
}

const ImagePreview = memo(function ImagePreview({
  src,
  alt,
  isCube,
  dims,
  onClick,
  onError,
  starOverlayRef,
  dqCanvasRef,
}: {
  src: string;
  alt: string | undefined;
  isCube: boolean;
  dims: [number, number] | null;
  onClick: (e: React.MouseEvent<HTMLElement>) => void;
  onError: () => void;
  starOverlayRef: React.RefObject<HTMLCanvasElement | null>;
  dqCanvasRef?: React.RefObject<HTMLCanvasElement | null>;
}) {
  const imgRef = useRef<HTMLImageElement>(null);
  const frameRef = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState<ImageBox>({ left: 0, top: 0, width: 0, height: 0 });

  const measure = useCallback(() => {
    const img = imgRef.current;
    if (!img) return;
    const next: ImageBox = { left: img.offsetLeft, top: img.offsetTop, width: img.offsetWidth, height: img.offsetHeight };
    setBox((prev) =>
      prev.left === next.left && prev.top === next.top && prev.width === next.width && prev.height === next.height
        ? prev
        : next,
    );
  }, []);

  useEffect(() => {
    const img = imgRef.current;
    if (!img) return;
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(img);
    const frame = frameRef.current;
    if (frame) ro.observe(frame);
    return () => ro.disconnect();
  }, [measure, src]);

  const handleMouseMove = useCallback(
    (e: React.MouseEvent<HTMLImageElement>) => {
      const coord = pixelFromRect(e.clientX, e.clientY, e.currentTarget.getBoundingClientRect(), dims);
      if (coord) setMousePixel(coord);
    },
    [dims],
  );

  return (
    <div ref={frameRef} className="relative flex-1 min-h-0 flex items-center justify-center">
      <img
        ref={imgRef}
        src={src}
        alt={alt}
        className={`max-w-full max-h-full object-contain ${isCube ? "cursor-crosshair" : ""}`}
        onClick={onClick}
        onError={onError}
        onLoad={measure}
        onMouseMove={handleMouseMove}
        onMouseLeave={clearMousePixel}
        loading="eager"
        decoding="async"
      />
      <div
        className="absolute pointer-events-none"
        style={{ left: box.left, top: box.top, width: box.width, height: box.height }}
      >
        <Overlay starOverlayRef={starOverlayRef} dqCanvasRef={dqCanvasRef} isCube={isCube} />
      </div>
    </div>
  );
});

function PreviewTabInner({ useGpu, rawPixels, rgbRawPixels, onImageClick, onCubePixelClick, onBackToFile, starOverlayRef, dqCanvasRef }: PreviewTabProps) {
  const { file } = useFileContext();
  const { stfParams, histData } = useHistContext();
  const { isCube } = useCubeContext();
  const { processed, stfPreviewUrl } = useRenderContext();
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

  const fileKey = fileKeyOf(file);
  const isFileRgbView =
    !!compositePreviewUrl && !!file?.result?.is_rgb && compositePreviewUrl === (file?.result?.previewUrl ?? null);
  const showComposite = !!compositePreviewUrl && !(isFileRgbView && processed !== null);

  const fileDims = file?.result?.dimensions ?? null;
  const wizardCompositeDims = useWizardCompositeDims();
  const compositeDims = isFileRgbView ? fileDims : wizardCompositeDims;
  const displayedDims = displayed.dimensions;
  const regionsOnDisplayed = !!displayedDims && regionKey !== null;
  const regionsOnRgbFile = isFileRgbView && !!fileDims && regionKey !== null;

  const histKey = `${fileKey ?? ""}|${displayed.path ?? ""}`;
  const [histOwner, setHistOwner] = useState<{ hist: HistogramData | null; key: string }>({ hist: histData, key: histKey });
  if (histOwner.hist !== histData) setHistOwner({ hist: histData, key: histKey });
  const histForDisplayed = histOwner.hist === histData && histOwner.key === histKey ? histData : null;

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

  const [previewError, setPreviewError] = useState(false);
  const [retryKey, setRetryKey] = useState(0);
  const retryRef = useRef<{ timer: ReturnType<typeof setTimeout> | null; count: number }>({
    timer: null,
    count: 0,
  });

  const baseUrl = stfPreviewUrl ?? displayed.previewUrl;

  useEffect(() => {
    setPreviewError(false);
    setRetryKey(0);
    retryRef.current.count = 0;
  }, [fileKey, baseUrl]);

  useEffect(() => {
    const r = retryRef.current;
    return () => {
      if (r.timer) clearTimeout(r.timer);
    };
  }, []);

  const handlePreviewError = useCallback(() => {
    const r = retryRef.current;
    if (r.timer) return;
    if (r.count < MAX_RETRIES) {
      const delay = RETRY_DELAYS[r.count];
      r.timer = setTimeout(() => {
        r.timer = null;
        r.count += 1;
        setRetryKey((k) => k + 1);
      }, delay);
    } else {
      setPreviewError(true);
    }
  }, []);

  const previewUrl = useMemo(() => {
    if (!baseUrl) return null;
    if (retryKey === 0) return baseUrl;
    return `${baseUrl}${baseUrl.includes("?") ? "&" : "?"}t=${retryKey}`;
  }, [baseUrl, retryKey]);

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
            <Suspense fallback={<Loader2 size={20} className="animate-spin text-zinc-600" />}>
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
              >
                <GpuRgbRenderer
                  rgb={rgbRawPixels}
                  stfR={displayReferred ? IDENTITY_STF : compositeStfR}
                  stfG={displayReferred ? IDENTITY_STF : compositeStfG}
                  stfB={displayReferred ? IDENTITY_STF : compositeStfB}
                  linked={displayReferred ? true : compositeStfLinked}
                />
              </GpuViewport>
            </Suspense>
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
            />
          </div>
        )}
      </div>
    );
  }

  const previewOnly = displayed.previewOnly;
  const badge = displayed.isProcessed && displayed.label
    ? <ProcessedBadge label={displayed.label} pngOnly={previewOnly} />
    : null;

  if (useGpu && rawPixels && !previewOnly) {
    return (
      <div className="flex flex-col h-full">
        <DisplayControls vmin={transfer.vmin} vmax={transfer.vmax} />
        <div className="relative flex-1 min-h-0">
          <Suspense fallback={<Loader2 size={20} className="animate-spin text-zinc-600" />}>
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
          </Suspense>
          {badge}
          {isCube && (
            <div className="absolute bottom-2 right-2 bg-black/60 text-[10px] text-purple-300 px-2 py-1 rounded pointer-events-none">
              Click to extract spectrum
            </div>
          )}
        </div>
      </div>
    );
  }

  const controls = (
    <div title={previewOnly ? PNG_ONLY_TITLE : NEEDS_GPU_TITLE}>
      <DisplayControls vmin={transfer.vmin} vmax={transfer.vmax} disabled />
    </div>
  );

  if (previewUrl && !previewError) {
    return (
      <div className="flex flex-col h-full">
        {controls}
        <div className="relative flex-1 min-h-0 flex flex-col">
          <ImagePreview
            src={previewUrl}
            alt={file?.name}
            isCube={isCube}
            dims={displayedDims}
            onClick={onImageClick}
            onError={handlePreviewError}
            starOverlayRef={starOverlayRef}
            dqCanvasRef={dqCanvasRef}
          />
          {badge}
        </div>
      </div>
    );
  }

  if (previewError) {
    return (
      <div className="flex flex-col h-full">
        {controls}
        <div className="flex-1 flex flex-col items-center justify-center gap-2 text-zinc-600">
          <Image size={32} strokeWidth={1} />
          <p className="text-xs">Preview unavailable</p>
          <button
            onClick={() => {
              retryRef.current.count = 0;
              setPreviewError(false);
              setRetryKey((k) => k + 1);
            }}
            className="text-[10px] hover:text-zinc-300 mt-1"
            style={{ color: "var(--ab-teal)" }}
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  return null;
}

export default memo(PreviewTabInner);
